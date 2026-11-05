//! `lc run-logged`: the small runner between a terminal and a worker CLI.
//! It runs the CLI on pipes (a pseudo console would wrap its JSON lines),
//! saves every byte it prints, draws one readable line per step, reports its
//! status as OSC 7501, and ends with the CLI's own exit code.

use crate::paths::write_atomic;
use crate::task::opencode::{RouteCheck, route_check};
use crate::task::types::{Beat, cut};
use crate::task::worker::{Progress, TurnOutcome, Worker};
use clap::Args as ClapArgs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(ClapArgs)]
pub struct Args {
    /// The task's folder: its turn files live here.
    #[arg(long)]
    pub dir: PathBuf,
    #[arg(long)]
    pub task: String,
    #[arg(long)]
    pub turn: u32,
    #[arg(long)]
    pub worker: String,
    /// The file the worker reads as stdin (closed when absent).
    #[arg(long)]
    pub stdin: Option<PathBuf>,
    /// The route the turn was asked for, for the OpenCode export check.
    #[arg(long)]
    pub route: Option<String>,
    /// How many of the program's first items are the OpenCode CLI itself.
    #[arg(long)]
    pub prefix_len: Option<usize>,
    /// The program and its arguments, after `--`.
    #[arg(last = true, required = true, allow_hyphen_values = true)]
    pub program: Vec<String>,
}

#[derive(Default)]
struct Shared {
    out_bytes: u64,
    active_ms: u64,
    cpu_ms: u64,
    steps: u64,
    tools: Vec<String>,
    cost_usd: f64,
    peak: u64,
    last_osc: Option<Instant>,
    worker_ended: bool,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// When the turn began: a line on the screen says how far in it is. std gives
/// no local time without a crate, and a UTC clock misleads anyone not on UTC.
static BEGAN: OnceLock<Instant> = OnceLock::new();

fn elapsed() -> String {
    let seconds = BEGAN.get_or_init(Instant::now).elapsed().as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// To the terminal. A closed one must not stop the turn, as `print!` would by
/// panicking: the worker's own output goes to files either way.
fn say(text: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn screen(line: &str) {
    say(&format!("{}  {}\n", elapsed(), cut(line, 180)));
}

/// At most `max` bytes, on a character boundary (the status field carries base64).
fn bytes_cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Says what the runner is doing, for the core and any viewer; working news at
/// most every 2 s, the end always.
fn status(shared: &Mutex<Shared>, state: &str, message: &str) {
    let now = Instant::now();
    {
        let mut st = shared.lock().unwrap_or_else(|p| p.into_inner());
        if state == "working" && st.last_osc.is_some_and(|at| now.duration_since(at) < Duration::from_secs(2)) {
            return;
        }
        st.last_osc = Some(now);
    }
    let msg = crate::put::encode(bytes_cut(message, 200).as_bytes());
    say(&format!("\x1b]7501;state={state}:app=limen:msg={msg}\x1b\\"));
}

fn beat(shared: &Mutex<Shared>, path: &Path) {
    let beat = {
        let st = shared.lock().unwrap_or_else(|p| p.into_inner());
        Beat {
            at_ms: now_ms(),
            active_ms: st.active_ms,
            cpu_ms: st.cpu_ms,
            out_bytes: st.out_bytes,
            steps: st.steps,
            tools: st.tools.clone(),
            cost_usd: st.cost_usd,
            peak: st.peak,
        }
    };
    if let Ok(bytes) = serde_json::to_vec(&beat) {
        let _ = write_atomic(path, &bytes);
    }
}

/// One beat a second while the turn runs, and a last one as it ends. It is
/// activity, not output, that counts: a compiler in silence holds the idle
/// limit off (UPGRADES #28).
fn beat_loop(shared: Arc<Mutex<Shared>>, path: PathBuf, done: Arc<AtomicBool>) {
    let (mut last_out, mut last_cpu) = (0u64, 0u64);
    loop {
        let last = done.load(Ordering::SeqCst);
        let now = now_ms();
        let cpu = crate::platform::job_cpu_ms().unwrap_or(0);
        {
            let mut st = shared.lock().unwrap_or_else(|p| p.into_inner());
            if st.worker_ended || st.out_bytes > last_out || cpu.saturating_sub(last_cpu) >= 40 {
                st.active_ms = now;
            }
            st.cpu_ms = cpu;
            last_out = st.out_bytes;
        }
        last_cpu = cpu;
        beat(&shared, &path);
        if last {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// One export, at most 60 s: its stdout, or the words of its failure.
fn export_once(prefix: &[String], session: &str, cwd: &Path) -> Result<String, String> {
    let mut cmd = Command::new(&prefix[0]);
    cmd.args(&prefix[1..])
        .args(["session", "export", "--sanitize", session])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // No `no_console`: like the worker, it shares this terminal's hidden console.
    let mut child = cmd.spawn().map_err(|e| format!("cannot run the export: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("export timed out".into());
            }
        }
    };
    let out = out_thread.join().unwrap_or_default();
    let err = err_thread.join().unwrap_or_default();
    if !status.success() {
        let text = String::from_utf8_lossy(&err);
        let text = text.trim();
        return Err(if text.is_empty() {
            format!("export exited with code {}", status.code().unwrap_or(-1))
        } else {
            cut(text, 400)
        });
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// `verify_model`'s tries: a structurally incomplete export is fetched again,
/// a complete one that names another route fails at once.
fn verify_route(prefix: &[String], session: &str, route: &str, cwd: &Path, dir: &Path, turn: u32) -> RouteCheck {
    let mut check = RouteCheck::default();
    for attempt in 1..=4u32 {
        match export_once(prefix, session, cwd) {
            Err(message) => {
                check = RouteCheck {
                    status: "error".into(),
                    expected: route.into(),
                    message: Some(format!(
                        "{message} after {attempt} export {}",
                        if attempt == 1 { "attempt" } else { "attempts" }
                    )),
                    sample: Vec::new(),
                    attempts: attempt,
                };
                break;
            }
            Ok(text) => {
                let mut candidate = route_check(&text, route);
                candidate.attempts = attempt;
                let retry = candidate.retryable() && attempt < 4;
                check = candidate;
                if retry {
                    std::thread::sleep(Duration::from_millis(500 * attempt as u64));
                } else {
                    break;
                }
            }
        }
    }
    if check.retryable()
        && let Some(message) = check.message.clone()
    {
        check.message = Some(format!(
            "{message} after {} export {}",
            check.attempts,
            if check.attempts == 1 { "attempt" } else { "attempts" }
        ));
    }
    let path = dir.join(format!("turn-{turn}.route.json"));
    if let Ok(bytes) = serde_json::to_vec_pretty(&check) {
        let _ = write_atomic(&path, &bytes);
    }
    check
}

pub fn run(args: Args) -> ! {
    BEGAN.get_or_init(Instant::now);
    let worker = Worker::from_key(&args.worker).unwrap_or(Worker::Go);
    let label = Worker::from_key(&args.worker).map_or_else(|_| args.worker.clone(), |w| w.label().to_string());
    say(&format!("task {}, turn {}, {}\n", args.task, args.turn, label));
    let jsonl = args.dir.join(format!("turn-{}.jsonl", args.turn));
    let err = args.dir.join(format!("turn-{}.err", args.turn));
    let beat_path = args.dir.join(format!("turn-{}.beat", args.turn));
    let exit_path = args.dir.join(format!("turn-{}.exit", args.turn));
    let shared = Arc::new(Mutex::new(Shared { active_ms: now_ms(), ..Default::default() }));
    // Before the worker: once it runs, nothing here may stop it from being heard.
    let files = std::fs::create_dir_all(&args.dir)
        .and_then(|()| Ok((std::fs::File::create(&jsonl)?, std::fs::File::create(&err)?)));
    let (out_file, mut err_file) = match files {
        Ok(files) => files,
        Err(e) => {
            screen(&format!("cannot write this turn's output in {}: {e}", args.dir.display()));
            status(&shared, "error", "cannot write this turn's output");
            std::process::exit(127);
        }
    };
    // Opened before the worker starts, as the output files are: a prompt that
    // can't be read is said now, never half a turn later.
    let prompt_file = match &args.stdin {
        None => None,
        Some(path) => match std::fs::File::open(path) {
            Ok(file) => Some(file),
            Err(e) => {
                screen(&format!("cannot open this turn's prompt {}: {e}", path.display()));
                status(&shared, "error", "cannot open this turn's prompt");
                std::process::exit(127);
            }
        },
    };
    let done = Arc::new(AtomicBool::new(false));
    let beat_thread = {
        let (shared, done, beat_path) = (shared.clone(), done.clone(), beat_path);
        std::thread::spawn(move || beat_loop(shared, beat_path, done))
    };
    // No `no_console`: the child shares this terminal's hidden console, which the
    // terminal's own job already covers. It must not get a second one.
    let mut cmd = Command::new(&args.program[0]);
    cmd.args(&args.program[1..]).stdout(Stdio::piped()).stderr(Stdio::piped());
    match prompt_file {
        Some(file) => cmd.stdin(Stdio::from(file)),
        None => cmd.stdin(Stdio::null()),
    };
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let _ = writeln!(err_file, "cannot start {}: {e}", args.program[0]);
            screen(&format!("cannot start {}: {e}", args.program[0]));
            status(&shared, "error", &format!("cannot start: {e}"));
            std::process::exit(127);
        }
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out_shared = shared.clone();
    let out_thread = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut file = out_file;
        let mut progress = Progress::new(worker, std::env::current_dir().ok());
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // Exactly as it came, invalid UTF-8 included.
                    let _ = file.write_all(&buf);
                    let _ = file.flush();
                    let shown = {
                        let mut st = out_shared.lock().unwrap_or_else(|p| p.into_inner());
                        st.out_bytes += buf.len() as u64;
                        String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string()
                    };
                    if let Some(line) = progress.feed(&shown) {
                        screen(&line);
                        {
                            let mut st = out_shared.lock().unwrap_or_else(|p| p.into_inner());
                            st.steps = progress.steps;
                            st.tools = progress.tools.clone();
                            st.cost_usd = progress.cost_usd;
                            st.peak = progress.peak;
                        }
                        // The status message is the last tool, not a step's numbers.
                        let message = progress.tools.last().cloned().unwrap_or_else(|| line.clone());
                        status(&out_shared, "working", &message);
                    }
                }
            }
        }
        let mut st = out_shared.lock().unwrap_or_else(|p| p.into_inner());
        st.steps = progress.steps;
        st.tools = progress.tools.clone();
        st.cost_usd = progress.cost_usd;
        st.peak = progress.peak;
    });
    let err_thread = std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut file = err_file;
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let _ = file.write_all(&buf);
                    let _ = file.flush();
                }
            }
        }
    });
    let exit_status = match child.wait() {
        Ok(status) => status,
        Err(e) => {
            screen(&format!("cannot wait for the worker: {e}"));
            std::process::exit(127);
        }
    };
    let _ = out_thread.join();
    let _ = err_thread.join();
    // Both pipes are closed, so nothing the worker left behind is still writing.
    // What follows is this runner's own work under its own limits, and the idle
    // limit is the worker's: the beats go on, and they count as activity.
    shared.lock().unwrap_or_else(|p| p.into_inner()).worker_ended = true;
    let signal = crate::platform::exit_signal(&exit_status);
    let code = exit_status.code().unwrap_or_else(|| signal.map_or(1, |s| 128 + s));
    // OpenCode may be routed somewhere other than asked; its own export is the
    // evidence. It runs here, under the terminal's limits, never in the core.
    if let Some(route) = &args.route
        && code == 0
        && let Some(prefix_len) = args.prefix_len
        && let Ok(bytes) = std::fs::read(&jsonl)
    {
        let outcome = TurnOutcome::from_bytes(worker, &bytes);
        if outcome.sessions.len() == 1
            && let Some(session) = outcome.session.as_deref()
        {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let check = verify_route(&args.program[..prefix_len], session, route, &cwd, &args.dir, args.turn);
            let shown = match &check.message {
                Some(message) => format!("model route {}: {message}", check.status),
                None => format!("model route {}", check.status),
            };
            screen(&shown);
        }
    }
    done.store(true, Ordering::SeqCst);
    let _ = beat_thread.join();
    if let Ok(bytes) = serde_json::to_vec(&serde_json::json!({"code": code})) {
        let _ = write_atomic(&exit_path, &bytes);
    }
    let ending = match signal {
        Some(signal) => format!("ended: signal {signal} (exit code {code})"),
        None => format!("ended: exit code {code}"),
    };
    screen(&ending);
    status(&shared, if code == 0 { "done" } else { "error" }, &ending);
    std::process::exit(code);
}
