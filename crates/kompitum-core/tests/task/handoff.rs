//! The hand-off between workers, and a task stranded with every worker out.

use super::*;

const MODEL: &str = "deepseek/deepseek-v4.1-flash";

fn event(value: Value) -> String {
    json!({"type": "event", "event": value}).to_string()
}

fn cc_result(session: &str, subtype: &str, error: &str, final_text: &str) -> String {
    json!({"type": "result", "subtype": subtype, "sessionId": session,
        "usage": {"inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheWriteTokens": 0},
        "durationMs": 10, "finalText": final_text, "error": error})
    .to_string()
}

/// A Command Code turn refused at once, before a step: no work and no cost.
fn refused_at_once() -> Vec<Value> {
    let mut actions = say(&[
        event(json!({"type": "run_start", "sessionId": "s-at-once"})),
        event(json!({"type": "turn_start", "turnNumber": 1})),
        event(json!({"type": "model_request_start", "model": MODEL})),
        cc_result("s-at-once", "error", "Error: Insufficient credits for Command Code.", ""),
    ]);
    actions.push(json!({"err": "Error: Insufficient credits for Command Code."}));
    actions.push(json!({"exit": 1}));
    actions
}

/// A Command Code turn that writes a file, reads one and thinks, then is refused.
pub(super) fn refused_after_work() -> Vec<Value> {
    let mut actions = vec![json!({"write": ["made.txt", "hello"]})];
    actions.extend(say(&[
        event(json!({"type": "run_start", "sessionId": "s-handoff"})),
        event(json!({"type": "turn_start", "turnNumber": 1})),
        event(json!({"type": "model_request_start", "model": MODEL})),
        event(json!({"type": "thinking_end", "text": "I wrote made.txt and read notes.txt."})),
        event(json!({"type": "tool_queued", "toolCallId": "call_1", "toolName": "read_file",
            "input": {"file_path": "notes.txt"}})),
        event(json!({"type": "model_request_end", "model": MODEL,
            "usage": {"inputTokens": 100, "outputTokens": 10, "cacheReadTokens": 0, "cacheWriteTokens": 0},
            "stopReason": "stop", "effort": "max"})),
        event(json!({"type": "run_error", "error": {"name": "TransportError",
            "message": "POST /alpha/generate → 400 error: You have insufficient credits to make this request."}})),
        cc_result("s-handoff", "error", "Error: Insufficient credits for Command Code.", ""),
    ]));
    actions.push(json!({"err": "Reasoning effort set to max.\nError: Insufficient credits for Command Code."}));
    actions.push(json!({"exit": 1}));
    actions
}

/// An OpenCode turn that does work and is then told its weekly limit is out.
fn free_refused_after_work() -> Vec<Value> {
    let mut actions = vec![json!({"write": ["free.txt", "free work"]})];
    actions.extend(say(&[
        json!({"type": "step_start", "sessionID": "ses_free", "part": {"type": "step-start"}}).to_string(),
        json!({"type": "text", "sessionID": "ses_free",
            "part": {"type": "text", "text": "I did the first part"}})
        .to_string(),
        json!({"type": "reasoning", "sessionID": "ses_free",
            "part": {"type": "reasoning", "text": "I did the first part"}})
        .to_string(),
        json!({"type": "error", "sessionID": "ses_free", "error": {"name": "ProviderError",
            "data": {"code": "429", "message": "reached your weekly usage limit"}}})
        .to_string(),
        json!({"type": "step_finish", "sessionID": "ses_free",
            "part": {"type": "step-finish", "cost": 0.01,
                "tokens": {"input": 100, "output": 10, "reasoning": 0, "cache": {"read": 0, "write": 0}}}})
        .to_string(),
    ]));
    actions.push(json!({"exit": 0}));
    actions
}

pub(super) fn go_good(kept: Option<&Path>) -> Vec<Value> {
    let mut actions = Vec::new();
    if let Some(path) = kept {
        actions.push(json!({"file_arg_to": path.display().to_string()}));
    }
    actions.extend(say(&stream("ses_go", "go did it")));
    actions.push(json!({"exit": 0}));
    actions
}

fn block_out(s: &Session, worker: &str, day: &str, reason: &str) {
    s.json(&["workers", "--block", worker, "--until", day, "--reason", reason]);
}

fn block_cc_and_go(s: &Session) {
    block_out(s, "cc", "2099-01-01", "a");
    block_out(s, "go", "2099-01-02", "b");
}

#[test]
fn a_refused_turn_hands_off_to_the_next_worker_with_a_note() {
    let s = Session::new();
    let repo = s.repo();
    let projects = s.root.join("cc-projects").join("proj");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(projects.join("s-handoff.jsonl"), json!({"usage": {"costUsd": 0.13}}).to_string() + "\n").unwrap();
    let kept = s.root.join("handoff-kept.md");
    let script = s.script("handoff", json!([refused_after_work(), go_good(Some(&kept))]));
    s.config(&script, "");
    let brief = "write made.txt and finish\n";
    let started = s.start("hand", brief, &repo, &[]);
    assert_eq!(started["worker"], "Command Code", "{started}");

    let mut waiting = s
        .cmd()
        .args(["task", "wait", "hand", "--timeout", "60"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    assert!(waiting.try_wait().unwrap().is_none(), "the wait stayed across the hand-off");
    let out = waiting.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    let row: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(row["state"], "done", "{row}");
    assert_eq!(row["worker"], "opencode Go", "{row}");
    assert_eq!(row["workers"], json!(["cc", "go"]), "{row}");
    assert_eq!(row["turns"], 2, "{row}");

    let view = s.task("hand");
    let sentence =
        "Command Code ran out of credits after 1 step and $0.13, so opencode Go went on from 1 changed file.";
    assert_eq!(view["task"]["handoffs"], json!([sentence]), "{view}");
    let changed = view["task"]["changed"].as_array().unwrap();
    assert!(changed.iter().any(|file| file["path"] == "made.txt"), "{view}");
    let workers = s.json(&["workers"]);
    assert_eq!(workers["workers"][0]["state"], "out", "{workers}");
    assert_eq!(workers["workers"][0]["window"], "credits", "{workers}");

    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.contains("`made.txt` +1 -0, 5 bytes"), "{prompt}");
    assert!(prompt.contains("`notes.txt`"), "{prompt}");
    assert!(prompt.contains("I wrote made.txt and read notes.txt."), "{prompt}");
    assert!(!prompt.contains("opencode free"), "{prompt}");
    assert!(prompt.contains("Command Code ran out of credits after 1 step and $0.13."), "{prompt}");
    let after = &prompt[prompt.find("THE BRIEF FOLLOWS.").expect("the note says where the brief is")..];
    assert!(after.contains(brief.trim_end()), "{after}");
    let folder = Path::new(view["folder"].as_str().unwrap());
    assert!(folder.join("turn-2.note.md").is_file(), "the note is on disk");
}

#[test]
fn a_worker_refused_at_once_restarts_with_no_note() {
    let s = Session::new();
    let repo = s.repo();
    let kept = s.root.join("fresh-kept.md");
    let script = s.script("fresh", json!([refused_at_once(), go_good(Some(&kept))]));
    s.config(&script, "");
    let brief = "the whole brief\nwith two lines\n";
    s.start("fresh", brief, &repo, &[]);
    let (exit, rows, summary) = s.wait(&["fresh"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(
        rows[0]["handoffs"],
        json!(["Command Code refused it at once, so opencode Go started it fresh."]),
        "{rows:?}"
    );
    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.starts_with(brief.trim_end()), "no note, so it starts with the brief: {prompt}");
    assert!(prompt.contains("# Limits on this task (added by Kompitum)"), "{prompt}");
    assert!(!prompt.contains("You are taking over from another worker"), "no note: {prompt}");
}

#[test]
fn a_note_survives_a_turn_that_did_nothing() {
    let s = Session::new();
    let repo = s.repo();
    let kept = s.root.join("survives-kept.md");
    let script = s.script("survives", json!([free_refused_after_work(), refused_at_once(), go_good(Some(&kept))]));
    s.config(&script, "");
    s.start("survives", "brief", &repo, &["--worker", "free"]);
    let (exit, rows, summary) = s.wait(&["survives"], &["--timeout", "90"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["worker"], "opencode Go", "{rows:?}");
    assert_eq!(rows[0]["workers"], json!(["free", "cc", "go"]), "{rows:?}");
    let view = s.task("survives");
    assert_eq!(view["task"]["handoffs"].as_array().unwrap().len(), 2, "{view}");
    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.contains("opencode free hit its weekly limit after 1 step and $0.01."), "{prompt}");
    assert!(prompt.contains("I did the first part"), "free's note reached Go: {prompt}");
    assert!(prompt.contains("`free.txt`"), "{prompt}");
}

/// The real path cannot reach eight turns: every refusal blocks its worker for
/// at least ten minutes and a chain has three workers, so the eighth turn is
/// hours away. The rule itself is pure and tested here.
#[test]
fn a_refused_task_stops_at_the_turn_limit() {
    let limit = kompitum_core::task::handoff::stopped_at_turn_limit;
    assert_eq!(limit(7, "QUOTA EXHAUSTED: Command Code is out of credits"), None);
    assert_eq!(
        limit(8, "QUOTA EXHAUSTED: Command Code is out of credits"),
        Some("stopped after 8 turns: QUOTA EXHAUSTED: Command Code is out of credits".into())
    );
}

#[test]
fn a_worker_that_dies_at_its_start_is_started_again() {
    let s = Session::new();
    let repo = s.repo();
    let died = vec![json!({"err": "Error: Standalone server exited before reporting readiness"}), json!({"exit": 1})];
    let script = s.script("dies-once", json!([died, go_good(None)]));
    s.config(&script, "");
    s.start("dies-once", "brief", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["dies-once"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["turns"], 2, "{rows:?}");
    let view = s.task("dies-once");
    assert_eq!(view["task"]["turns"][0]["died_at_start"], true, "{view}");
    let warnings = view["task"]["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|warning| warning.as_str().unwrap()
            == "opencode Go died at its start, so it was started again: Error: Standalone server exited before reporting readiness"),
        "{view}"
    );
}

#[test]
fn a_worker_that_keeps_dying_fails_after_three_starts() {
    let s = Session::new();
    let repo = s.repo();
    let died = vec![json!({"err": "Error: Standalone server exited before reporting readiness"}), json!({"exit": 1})];
    let script = s.script("dies-always", json!([died]));
    s.config(&script, "");
    s.start("dies-always", "brief", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["dies-always"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "failed", "{rows:?}");
    assert_eq!(rows[0]["turns"], 3, "{rows:?}");
    assert_eq!(rows[0]["error"], "worker exited with code 1", "{rows:?}");
    let view = s.task("dies-always");
    assert_eq!(view["task"]["turns"][0]["died_at_start"], true, "{view}");
    assert_eq!(view["task"]["turns"][1]["died_at_start"], true, "{view}");
    assert_eq!(view["task"]["turns"][2]["died_at_start"], false, "{view}");
}

#[test]
fn all_workers_out_strands_the_task_and_wakes_the_wait_naming_each_reset() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("out", json!([go_good(None)]));
    s.config(&script, "");
    block_cc_and_go(&s);
    let started = s.start("nothing", "brief", &repo, &[]);
    assert_eq!(started["state"], "stranded", "{started}");
    let error = started["error"].as_str().unwrap();
    assert!(error.starts_with("ALL WORKERS OUT: "), "{error}");
    assert!(error.contains("Command Code is out by a person's word (a), back on"), "{error}");
    assert!(error.contains("opencode Go is out by a person's word (b), back on"), "{error}");

    let (exit, rows, summary) = s.wait(&["nothing"], &["--timeout", "20"]);
    assert_eq!(exit, 1, "{rows:?} {summary}");
    assert_eq!(rows[0]["state"], "stranded", "{rows:?}");
    assert_eq!(summary["summary"]["stranded"], json!(["nothing"]), "{summary}");
    assert_eq!(rows[0]["error"], started["error"], "the wait names each reset");
    s.json(&["task", "cancel", "nothing"]);
}

/// A calendar clock a few seconds ahead, which `--until` understands.
fn in_seconds(ms: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let at = kompitum_core::task::clock::civil(now.saturating_add(ms), kompitum_core::platform::utc_offset_minutes());
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", at.year, at.month, at.day, at.hour, at.minute, at.second)
}

#[test]
fn a_stranded_task_goes_on_when_a_block_lapses_or_is_cleared() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("lapse", json!([go_good(None)]));
    s.config(&script, "");
    s.json(&["workers", "--block", "go", "--until", &in_seconds(6_000), "--reason", "short"]);
    let started = s.start("lapse", "brief", &repo, &["--worker", "go"]);
    assert_eq!(started["state"], "stranded", "{started}");
    wait_until("the block lapsed and the task started", || {
        matches!(s.task("lapse")["task"]["state"].as_str().unwrap(), "running" | "done")
    });
    let (exit, rows, _) = s.wait(&["lapse"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");

    let kept = s.root.join("clear-kept.md");
    let script = s.script("clear", json!([free_refused_after_work(), go_good(Some(&kept))]));
    s.config(&script, "");
    block_cc_and_go(&s);
    s.start("cleared", "brief", &repo, &["--worker", "free"]);
    wait_until("free did its work and the task stranded", || s.task("cleared")["task"]["state"] == "stranded");
    let view = s.task("cleared");
    assert_eq!(view["task"]["worker"], "free", "{view}");
    s.json(&["workers", "--clear", "go"]);
    let (exit, rows, _) = s.wait(&["cleared"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["worker"], "opencode Go", "{rows:?}");
    assert_eq!(rows[0]["turns"], 2, "{rows:?}");
    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.contains("opencode free hit its weekly limit after 1 step and $0.01."), "{prompt}");
    assert!(prompt.contains("`free.txt`"), "{prompt}");
}

#[test]
fn a_stranded_task_fails_when_nothing_can_come_back() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("gone", json!([go_good(None)]));
    s.config(&script, "");
    s.json(&["workers", "--block", "go", "--until", &in_seconds(8_000), "--reason", "short"]);
    let started = s.start("gone", "brief", &repo, &["--worker", "go"]);
    assert_eq!(started["state"], "stranded", "{started}");
    // Its only worker goes away while it waits, so the block's end brings nothing back.
    s.config_for(&s.missing_cc(), &s.missing_cc(), "");
    let (exit, rows, _) = s.wait(&["gone"], &["--while-stranded", "--timeout", "40"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "failed", "{rows:?}");
    assert!(rows[0]["error"].as_str().is_some_and(|words| !words.is_empty()), "it says why: {rows:?}");
}

#[test]
fn a_stranded_task_can_be_cancelled_and_keeps_its_work() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("stranded-cancel", json!([free_refused_after_work()]));
    s.config(&script, "");
    block_cc_and_go(&s);
    s.start("keep", "brief", &repo, &["--worker", "free"]);
    wait_until("the task is stranded", || s.task("keep")["task"]["state"] == "stranded");
    let view = s.task("keep");
    let worktree = PathBuf::from(view["task"]["worktree"].as_str().unwrap());
    assert!(worktree.join("free.txt").is_file(), "the first worker's work is there");

    let (_, _, err) = s.out(&["task", "discard", "keep"]);
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    assert_eq!(error["error"]["kind"], "running", "{error}");

    let cancelled = s.json(&["task", "cancel", "keep"]);
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    assert!(worktree.join("free.txt").is_file(), "cancel keeps the work");
}

#[test]
fn a_hand_off_survives_a_core_restart() {
    let s = Session::new();
    let repo = s.repo();
    let kept = s.root.join("restart-kept.md");
    let script = s.script("restart-handoff", json!([free_refused_after_work(), go_good(Some(&kept))]));
    s.config(&script, "");
    block_cc_and_go(&s);
    s.start("restarted", "brief", &repo, &["--worker", "free"]);
    wait_until("the hand-off is stranded", || s.task("restarted")["task"]["state"] == "stranded");
    let view = s.task("restarted");
    let folder = PathBuf::from(view["folder"].as_str().unwrap());
    assert!(folder.join("turn-2.note.md").is_file(), "the note was written before the restart");

    let (code, _, err) = s.out(&["server", "restart"]);
    assert_eq!(code, 0, "{err}");
    // Command Code stays out, so the note goes to Go when its block is cleared.
    s.json(&["workers", "--clear", "go"]);
    let (exit, rows, _) = s.wait(&["restarted"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["turns"], 2, "{rows:?}");
    let view = s.task("restarted");
    let handoffs = view["task"]["handoffs"].as_array().unwrap();
    assert_eq!(handoffs.len(), 1, "{view}");
    assert!(handoffs[0].as_str().unwrap().starts_with("opencode free hit its weekly limit"), "{view}");
    let note = std::fs::read_to_string(folder.join("turn-2.note.md")).unwrap();
    let prompt = std::fs::read_to_string(folder.join("turn-2.prompt.md")).unwrap();
    assert!(prompt.starts_with(&note), "the prompt begins with its note");
    assert!(note.contains("`free.txt`"), "{note}");
}

#[test]
fn a_later_task_waits_behind_a_stranded_one_with_an_overlapping_scope() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("behind", json!([go_good(None)]));
    s.config(&script, "");
    block_cc_and_go(&s);
    let first = s.start("first", "brief", &repo, &["--scope", "src/**"]);
    assert_eq!(first["state"], "stranded", "{first}");
    let second = s.start("second", "brief", &repo, &["--scope", "src/x/**"]);
    assert_eq!(second["state"], "queued", "{second}");
    assert!(second["waiting"].as_str().unwrap().contains("first"), "{second}");
    s.json(&["task", "cancel", "first"]);
    s.json(&["task", "cancel", "second"]);
}

#[test]
fn a_wait_with_while_stranded_keeps_waiting() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("while-stranded", json!([go_good(None)]));
    s.config(&script, "");
    block_cc_and_go(&s);
    s.start("waited", "brief", &repo, &["--worker", "go"]);
    let mut waiting = s
        .cmd()
        .args(["task", "wait", "waited", "--while-stranded", "--timeout", "90"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    assert!(waiting.try_wait().unwrap().is_none(), "a stranded task does not end the wait");

    s.json(&["workers", "--clear", "cc"]);
    s.json(&["workers", "--clear", "go"]);
    let out = waiting.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    let row: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(row["state"], "done", "{row}");
}
