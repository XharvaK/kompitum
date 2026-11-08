//! Who started a task, the table that guards it, `lc status`, and its `--line`.

use super::*;
use std::io::Write;

fn run_as(s: &Session, by: &str, args: &[&str]) -> (i32, String, String) {
    let out = s.cmd().env("LIMEN_AGENT", by).args(args).output().unwrap();
    if !out.stderr.is_empty() {
        eprintln!("{args:?} as {by}: {}", String::from_utf8_lossy(&out.stderr));
    }
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn json_as(s: &Session, by: &str, args: &[&str]) -> Value {
    let (_, text, _) = run_as(s, by, args);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{args:?} as {by}: {e}: {text}"))
}

fn error_of((code, stdout, stderr): (i32, String, String)) -> Value {
    assert_ne!(code, 0, "expected a refusal: {stdout}{stderr}");
    let text = if stderr.trim().is_empty() { &stdout } else { &stderr };
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn start_as(s: &Session, by: &str, name: &str, repo: &Path, extra: &[&str]) -> Value {
    let brief = s.brief(name, "do the work");
    let folder = repo.display().to_string();
    let mut args = vec!["task", "start", name, "--brief", &brief, "--in", &folder];
    args.extend_from_slice(extra);
    json_as(s, by, &args)
}

/// Start one task and wait out the start slot until its worker runs.
fn start_running_as(s: &Session, by: &str, name: &str, repo: &Path, extra: &[&str]) -> Value {
    let started = start_as(s, by, name, repo, extra);
    assert!(matches!(started["state"].as_str(), Some("running" | "queued")), "{started}");
    wait_until("the task started", || s.task(name)["task"]["state"] == "running");
    started
}

/// Start one task as `by` and wait for it to finish.
fn finish_as(s: &Session, by: &str, name: &str, repo: &Path) -> Value {
    let mut actions = vec![json!({"write": [format!("made-{name}.txt"), "hello"]})];
    actions.extend(say(&stream(&format!("ses_{name}"), "done\n\n## Gaps\nNone.\n")));
    actions.push(json!({"exit": 0}));
    let script = s.script(name, json!([actions]));
    s.config(&script, "");
    let started = start_as(s, by, name, repo, &["--worker", "go"]);
    assert!(matches!(started["state"].as_str(), Some("running" | "queued")), "{started}");
    let (exit, rows, summary) = s.wait(&[name], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    rows[0].clone()
}

#[test]
fn each_callers_word_is_recorded_and_shown() {
    let s = Session::new();
    let repo = s.repo();
    for by in ["opus", "manager", "owner"] {
        finish_as(&s, by, &format!("t-{by}"), &repo);
    }
    let list = s.json(&["task", "ls", "--all"]);
    let rows = list["tasks"].as_array().unwrap();
    for by in ["opus", "manager", "owner"] {
        let name = format!("t-{by}");
        let row = rows.iter().find(|row| row["task"] == name.as_str()).unwrap_or_else(|| panic!("{name}: {list}"));
        assert_eq!(row["by"], by, "{list}");
        let view = s.task(&name);
        assert_eq!(view["task"]["by"], by, "{view}");
        assert_eq!(view["line"]["by"], by, "{view}");
        let review = s.json(&["--json", "task", "review", &name]);
        assert!(review["text"].as_str().unwrap().contains(&format!("by {by}")), "{review}");
        let accepted = json_as(&s, "owner", &["task", "accept", &name, "--review", "good"]);
        assert_eq!(accepted["state"], "accepted", "{accepted}");
        let ledger = s.json(&["ledger", "--json", "--task", &name]);
        assert_eq!(ledger["rows"][0]["by"], by, "{ledger}");
    }
}

#[test]
fn a_managers_accept_needs_the_owners_words() {
    let s = Session::new();
    let repo = s.repo();
    finish_as(&s, "manager", "held", &repo);
    let refused = error_of(run_as(&s, "manager", &["task", "accept", "held", "--review", "sound"]));
    assert_eq!(refused["error"]["kind"], "refused", "{refused}");
    assert!(refused["error"]["hint"].as_str().unwrap().contains("--owner-said"), "{refused}");
    assert_eq!(s.task("held")["task"]["state"], "done", "a refusal changes nothing");

    let accepted =
        json_as(&s, "manager", &["task", "accept", "held", "--review", "sound", "--owner-said", "I read it"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    let ledger = s.json(&["ledger", "--json", "--task", "held"]);
    assert_eq!(ledger["rows"][0]["review"], "Owner: \"I read it\". sound", "{ledger}");

    finish_as(&s, "opus", "spoken", &repo);
    let accepted = json_as(&s, "opus", &["task", "accept", "spoken", "--review", "fine", "--owner-said", "yes do"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    let ledger = s.json(&["ledger", "--json", "--task", "spoken"]);
    assert_eq!(ledger["rows"][0]["review"], "Owner: \"yes do\". fine", "{ledger}");
}

#[test]
fn another_callers_task_needs_yes_or_the_owner() {
    let s = Session::new();
    let repo = s.repo();
    let long = s.script("theirs", json!([[{"sleep_ms": 60000}]]));
    s.config(&long, "");
    start_running_as(&s, "opus", "theirs", &repo, &["--worker", "go"]);
    let refused = error_of(run_as(&s, "manager", &["task", "cancel", "theirs"]));
    assert_eq!(refused["error"]["kind"], "refused", "{refused}");
    assert!(refused["error"]["message"].as_str().unwrap().contains("opus"), "{refused}");
    assert!(refused["error"]["hint"].as_str().unwrap().contains("--yes"), "{refused}");
    assert_eq!(s.task("theirs")["task"]["state"], "running", "a refusal changes nothing");
    let cancelled = json_as(&s, "owner", &["task", "cancel", "theirs"]);
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");

    let long = s.script("mine", json!([[{"sleep_ms": 60000}]]));
    s.config(&long, "");
    start_running_as(&s, "manager", "mine", &repo, &["--worker", "go"]);
    let cancelled = json_as(&s, "manager", &["task", "cancel", "mine"]);
    assert_eq!(cancelled["state"], "cancelled", "its own task needs no flag: {cancelled}");

    finish_as(&s, "manager", "managed", &repo);
    let refused = error_of(run_as(&s, "opus", &["task", "discard", "managed"]));
    assert_eq!(refused["error"]["kind"], "refused", "{refused}");
    assert!(refused["error"]["message"].as_str().unwrap().contains("manager"), "{refused}");
    let discarded = json_as(&s, "opus", &["task", "discard", "managed", "--yes"]);
    assert_eq!(discarded["state"], "discarded", "{discarded}");

    finish_as(&s, "opus", "owned", &repo);
    let discarded = json_as(&s, "owner", &["task", "discard", "owned"]);
    assert_eq!(discarded["state"], "discarded", "{discarded}");
}

#[test]
fn status_is_the_whole_picture() {
    let s = Session::new();
    let repo = s.repo();
    finish_as(&s, "owner", "ended-1", &repo);
    let long = s.script("status-run", json!([[{"sleep_ms": 60000}]]));
    s.config(&long, "");
    start_running_as(&s, "manager", "running-1", &repo, &["--worker", "go"]);
    let state = s.state();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("claude-limits.json"),
        json!({
            "five_hour_resets_at": 1762560000u64,
            "seven_day_resets_at": 1762900000u64,
            "seen_at": 1762550000000u64
        })
        .to_string(),
    )
    .unwrap();

    let status = s.json(&["status", "--json"]);
    assert!(status["agents"].is_array(), "{status}");
    let review = status["tasks"]["review"].as_array().unwrap();
    assert_eq!(review.len(), 1, "{status}");
    assert_eq!(review[0]["name"], "ended-1", "{status}");
    assert_eq!(review[0]["by"], "owner", "{status}");
    assert_eq!(review[0]["repo"], s.task("ended-1")["task"]["repo"], "{status}");
    assert_eq!(review[0]["worker"], "opencode Go", "{status}");
    assert_eq!(review[0]["state"], "done", "{status}");
    assert!(review[0]["seconds"].is_number(), "{status}");
    assert!(review[0]["cost_usd"].is_number(), "{status}");
    assert!(review[0]["checks"].is_string(), "{status}");
    let running = status["tasks"]["running"].as_array().unwrap();
    assert_eq!(running.len(), 1, "{status}");
    assert_eq!(running[0]["name"], "running-1", "{status}");
    assert_eq!(running[0]["by"], "manager", "{status}");
    assert!(status["tasks"]["queued"].as_array().unwrap().is_empty(), "{status}");
    assert!(status["tasks"]["recent"].as_array().unwrap().is_empty(), "{status}");
    assert_eq!(status["workers"], s.json(&["workers", "--json"]), "the workers block is `lc workers`");
    assert_eq!(status["claude"]["five_hour_resets_at"], 1762560000u64, "{status}");
    assert_eq!(status["claude"]["seven_day_resets_at"], 1762900000u64, "{status}");
    assert_eq!(status["claude"]["seen_at"], 1762550000000u64, "{status}");
    assert_eq!(status["waiting"]["review"], 1, "{status}");
    assert_eq!(status["waiting"]["blocked"], 0, "{status}");

    json_as(&s, "owner", &["task", "cancel", "running-1"]);
    let accepted = json_as(&s, "owner", &["task", "accept", "ended-1", "--review", "good"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    let status = s.json(&["status", "--json"]);
    let review = status["tasks"]["review"].as_array().unwrap();
    assert_eq!(review.len(), 1, "the cancelled task waits for a look too: {status}");
    assert_eq!(review[0]["name"], "running-1", "{status}");
    assert_eq!(review[0]["state"], "cancelled", "{status}");
    let recent = status["tasks"]["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 1, "{status}");
    assert_eq!(recent[0]["name"], "ended-1", "{status}");
}

fn status_line(s: &Session, input: &str) -> (i32, String) {
    let mut child = s
        .cmd()
        .args(["status", "--line"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn the_status_line_keeps_claudes_limits_and_never_fails() {
    let s = Session::new();
    let limits = s.state().join("claude-limits.json");
    let input = json!({
        "model": {"display_name": "Claude Opus 4.5"},
        "rate_limits": {"five_hour": {"resets_at": 1762560000u64}, "seven_day": {"resets_at": 1762900000u64}}
    })
    .to_string();
    let (code, text) = status_line(&s, &input);
    assert_eq!(code, 0);
    assert_eq!(text.lines().count(), 1, "one line: {text:?}");
    assert!(text.contains("Claude Opus 4.5"), "{text}");
    let at = limen_core::task::clock::civil(1_762_560_000_000, limen_core::platform::utc_offset_minutes());
    assert!(text.contains(&format!("5h resets {:02}:{:02}", at.hour, at.minute)), "{text}");
    let saved: Value = serde_json::from_slice(&std::fs::read(&limits).unwrap()).unwrap();
    assert_eq!(saved["five_hour_resets_at"], 1762560000u64, "{saved}");
    assert_eq!(saved["seven_day_resets_at"], 1762900000u64, "{saved}");
    assert!(saved["seen_at"].as_u64().unwrap() > 1_700_000_000_000, "{saved}");

    let before = std::fs::read(&limits).unwrap();
    let (code, text) = status_line(&s, &json!({"model": {"display_name": "Claude"}}).to_string());
    assert_eq!(code, 0);
    assert!(text.contains("Claude") && !text.contains("5h resets"), "{text}");
    assert_eq!(std::fs::read(&limits).unwrap(), before, "with neither, the file is kept");

    let (code, text) = status_line(&s, "not json at all");
    assert_eq!(code, 0);
    assert_eq!(text, "\n", "bad input is one empty line");
    assert_eq!(std::fs::read(&limits).unwrap(), before);

    let (code, _) = status_line(&s, &json!({"rate_limits": {"seven_day": {"resets_at": 1762900000u64}}}).to_string());
    assert_eq!(code, 0);
    let saved: Value = serde_json::from_slice(&std::fs::read(&limits).unwrap()).unwrap();
    assert!(saved["five_hour_resets_at"].is_null(), "a missing window is null: {saved}");
    assert_eq!(saved["seven_day_resets_at"], 1762900000u64, "{saved}");
}
