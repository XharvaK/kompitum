//! Reading sessions, transcripts and git numbers for `kom task stats`.

use super::parse::{
    is_session_id, one_candidate, parse_claude, parse_commandcode, parse_commandcode_transcript, parse_export,
    parse_opencode,
};
use super::{
    AgentCall, Entry, NO_STEPS, Numbers, Parsed, finish, jobs_in, numbers, numstat_lines, patch_lines, recorded,
};
use crate::api::TaskStatsResult;
use crate::config::Config;
use crate::rpc::RpcError;
use crate::task::types::{Usage, cut};
use crate::task::worker::{self, Cli};
use std::collections::{BTreeSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_turn(cli: Cli, path: &Path, usage: &Usage) -> (Numbers, Option<String>, Vec<String>, Vec<String>) {
    let text = match read_text(path) {
        Ok(text) => text,
        Err(e) => {
            let words = format!("cannot read {}: {e}", path.display());
            return (recorded(usage), Some(format!("{words}; recorded totals only")), Vec::new(), vec![words]);
        }
    };
    let lines = text.lines().map(str::to_string);
    let parsed = match cli {
        Cli::Opencode => parse_opencode(lines),
        Cli::Claude => parse_claude(lines),
        Cli::CommandCode => parse_commandcode(lines),
        Cli::Codex => return (recorded(usage), Some(NO_STEPS.into()), Vec::new(), Vec::new()),
    };
    if parsed.steps.is_empty() {
        let note = match (parsed.note.clone(), usage.steps > 0) {
            (Some(note), true) => format!("{note}; recorded totals only"),
            (Some(note), false) => note,
            (None, true) => format!("no steps in {}; recorded totals only", path.display()),
            (None, false) => format!("no steps in {}", path.display()),
        };
        let numbers = if usage.steps > 0 { recorded(usage) } else { Numbers::default() };
        return (numbers, Some(note), parsed.children, Vec::new());
    }
    (numbers(&parsed.steps, &parsed.calls, &[]), parsed.note, parsed.children, Vec::new())
}

pub(crate) fn export_session(config: &Config, folder: &Path, session: &str) -> Result<String, String> {
    let prefix = worker::opencode_prefix(config).map_err(|e| e.message)?;
    let mut cmd = Command::new(&prefix[0]);
    cmd.args(&prefix[1..])
        .args(["session", "export", "--standalone", session])
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::task::runner::prepare_child(&mut cmd, None);
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

enum Child {
    Opencode(String),
    CommandCode(PathBuf),
}

fn cc_projects_root() -> PathBuf {
    match std::env::var_os("KOMPITUM_CC_PROJECTS") {
        Some(root) => {
            let path = PathBuf::from(root);
            match path.to_str().and_then(|text| text.strip_prefix("~/")) {
                Some(rest) => crate::platform::home_dir().join(rest),
                None => path,
            }
        }
        None => crate::platform::home_dir().join(".commandcode").join("projects"),
    }
}

fn transcript_file(name: &str) -> bool {
    name.ends_with(".jsonl") && !name.contains(".checkpoints.") && !name.contains(".prompts.")
}

fn commandcode_transcript(value: &str, folder: &Path) -> Result<PathBuf, RpcError> {
    let missing = |words: &str| {
        RpcError::new(
            "not_found",
            format!("no Command Code transcript for {value:?}: {words}"),
            "give a session id or the path to its .jsonl",
        )
    };
    if value.ends_with(".jsonl") {
        let path = PathBuf::from(value);
        let path = if path.is_absolute() { path } else { folder.join(path) };
        return path.is_file().then_some(path).ok_or_else(|| missing("the file is not there"));
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(cc_projects_root()).into_iter().flatten().flatten() {
        let dir = entry.path();
        let path = dir.join(format!("{value}.jsonl"));
        if dir.is_dir() && path.is_file() {
            found.push(path);
        }
    }
    let empty = found.is_empty();
    one_candidate(&mut found)
        .ok_or_else(|| missing(if empty { "no project has it" } else { "more than one project has it" }))
}

fn find_agent_transcript(agent: &AgentCall, transcript: &Path) -> Option<PathBuf> {
    let sibling = transcript.parent()?;
    let mut found = Vec::new();
    for candidate in &agent.candidates {
        let path = PathBuf::from(candidate);
        if path.extension().is_some_and(|extension| extension == "jsonl") {
            let path = if path.is_absolute() { path } else { sibling.join(path) };
            let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
            let looks_like_transcript = is_session_id(stem) || path.to_string_lossy().contains(".commandcode");
            if looks_like_transcript && path.is_file() && path != transcript {
                found.push(path);
            }
        } else {
            let path = sibling.join(format!("{candidate}.jsonl"));
            if path.is_file() && path != transcript {
                found.push(path);
            }
            for entry in std::fs::read_dir(cc_projects_root()).into_iter().flatten().flatten() {
                let path = entry.path().join(format!("{candidate}.jsonl"));
                if path.is_file() && path != transcript {
                    found.push(path);
                }
            }
        }
    }
    if found.is_empty() {
        for entry in std::fs::read_dir(sibling).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".jsonl").unwrap_or_default();
            if transcript_file(&name) && (stem == agent.id || stem.contains(&agent.id)) {
                let path = entry.path();
                if path != transcript {
                    found.push(path);
                }
            }
        }
    }
    one_candidate(&mut found)
}

fn commandcode_children(
    parsed: &Parsed,
    transcript: &Path,
    depth: u32,
    queue: &mut VecDeque<(Child, u32)>,
    entries: &mut Vec<Entry>,
) {
    if depth >= 3 {
        return;
    }
    for agent in &parsed.agents {
        match find_agent_transcript(agent, transcript) {
            Some(path) => queue.push_back((Child::CommandCode(path), depth + 1)),
            None => entries.push(Entry {
                numbers: Numbers::default(),
                label: format!("subagent {}", agent.id),
                depth: depth + 1,
                note: Some("transcript not found".into()),
            }),
        }
    }
}

fn read_children(
    mut queue: VecDeque<(Child, u32)>,
    entries: &mut Vec<Entry>,
    problems: &mut Vec<String>,
    config: &Config,
    folder: &Path,
    visited: &mut BTreeSet<String>,
) {
    while let Some((child, depth)) = queue.pop_front() {
        match child {
            Child::Opencode(id) => {
                if !visited.insert(id.clone()) {
                    continue;
                }
                let label = format!("subagent {id}");
                match export_session(config, folder, &id) {
                    Ok(text) => match parse_export(&text) {
                        Ok(parsed) => {
                            if depth < 3 {
                                for child in parsed.children {
                                    queue.push_back((Child::Opencode(child), depth + 1));
                                }
                            }
                            entries.push(Entry {
                                numbers: numbers(&parsed.steps, &parsed.calls, &[]),
                                label,
                                depth,
                                note: parsed.note,
                            });
                        }
                        Err(e) => {
                            let words = format!("cannot read the export of {id}: {e}");
                            problems.push(words.clone());
                            entries.push(Entry { numbers: Numbers::default(), label, depth, note: Some(words) });
                        }
                    },
                    Err(e) => {
                        let words = format!("cannot export {id}: {e}");
                        problems.push(words.clone());
                        entries.push(Entry { numbers: Numbers::default(), label, depth, note: Some(words) });
                    }
                }
            }
            Child::CommandCode(path) => {
                if !visited.insert(path.display().to_string()) {
                    continue;
                }
                let label =
                    format!("subagent {}", path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("transcript"));
                match read_text(&path) {
                    Ok(text) => {
                        let parsed = parse_commandcode_transcript(text.lines().map(str::to_string));
                        commandcode_children(&parsed, &path, depth, &mut queue, entries);
                        entries.push(Entry {
                            numbers: numbers(&parsed.steps, &parsed.calls, &parsed.marked),
                            label,
                            depth,
                            note: parsed.note,
                        });
                    }
                    Err(e) => {
                        let words = format!("cannot read {}: {e}", path.display());
                        problems.push(words.clone());
                        entries.push(Entry { numbers: Numbers::default(), label, depth, note: Some(words) });
                    }
                }
            }
        }
    }
}

fn jobs_of(folder: &Path, problems: &mut Vec<String>) -> u64 {
    let path = folder.join("brief.md");
    match read_text(&path) {
        Ok(text) => jobs_in(&text),
        Err(_) if !path.exists() => 1,
        Err(e) => {
            problems.push(format!("cannot read {}: {e}", path.display()));
            1
        }
    }
}

fn git_numstat(worktree: &str, base: &str) -> Result<u64, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(worktree)
        .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
        .args(["diff", "--numstat", "--no-renames", base])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::no_console(&mut cmd);
    let out = cmd.output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let text = text.trim();
        return Err(if text.is_empty() {
            format!("git exited with code {}", out.status.code().unwrap_or(-1))
        } else {
            cut(text, 400)
        });
    }
    Ok(numstat_lines(&String::from_utf8_lossy(&out.stdout)))
}

fn changed_lines(state: &Path, task: &crate::task::types::Task, problems: &mut Vec<String>) -> Option<u64> {
    let patch = state.join("patches").join(format!("{}.patch", task.name));
    match read_text(&patch) {
        Ok(text) => return Some(patch_lines(&text)),
        Err(_) if !patch.exists() => {}
        Err(e) => problems.push(format!("cannot read {}: {e}", patch.display())),
    }
    let worktree = task.worktree.as_deref().filter(|path| Path::new(path).is_dir())?;
    match git_numstat(worktree, &task.base) {
        Ok(lines) => Some(lines),
        Err(e) => {
            problems.push(format!("cannot count changed lines in {worktree}: {e}"));
            None
        }
    }
}

pub(super) fn task_result(
    state: &Path,
    config: &Config,
    task: &crate::task::types::Task,
    workers: &[worker::Worker],
) -> TaskStatsResult {
    let folder = crate::task::task_folder(state, &task.name);
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let mut queue = VecDeque::new();
    let mut visited = BTreeSet::new();
    for turn in &task.turns {
        if let Some(session) = &turn.session {
            visited.insert(session.clone());
        }
        let label = format!("turn {} ({})", turn.n, turn.worker);
        let cli = turn.cli_or_lookup(workers);
        let path = folder.join(format!("turn-{}.jsonl", turn.n));
        let (numbers, note, children, mut trouble) = read_turn(cli, &path, &turn.usage);
        problems.append(&mut trouble);
        for child in children {
            queue.push_back((Child::Opencode(child), 1));
        }
        entries.push(Entry { numbers, label, depth: 0, note });
    }
    read_children(queue, &mut entries, &mut problems, config, Path::new(&task.repo), &mut visited);
    let jobs = jobs_of(&folder, &mut problems);
    let changed = changed_lines(state, task, &mut problems);
    finish(task.name.clone(), entries, problems, Some(jobs), changed)
}

pub(super) fn session_result(config: &Config, folder: &Path, session: &str) -> Result<TaskStatsResult, RpcError> {
    let text = export_session(config, folder, session).map_err(|e| {
        RpcError::new(
            "unavailable",
            format!("cannot export session {session}: {e}"),
            "check that OpenCode can run in the folder",
        )
    })?;
    let parsed = parse_export(&text).map_err(|e| {
        RpcError::new("unavailable", format!("cannot read the export of {session}: {e}"), "try the export again")
    })?;
    let numbers = numbers(&parsed.steps, &parsed.calls, &[]);
    let mut entries = vec![Entry { numbers, label: format!("session {session}"), depth: 0, note: parsed.note }];
    let mut problems = Vec::new();
    let mut queue = VecDeque::new();
    for child in parsed.children {
        queue.push_back((Child::Opencode(child), 1));
    }
    let mut visited = BTreeSet::from([session.to_string()]);
    read_children(queue, &mut entries, &mut problems, config, folder, &mut visited);
    Ok(finish(session.to_string(), entries, problems, None, None))
}

pub(super) fn commandcode_result(value: &str, folder: &Path, config: &Config) -> Result<TaskStatsResult, RpcError> {
    let path = commandcode_transcript(value, folder)?;
    let subject =
        path.file_stem().and_then(|stem| stem.to_str()).filter(|stem| !stem.is_empty()).unwrap_or(value).to_string();
    let text = read_text(&path).map_err(|e| {
        RpcError::new(
            "unavailable",
            format!("cannot read {}: {e}", path.display()),
            "check the transcript's permissions",
        )
    })?;
    let parsed = parse_commandcode_transcript(text.lines().map(str::to_string));
    let numbers = numbers(&parsed.steps, &parsed.calls, &parsed.marked);
    let mut entries = vec![Entry { numbers, label: format!("session {subject}"), depth: 0, note: parsed.note.clone() }];
    let mut problems = Vec::new();
    let mut queue = VecDeque::new();
    commandcode_children(&parsed, &path, 0, &mut queue, &mut entries);
    let mut visited = BTreeSet::from([path.display().to_string()]);
    read_children(queue, &mut entries, &mut problems, config, folder, &mut visited);
    Ok(finish(subject, entries, problems, None, None))
}
