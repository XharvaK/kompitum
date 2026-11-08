//! `kom mcp` end to end: the real binary speaking JSON-RPC over its pipes.
//! Each test has its own session, like `tests/task.rs`, and stops any core it
//! started at the end.

use kompitum_core::platform;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_kompitum-core");

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    session: String,
    root: PathBuf,
}

impl Mcp {
    fn new(agent: &str) -> Mcp {
        static N: AtomicU32 = AtomicU32::new(0);
        let session = format!("test-mcp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
        let root = std::env::temp_dir().join(format!("kom-{session}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut cmd = env_cmd(&session, &root);
        cmd.args(["mcp", "--as", agent]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        platform::no_console(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, stdin, stdout, session, root }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        assert!(!line.trim().is_empty(), "mcp closed its stdout");
        serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}: {line}"))
    }

    fn ask(&mut self, message: Value) -> Value {
        self.send(message);
        self.recv()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.stdin.take();
        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        let mut stop = env_cmd(&self.session, &self.root);
        stop.args(["server", "stop"]);
        platform::no_console(&mut stop);
        let _ = stop.output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn env_cmd(session: &str, root: &Path) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.current_dir(root)
        .env("KOMPITUM_SESSION", session)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("KOMPITUM_SETTLE_MS", "0")
        .env_remove("KOMPITUM_AGENT")
        .env_remove("KOMPITUM_TERMINAL_ID")
        .env_remove("KOMPITUM_SCOPE")
        .env_remove("KOMPITUM_SOCKET")
        .env_remove("KOMPITUM_WORKER")
        .env_remove("KOMPITUM_COMSPEC")
        .env_remove("OPENCODE_DISABLE_PROJECT_CONFIG")
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_CLIENT")
        .env_remove("SSH_TTY");
    #[cfg(windows)]
    cmd.env("COMSPEC", real_cmd());
    cmd
}

#[cfg(windows)]
fn real_cmd() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    Path::new(&root).join(r"System32\cmd.exe").display().to_string()
}

#[test]
fn initialize_echoes_a_known_protocol_and_names_the_server() {
    let mut mcp = Mcp::new("test-agent");
    let reply = mcp.ask(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18"},
    }));
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18", "{reply}");
    assert_eq!(reply["result"]["serverInfo"]["name"], "kompitum", "{reply}");
    assert!(reply["result"]["instructions"].as_str().unwrap().contains("Here you are test-agent."), "{reply}");

    let reply = mcp.ask(json!({
        "jsonrpc": "2.0", "id": 2, "method": "initialize",
        "params": {"protocolVersion": "1999-01-01"},
    }));
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18", "{reply}");
}

#[test]
fn a_notification_gets_no_reply() {
    let mut mcp = Mcp::new("test-agent");
    mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    mcp.send(json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}));
    let reply = mcp.recv();
    assert_eq!(reply["id"], 7, "{reply}");
    assert_eq!(reply["result"], json!({}), "{reply}");
}

#[test]
fn tools_list_is_built_from_kom() {
    let mut mcp = Mcp::new("test-agent");
    let reply = mcp.ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let tools = reply["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    for name in [
        "task_start",
        "task_ls",
        "task_show",
        "task_wait",
        "task_review",
        "task_accept",
        "task_redo",
        "task_cancel",
        "task_discard",
        "workers",
        "ledger",
        "status",
    ] {
        assert!(names.contains(&name), "{name} missing from {names:?}");
    }
    let start = tools.iter().find(|tool| tool["name"] == "task_start").unwrap();
    let required: Vec<&str> =
        start["inputSchema"]["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    for name in ["name", "brief", "in"] {
        assert!(required.contains(&name), "{name} missing from {required:?}");
    }
    assert_eq!(start["inputSchema"]["properties"]["scope"]["type"], "array", "{start}");
    assert_eq!(start["inputSchema"]["properties"]["web"]["type"], "boolean", "{start}");
    for tool in tools {
        assert!(tool["inputSchema"]["properties"].get("json").is_none(), "{tool}");
    }
    let status = tools.iter().find(|tool| tool["name"] == "status").unwrap();
    assert!(status["inputSchema"]["properties"].get("line").is_none(), "{status}");
}

#[test]
fn bad_lines_methods_and_tools_are_protocol_errors() {
    let mut mcp = Mcp::new("test-agent");
    let reply = mcp.ask(json!({"jsonrpc": "2.0", "id": 3, "method": "nope"}));
    assert_eq!(reply["error"]["code"], -32601, "{reply}");

    writeln!(mcp.stdin.as_mut().unwrap(), "not json").unwrap();
    mcp.stdin.as_mut().unwrap().flush().unwrap();
    let reply = mcp.recv();
    assert_eq!(reply["error"]["code"], -32700, "{reply}");
    assert!(reply["id"].is_null(), "{reply}");

    let reply = mcp.ask(json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": {"name": "nope", "arguments": {}},
    }));
    assert_eq!(reply["error"]["code"], -32602, "{reply}");
    assert!(reply["error"]["message"].as_str().unwrap().contains("nope"), "{reply}");
}

#[test]
fn a_call_runs_kom_and_reports_its_error() {
    let mut mcp = Mcp::new("test-agent");
    let reply = mcp.ask(json!({
        "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": {"name": "task_show", "arguments": {"task": "missing-task"}},
    }));
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("missing-task"), "{text}");
}
