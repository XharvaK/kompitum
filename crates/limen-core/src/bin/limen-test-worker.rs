//! A stand-in for a worker CLI, for `tests/task.rs` only: it follows a small
//! JSON script of turns and prints whatever the test asks it to print.
//!
//! Usage: `limen-test-worker <script> <the arguments a real CLI would get>`.
//! `--version` is answered as OpenCode v2 does, so the core's version check
//! passes. The script is `{"turns": [[action, ...], ...]}`; the `.count` file
//! beside it says which turn runs next, and the last turn is repeated after.

use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Found on PATH, there is no script: just `<worker> --version`.
    if args.first().map(String::as_str) == Some("--version") {
        println!("v2.0.0");
        return;
    }
    let Some(script) = args.first().cloned() else {
        eprintln!("limen-test-worker: a script path is required");
        std::process::exit(2);
    };
    // The runner's route check: an export, never a turn, so the counter stays.
    if args.get(1).map(String::as_str) == Some("session") && args.get(2).map(String::as_str) == Some("export") {
        // The real CLI's failure when it reaches for a managed service it can't use.
        if !args.iter().any(|arg| arg == "--standalone") {
            eprintln!("Error: Managed service port 49374 on 127.0.0.1 is already in use by another process.");
            std::process::exit(1);
        }
        let value = std::fs::read_to_string(&script).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
        if let Some(ms) = value.as_ref().and_then(|value| value.get("export_sleep_ms")).and_then(Value::as_u64) {
            std::thread::sleep(Duration::from_millis(ms));
        }
        let export = value
            .as_ref()
            .and_then(|value| value.get("export"))
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| {
                r#"{"messages":[{"model":{"providerID":"opencode-go","id":"deepseek-v4.1-flash","variant":"max"}}]}"#
                    .into()
            });
        println!("{export}");
        return;
    }
    match args.get(1).map(String::as_str) {
        Some("--version") => {
            println!("v2.0.0");
            return;
        }
        Some("--child") => {
            // The grandchild of the `child` action: say who it is, then wait to be ended.
            if let Some(pidfile) = args.get(2) {
                let _ = std::fs::write(pidfile, std::process::id().to_string());
            }
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        _ => {}
    }
    let count_path = format!("{script}.count");
    let count: usize = std::fs::read_to_string(&count_path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let _ = std::fs::write(&count_path, (count + 1).to_string());
    let text = match std::fs::read_to_string(&script) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("limen-test-worker: cannot read {script}: {e}");
            std::process::exit(2);
        }
    };
    let turns = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("turns").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let Some(turn) = turns.get(count).or_else(|| turns.last()) else { return };
    if let Some(actions) = turn.as_array() {
        for action in actions {
            act(action, &script);
        }
    }
}

fn act(action: &Value, script: &str) {
    let Some((key, value)) = action.as_object().and_then(|o| o.iter().next()) else { return };
    match key.as_str() {
        "out" => {
            println!("{}", value.as_str().unwrap_or_default());
            let _ = std::io::stdout().flush();
        }
        "err" => {
            eprintln!("{}", value.as_str().unwrap_or_default());
            let _ = std::io::stderr().flush();
        }
        "write" => {
            let path = value.get(0).and_then(Value::as_str).unwrap_or_default();
            let text = value.get(1).and_then(Value::as_str).unwrap_or_default();
            if let Some(parent) = Path::new(path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, text);
        }
        "sleep_ms" => std::thread::sleep(Duration::from_millis(value.as_u64().unwrap_or(0))),
        "burn_ms" => burn(Duration::from_millis(value.as_u64().unwrap_or(0))),
        "child" => {
            let pidfile = value.as_str().unwrap_or_default();
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe)
                    .args([script, "--child", pidfile])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            }
        }
        "args_to" => {
            let path = value.as_str().unwrap_or_default();
            let mut text = String::new();
            for arg in std::env::args().skip(1) {
                text.push_str(&arg);
                text.push('\n');
            }
            let _ = std::fs::write(path, text);
        }
        "stdin_to" => {
            let path = value.as_str().unwrap_or_default();
            let mut text = String::new();
            let _ = std::io::stdin().read_to_string(&mut text);
            let _ = std::fs::write(path, text);
        }
        "env_to" => {
            let path = value.as_str().unwrap_or_default();
            let mut text = String::new();
            for (key, value) in std::env::vars() {
                text.push_str(&key);
                text.push('=');
                text.push_str(&value);
                text.push('\n');
            }
            let _ = std::fs::write(path, text);
        }
        "file_arg_to" => {
            let dest = value.as_str().unwrap_or_default();
            let args: Vec<String> = std::env::args().collect();
            if let Some(at) = args.iter().position(|a| a == "--file")
                && let Some(source) = args.get(at + 1)
                && let Ok(bytes) = std::fs::read(source)
            {
                let _ = std::fs::write(dest, bytes);
            }
        }
        "copy" => {
            let from = value.get(0).and_then(Value::as_str).unwrap_or_default();
            let to = value.get(1).and_then(Value::as_str).unwrap_or_default();
            if let Ok(bytes) = std::fs::read(from) {
                if let Some(parent) = Path::new(to).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(to, bytes);
            }
        }
        // A program the turn runs, and what it said, for a test to look at:
        // `exit=<code>` first, then its output, both streams together.
        "run" => {
            let words: Vec<&str> = value.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let Some((to_file, rest)) = words.split_first() else { return };
            let program = rest.first().copied().unwrap_or_default();
            let mut command = std::process::Command::new(program);
            command.args(rest.get(1..).unwrap_or_default()).stdin(std::process::Stdio::null());
            let (code, output) = match command.output() {
                Ok(out) => (
                    out.status.code().unwrap_or(-1),
                    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
                ),
                Err(e) => (127, format!("cannot run {program}: {e}\n")),
            };
            let _ = std::fs::write(to_file, format!("exit={code}\n{output}"));
        }
        "exit" => std::process::exit(value.as_i64().unwrap_or(0) as i32),
        _ => {}
    }
}

/// Spend the time doing arithmetic the optimizer may not remove.
fn burn(duration: Duration) {
    let start = Instant::now();
    let mut n: u64 = 0;
    while start.elapsed() < duration {
        for _ in 0..10_000 {
            n = std::hint::black_box(n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407));
        }
    }
    std::hint::black_box(n);
}
