//! `kompitum-app do <command> [--arg value ... | json]`, `kompitum-app ls`, `kompitum-app commands`
//! and `kompitum-app status`: drive a running window from a shell (agents, tests)
//! over `$KOMPITUM_APP_HOME/kompitum-app.sock`, one JSON line each way.

use crate::mods::{self, EXEC_LIMIT, Request};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

fn socket() -> PathBuf {
    mods::app_dir().join("kompitum-app.sock")
}

/// Serves the socket for this window. A live socket means another window owns this home.
pub fn listen(tx: async_channel::Sender<Request>) {
    let path = socket();
    if UnixStream::connect(&path).is_ok() {
        tracing::warn!(
            "another Kompitum is serving {}; `kompitum-app do` goes there",
            path.display()
        );
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return tracing::warn!("cannot serve {}: {e}", path.display()),
    };
    let _ = std::thread::Builder::new()
        .name("kompitum-app-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve(stream, tx));
            }
        });
}

fn serve(stream: UnixStream, tx: async_channel::Sender<Request>) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Value>(&line) {
        Ok(mut cmd) => {
            let shot = cmd.get("shot").is_some();
            let lead_setup = cmd.get("do").and_then(Value::as_str) == Some("lead.setup");
            let hook = cmd.get("do").and_then(Value::as_str) == Some("lead.hook");
            let blocking = hook && blocking_event(&cmd);
            let hook_id = blocking.then(hook_id);
            if let Some(id) = &hook_id
                && let Some(args) = cmd.get_mut("args").and_then(Value::as_object_mut)
            {
                args.insert("hook_id".into(), json!(id));
            }
            // A blocking hook's owner may go away first: its socket tells us.
            if let Some(id) = hook_id
                && let Ok(mut watch) = stream.try_clone()
            {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let mut byte = [0u8; 1];
                    loop {
                        match watch.read(&mut byte) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                    let _ = tx.send_blocking(Request::HookClosed { id });
                });
            }
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            if tx
                .send_blocking(Request::Control {
                    cmd,
                    reply: reply_tx,
                })
                .is_err()
            {
                json!({ "error": "the window did not answer" })
            } else if blocking {
                // Claude's own hook timeout ends a waiting ask, not this thread.
                reply_rx
                    .recv()
                    .unwrap_or_else(|_| json!({ "error": "the window did not answer" }))
            } else {
                let timeout = Duration::from_secs(if shot {
                    30
                } else if lead_setup {
                    2 * EXEC_LIMIT.as_secs() + 5
                } else {
                    5
                });
                reply_rx
                    .recv_timeout(timeout)
                    .unwrap_or_else(|_| json!({ "error": "the window did not answer" }))
            }
        }
        Err(e) => json!({ "error": format!("bad request: {e}") }),
    };
    let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
}

fn blocking_event(cmd: &Value) -> bool {
    matches!(
        cmd.pointer("/args/event").and_then(Value::as_str),
        Some("PermissionRequest" | "PreToolUse")
    )
}

fn hook_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "hook-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// One Kompitum per home: a second launch brings the running window to the
/// front and quits. True when one is running (even one too busy to answer:
/// a second window would ring every alert twice).
pub fn raise_running() -> bool {
    raise_running_at(&socket())
}

fn raise_running_at(path: &std::path::Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    if stream
        .write_all(format!("{}\n", json!({ "raise": true })).as_bytes())
        .is_ok()
    {
        let _ = BufReader::new(&stream).read_line(&mut String::new());
    }
    true
}

/// The client side; returns the process exit code.
pub fn client(args: &[String]) -> i32 {
    if args == ["commands"] {
        println!(
            "{}",
            serde_json::to_string_pretty(&crate::commands::listing()).unwrap()
        );
        return 0;
    }
    if let [verb, event] = args
        && verb == "hook"
    {
        return hook(event);
    }
    let cmd = match args {
        [verb, rest @ ..] if verb == "shot" && rest.len() <= 1 => {
            let path = PathBuf::from(
                rest.first()
                    .map(String::as_str)
                    .unwrap_or("kompitum-shot.png"),
            );
            let path = match std::path::absolute(path) {
                Ok(path) => path,
                Err(e) => return fail(&format!("cannot resolve screenshot path: {e}")),
            };
            if path
                .extension()
                .is_none_or(|ext| !ext.eq_ignore_ascii_case("png"))
            {
                return fail("screenshot path must end in .png");
            }
            json!({ "shot": path })
        }
        [verb] if verb == "status" => json!({ "status": true }),
        [verb] if verb == "ls" => json!({ "ls": true }),
        [verb, message] if verb == "feedback" => {
            json!({ "do": "app.feedback", "args": { "message": message, "from": "agent" } })
        }
        [verb, command, rest @ ..] if verb == "do" => match parse_args(rest) {
            Ok(a) => json!({ "do": command, "args": a }),
            Err(e) => return fail(&e),
        },
        _ => {
            return fail(
                "usage: kompitum-app ls | kompitum-app commands | kompitum-app status | kompitum-app shot [file.png] | kompitum-app do <command> [--arg value ... | json]",
            );
        }
    };
    let mut stream = match UnixStream::connect(socket()) {
        Ok(s) => s,
        Err(_) if args.first().is_some_and(|v| v == "feedback") => {
            let url = crate::feedback::issue_url(args.get(1).map(String::as_str).unwrap_or(""), "agent");
            println!("{url}");
            eprintln!(
                "Kompitum isn't open, so no browser was opened. The link above is the filled-in issue: open it to read and submit. Nothing was sent."
            );
            return 1;
        }
        Err(e) => {
            return fail(&format!(
                "no Kompitum window at {}: {e}",
                socket().display()
            ));
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(
        if cmd.get("do").and_then(Value::as_str) == Some("lead.setup") {
            2 * EXEC_LIMIT.as_secs() + 10
        } else {
            35
        },
    )));
    let mut line = String::new();
    if stream.write_all(format!("{cmd}\n").as_bytes()).is_err()
        || BufReader::new(&stream).read_line(&mut line).is_err()
    {
        return fail("the window hung up");
    }
    let reply: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    if args.first().is_some_and(|v| v == "shot") {
        if let Some(error) = reply.get("error") {
            return fail(error.as_str().unwrap_or("screenshot failed"));
        }
        let Some(path) = reply.get("path").and_then(Value::as_str) else {
            return fail("the window returned no screenshot path");
        };
        println!("{path}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&reply).unwrap_or_default()
        );
    }
    if reply.get("error").is_some() { 1 } else { 0 }
}

/// `kompitum-app hook <event>`: a Claude Code hook's way to tell the app what
/// happened. Every failure is silent and exits 0: a hook must never block Claude.
fn hook(event: &str) -> i32 {
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        return 0;
    }
    let input: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let Ok(terminal) = std::env::var("KOMPITUM_TERMINAL_ID") else {
        return 0;
    };
    if terminal.trim().is_empty() {
        return 0;
    }
    let Ok(mut stream) = UnixStream::connect(socket()) else {
        return 0;
    };
    let cmd = json!({
        "do": "lead.hook",
        "args": { "event": event, "terminal": terminal, "input": input },
    });
    if stream.write_all(format!("{cmd}\n").as_bytes()).is_err() {
        return 0;
    }
    // No timeout of its own: Claude's hook timeout ends it.
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return 0;
    }
    if let Ok(reply) = serde_json::from_str::<Value>(&line)
        && let Some(output) = reply.get("output").filter(|value| !value.is_null())
    {
        println!("{}", serde_json::to_string(output).unwrap_or_default());
    }
    0
}

/// `--name api --argv '["pi"]'` (values that parse as JSON are JSON), or one JSON object.
fn parse_args(rest: &[String]) -> Result<Value, String> {
    match rest {
        [] => Ok(Value::Null),
        [raw] if raw.trim_start().starts_with('{') => {
            serde_json::from_str(raw).map_err(|e| format!("args are not JSON: {e}"))
        }
        _ => {
            let mut out = serde_json::Map::new();
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let key = flag
                    .strip_prefix("--")
                    .ok_or_else(|| format!("expected --<arg>, got {flag:?}"))?;
                let value = it.next().ok_or_else(|| format!("--{key} needs a value"))?;
                let v = serde_json::from_str::<Value>(value)
                    .ok()
                    .filter(|v| !v.is_string())
                    .unwrap_or_else(|| Value::String(value.clone()));
                out.insert(key.to_owned(), v);
            }
            Ok(Value::Object(out))
        }
    }
}

/// Encode before opening the destination: GPU errors must not truncate a good shot.
pub fn save_shot(window: &gpui::Window, path: &std::path::Path) -> anyhow::Result<()> {
    let image = window.render_to_image()?;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.write_header()?.write_image_data(image.as_raw())?;
    }
    // A sibling temporary file keeps publication on the same filesystem.
    let temporary = path.with_file_name(format!(".kompitum-shot-{}.png", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> anyhow::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn fail(message: &str) -> i32 {
    eprintln!("kompitum-app: {message}");
    2
}

#[cfg(test)]
mod hook_tests {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    /// The app's ordinary binary: Cargo sets CARGO_BIN_EXE only for integration
    /// tests, so a unit test looks beside its own test binary too.
    fn app_binary() -> Option<std::path::PathBuf> {
        if let Some(path) = option_env!("CARGO_BIN_EXE_kompitum-app") {
            let path = std::path::PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
        let test = std::env::current_exe().ok()?;
        let debug = test.parent()?.parent()?;
        ["kompitum-app.exe", "kompitum-app"]
            .into_iter()
            .map(|name| debug.join(name))
            .find(|path| path.is_file())
    }

    #[test]
    fn hook_with_no_window_exits_zero_and_prints_nothing() {
        let Some(bin) = app_binary() else {
            eprintln!("kompitum-app hook: no built binary beside the test executable; skipped");
            return;
        };
        let home = std::env::temp_dir().join(format!(
            "kompitum-hook-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let mut child = Command::new(bin)
            .args(["hook", "SessionStart"])
            .env("KOMPITUM_TERMINAL_ID", "t_test")
            .env("KOMPITUM_APP_HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start kompitum-app hook");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(br#"{"session_id":"s","hook_event_name":"SessionStart"}"#)
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let _ = std::fs::remove_dir_all(&home);
        assert!(
            out.status.success(),
            "hook exited {:?}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stdout.is_empty(),
            "hook printed: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_raises_the_running_window() {
        let dir = std::env::temp_dir().join(format!("kompitum-app-raise-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kompitum-app.sock");
        // Nothing running: this launch opens its own window.
        assert!(!raise_running_at(&path));
        // One running: it gets asked to come to the front.
        let listener = UnixListener::bind(&path).unwrap();
        let asked = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream).write_all(b"{\"ok\":true}\n").unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        });
        assert!(raise_running_at(&path));
        assert_eq!(asked.join().unwrap(), json!({ "raise": true }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
