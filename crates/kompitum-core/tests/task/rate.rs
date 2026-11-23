//! A rate limit waits on the same worker, and kom task retry ends the wait.

use super::*;

const MODEL: &str = "deepseek/deepseek-v4.1-flash";

fn event(value: Value) -> String {
    json!({"type": "event", "event": value}).to_string()
}

/// A Command Code turn the provider refused for its rate limit.
fn rate_refusal_actions() -> Vec<Value> {
    let mut actions = say(&[
        event(json!({"type": "run_start", "sessionId": "s-rate"})),
        event(json!({"type": "model_request_start", "model": MODEL})),
        json!({"type": "result", "subtype": "error", "sessionId": "s-rate",
            "usage": {"inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheWriteTokens": 0},
            "durationMs": 10, "finalText": "", "error": "rate limit exceeded"})
        .to_string(),
    ]);
    actions.push(json!({"exit": 1}));
    actions
}

/// A Command Code turn that ended well, in the shape a real one reports.
fn cc_good_actions(session: &str, text: &str) -> Vec<Value> {
    let mut actions = say(&[
        event(json!({"type": "run_start", "sessionId": session})),
        event(json!({"type": "model_request_start", "model": MODEL})),
        event(json!({"type": "text_delta", "delta": text})),
        event(json!({"type": "model_request_end", "model": MODEL,
            "usage": {"inputTokens": 10, "outputTokens": 3, "cacheReadTokens": 0, "cacheWriteTokens": 0},
            "stopReason": "stop", "effort": "max"})),
        json!({"type": "result", "subtype": "success", "sessionId": session,
            "usage": {"inputTokens": 10, "outputTokens": 3, "cacheReadTokens": 0, "cacheWriteTokens": 0},
            "durationMs": 10, "finalText": text})
        .to_string(),
    ]);
    actions.push(json!({"exit": 0}));
    actions
}

/// A credits refusal, which keeps today's behaviour: the worker is blocked at once.
fn credits_refusal_actions() -> Vec<Value> {
    let text = "You have insufficient credits to make this request.";
    let mut actions = say(&[
        event(json!({"type": "run_start", "sessionId": "s-credits"})),
        event(json!({"type": "model_request_start", "model": MODEL})),
        json!({"type": "result", "subtype": "error", "sessionId": "s-credits",
            "usage": {"inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheWriteTokens": 0},
            "durationMs": 10, "finalText": "", "error": text})
        .to_string(),
    ]);
    actions.push(json!({"exit": 1}));
    actions
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// The task's rate wait, once the pump has recorded the nth refusal.
fn rate_wait(s: &Session, name: &str, refusals: u64) -> Value {
    wait_until("the rate wait", || {
        let view = s.task(name);
        view["task"]["rate_refusals"].as_u64() == Some(refusals)
            && view["task"]["rate_wait_until_ms"].is_u64()
            && view["task"]["waiting"].as_str().is_some_and(|waiting| waiting.starts_with("rate limited:"))
    });
    s.task(name)
}

/// Workers start 8 s apart, so a retry in a test waits that out first.
fn wait_out_start_slot(s: &Session, name: &str) {
    let view = s.task(name);
    let started = view["task"]["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .and_then(|turn| turn["started_ms"].as_u64())
        .unwrap_or(0);
    let left = (started + 8_500).saturating_sub(now_ms());
    std::thread::sleep(Duration::from_millis(left));
}

fn assert_wait(view: &Value, expected_ms: u64) {
    assert_eq!(view["task"]["state"], "queued", "{view}");
    let until = view["task"]["rate_wait_until_ms"].as_u64().unwrap();
    let now = now_ms();
    let wait = until.saturating_sub(now);
    assert!(wait.abs_diff(expected_ms) < 10_000, "the wait should be about {expected_ms} ms, was {wait} ms: {view}");
    let waiting = view["task"]["waiting"].as_str().unwrap_or_default();
    assert!(waiting.starts_with("rate limited: trying Command Code again "), "{waiting}");
    assert!(waiting.contains(" at "), "{waiting}");
    let clock = waiting.rsplit(' ').next().unwrap_or_default();
    assert!(
        clock.len() == 5 && clock.as_bytes()[2] == b':' && clock[..2].parse::<u32>().is_ok(),
        "local time, 24-hour: {waiting}"
    );
}

fn cc_worker(s: &Session) -> Value {
    let workers = s.json(&["workers"]);
    workers["workers"].as_array().unwrap().iter().find(|worker| worker["key"] == "cc").unwrap().clone()
}

#[test]
fn a_rate_refusal_waits_a_minute_on_the_same_worker() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("rate-one", json!([rate_refusal_actions(), cc_good_actions("s-back", "back again")]));
    s.config(&script, "");
    let started = s.start("rate-1", "brief", &repo, &["--worker", "cc"]);
    assert_eq!(started["worker"], "Command Code", "{started}");

    let view = rate_wait(&s, "rate-1", 1);
    assert_wait(&view, 60_000);
    assert_eq!(view["task"]["worker"], "cc", "the same worker waits to try again: {view}");
    assert_eq!(view["task"]["rate_refusals"], 1, "{view}");
    assert_eq!(cc_worker(&s)["state"], "ready", "a rate wait does not block the worker");
}

#[test]
fn retry_during_a_rate_wait_starts_the_turn_at_once_on_the_same_worker() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("rate-retry", json!([rate_refusal_actions(), cc_good_actions("s-back", "back again")]));
    s.config(&script, "");
    s.start("rate-2", "brief", &repo, &["--worker", "cc"]);
    rate_wait(&s, "rate-2", 1);
    wait_out_start_slot(&s, "rate-2");

    let start = Instant::now();
    let retried = s.json(&["task", "retry", "rate-2"]);
    assert_eq!(retried["state"], "running", "retry starts the turn now: {retried}");
    assert_eq!(retried["worker"], "Command Code", "on the same worker: {retried}");
    let (exit, rows, _) = s.wait(&["rate-2"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert!(start.elapsed() < Duration::from_secs(30), "retry did not wait its minute");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["worker"], "Command Code", "{rows:?}");
    assert_eq!(rows[0]["turns"], 2, "{rows:?}");
    assert_eq!(s.task("rate-2")["task"]["rate_wait_until_ms"], Value::Null, "the wait was served");
}

#[test]
fn a_second_rate_refusal_waits_three_minutes() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script(
        "rate-two",
        json!([rate_refusal_actions(), rate_refusal_actions(), cc_good_actions("s-back", "back again")]),
    );
    s.config(&script, "");
    s.start("rate-3", "brief", &repo, &["--worker", "cc"]);
    rate_wait(&s, "rate-3", 1);
    wait_out_start_slot(&s, "rate-3");
    s.json(&["task", "retry", "rate-3"]);

    let view = rate_wait(&s, "rate-3", 2);
    assert_wait(&view, 180_000);
    assert_eq!(view["task"]["worker"], "cc", "{view}");
    assert_eq!(view["task"]["rate_refusals"], 2, "{view}");
    assert_eq!(cc_worker(&s)["state"], "ready", "the second wait does not block the worker either");
    s.json(&["task", "cancel", "rate-3"]);
}

#[test]
fn a_third_rate_refusal_blocks_the_worker_and_hands_the_turn_on() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script(
        "rate-three",
        json!([
            rate_refusal_actions(),
            rate_refusal_actions(),
            rate_refusal_actions(),
            say(&stream("ses_go", "go did it")),
        ]),
    );
    s.config(&script, "");
    s.start("rate-4", "brief", &repo, &[]);
    rate_wait(&s, "rate-4", 1);
    wait_out_start_slot(&s, "rate-4");
    s.json(&["task", "retry", "rate-4"]);
    rate_wait(&s, "rate-4", 2);
    wait_out_start_slot(&s, "rate-4");
    s.json(&["task", "retry", "rate-4"]);

    let (exit, rows, _) = s.wait(&["rate-4"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "the refused task goes on with Go: {rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["worker"], "opencode Go", "{rows:?}");
    assert_eq!(rows[0]["workers"], json!(["cc", "cc", "cc", "go"]), "{rows:?}");
    assert_eq!(cc_worker(&s)["state"], "out", "the third refusal blocks the worker");
}

#[test]
fn a_quota_refusal_still_blocks_the_worker_at_once() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("quota", json!([credits_refusal_actions()]));
    s.config_for(&s.missing_cc(), &script, "");
    s.start("quota-1", "brief", &repo, &["--worker", "cc"]);
    let (exit, rows, _) = s.wait(&["quota-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "stranded", "{rows:?}");
    assert_eq!(cc_worker(&s)["state"], "out", "a credits refusal blocks at once");
    let view = s.task("quota-1");
    assert_eq!(view["task"]["rate_refusals"], 0, "a quota refusal is not a rate wait: {view}");
    assert!(view["task"]["rate_wait_until_ms"].is_null(), "{view}");
    s.json(&["task", "cancel", "quota-1"]);
}

#[test]
fn retry_on_a_running_task_is_refused() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("running", json!([[{"sleep_ms": 60000}]]));
    s.config(&script, "");
    s.start("rate-5", "brief", &repo, &["--worker", "go"]);
    let (code, _, err) = s.out(&["task", "retry", "rate-5"]);
    assert_ne!(code, 0, "{err}");
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    assert_eq!(error["error"]["kind"], "invalid_params", "{error}");
    assert_eq!(error["error"]["message"], "rate-5 isn't waiting to retry", "{error}");
    s.json(&["task", "cancel", "rate-5"]);
}
