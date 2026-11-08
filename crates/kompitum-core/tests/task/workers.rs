//! The worker loop with two CLIs: Command Code's stream and transcript, the
//! chain and its blocks, the start slot, the route check and `kom workers`.

use super::*;

const MODEL: &str = "deepseek/deepseek-v4.1-flash";

fn event(value: Value) -> String {
    json!({"type": "event", "event": value}).to_string()
}

/// A Command Code turn in the shape of a real one.
fn cc_stream(session: &str, model: &str, text: &str) -> Vec<String> {
    vec![
        event(json!({"type": "run_start", "sessionId": session})),
        event(json!({"type": "turn_start", "turnNumber": 1})),
        event(json!({"type": "message_start"})),
        event(json!({"type": "model_request_start", "model": model})),
        event(json!({"type": "tool_queued", "toolCallId": "call_1", "toolName": "write_file",
            "input": {"file_path": "made.txt", "content": "x"}})),
        event(json!({"type": "tool_running", "toolCallId": "call_1", "toolName": "write_file"})),
        event(json!({"type": "tool_completed", "toolCallId": "call_1", "toolName": "write_file"})),
        event(json!({"type": "model_request_end", "model": model,
            "usage": {"inputTokens": 17639, "outputTokens": 88, "cacheReadTokens": 8192, "cacheWriteTokens": 0},
            "stopReason": "tool_calls", "effort": "max"})),
        event(json!({"type": "turn_end", "turnNumber": 1, "hadToolCalls": true,
            "usage": {"inputTokens": 17639, "outputTokens": 88, "cacheReadTokens": 8192, "cacheWriteTokens": 0}})),
        event(json!({"type": "turn_start", "turnNumber": 2})),
        event(json!({"type": "model_request_start", "model": model})),
        event(json!({"type": "text_delta", "delta": text})),
        event(json!({"type": "model_request_end", "model": model,
            "usage": {"inputTokens": 17775, "outputTokens": 3, "cacheReadTokens": 17536, "cacheWriteTokens": 0},
            "stopReason": "stop", "effort": "max"})),
        event(json!({"type": "turn_end", "turnNumber": 2, "hadToolCalls": false,
            "usage": {"inputTokens": 17775, "outputTokens": 3, "cacheReadTokens": 17536, "cacheWriteTokens": 0}})),
        event(json!({"type": "run_end", "result": {"finalText": text, "stopReason": "end_turn", "turnCount": 2,
            "usage": {"inputTokens": 35414, "outputTokens": 91, "cacheReadTokens": 25728, "cacheWriteTokens": 0}}})),
        json!({"type": "result", "subtype": "success", "sessionId": session, "stopReason": "end_turn",
            "usage": {"inputTokens": 35414, "outputTokens": 91, "cacheReadTokens": 25728, "cacheWriteTokens": 0},
            "durationMs": 4845, "finalText": text})
        .to_string(),
    ]
}

fn cc_actions(session: &str, model: &str, text: &str) -> Vec<Value> {
    let mut actions = say(&cc_stream(session, model, text));
    actions.push(json!({"exit": 0}));
    actions
}

/// The refused turn as `kompitumize-script` wrote it: credits, on the first request.
fn refusal_actions() -> Vec<Value> {
    let mut actions = say(&[
        event(json!({"type": "run_start", "sessionId": "s-credits"})),
        event(json!({"type": "turn_start", "turnNumber": 1})),
        event(json!({"type": "message_start"})),
        event(json!({"type": "model_request_start", "model": MODEL})),
        event(json!({"type": "run_error", "error": {"name": "TransportError",
            "message": "POST /alpha/generate → 400 error: You have insufficient credits to make this request."}})),
        event(json!({"type": "run_end", "result": {"finalText": "", "stopReason": "run_error", "turnCount": 1,
            "usage": {"inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheWriteTokens": 0}}})),
        json!({"type": "result", "subtype": "error", "sessionId": "s-credits",
            "usage": {"inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheWriteTokens": 0},
            "durationMs": 15357, "finalText": "",
            "error": "Error: Insufficient credits for Command Code.\nBuy extra usage (pay-as-you-go at API cost, zero markup) or upgrade: https://commandcode.ai/billing"})
            .to_string(),
    ]);
    actions.push(json!({"err": "Reasoning effort set to max for DeepSeek V4.1 Flash (latest).\nError: Insufficient credits for Command Code.\nBuy extra usage (pay-as-you-go at API cost, zero markup) or upgrade: https://commandcode.ai/billing"}));
    actions.push(json!({"exit": 1}));
    actions
}

fn weekly_refusal_actions() -> Vec<Value> {
    let text = "You've reached your weekly usage limit. Resets in 4d 22h (Sat 1:55).";
    let mut actions = say(&[
        event(json!({"type": "run_start", "sessionId": "s-weekly"})),
        event(json!({"type": "model_request_start", "model": MODEL})),
        json!({"type": "result", "subtype": "error", "sessionId": "s-weekly",
            "usage": {"inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 0, "cacheWriteTokens": 0},
            "durationMs": 10, "finalText": "", "error": text})
        .to_string(),
    ]);
    actions.push(json!({"exit": 1}));
    actions
}

/// A script with turns and, when asked, the top-level export the test worker prints.
fn script_with(s: &Session, name: &str, turns: Value, export: Option<&str>) -> PathBuf {
    let path = s.root.join(format!("{name}.json"));
    let mut body = json!({"turns": turns});
    if let Some(export) = export {
        body["export"] = json!(export);
    }
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    path
}

fn git_dir() -> PathBuf {
    let name = format!("git{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .find(|dir| dir.join(&name).is_file())
        .expect("git is on PATH")
}

fn cost_of(task: &Value) -> f64 {
    task["task"]["turns"][0]["usage"]["cost_usd"].as_f64().unwrap_or(-1.0)
}

#[cfg(windows)]
fn default_cc_windows(s: &Session, script: &Path, bin: &Path) {
    let appdata = s.root.join("appdata");
    let dist = appdata.join("npm").join("node_modules").join("command-code").join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::copy(script, dist.join("index.mjs")).unwrap();
    std::fs::copy(TEST_WORKER, bin.join("node.exe")).unwrap();
    s.env("APPDATA", &appdata);
    s.env("PATH", format!("{};{}", git_dir().display(), bin.display()));
}

#[cfg(not(windows))]
fn default_cc_unix(s: &Session, script: &Path, bin: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let path = bin.join("commandcode");
    let text =
        format!("#!/bin/sh\nexec {} {} \"$@\"\n", shell_quote(TEST_WORKER), shell_quote(&script.display().to_string()));
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    s.env("PATH", format!("{}:{}", git_dir().display(), bin.display()));
}

#[cfg(not(windows))]
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[test]
fn a_command_code_turn_gets_its_brief_on_stdin_and_ends_done() {
    let s = Session::new();
    let repo = s.repo();
    let projects = s.root.join("cc-projects");
    std::fs::create_dir_all(projects.join("proj")).unwrap();
    std::fs::write(projects.join("proj").join("s-1.jsonl"), json!({"usage": {"costUsd": 0.25}}).to_string() + "\n")
        .unwrap();
    s.env("KOMPITUM_CC_PROJECTS", &projects);
    let stdin = s.root.join("stdin.txt");
    let args = s.root.join("args.txt");
    let env = s.root.join("env.txt");
    let mut actions = vec![
        json!({"args_to": args.display().to_string()}),
        json!({"stdin_to": stdin.display().to_string()}),
        json!({"env_to": env.display().to_string()}),
    ];
    actions.extend(cc_actions("s-1", MODEL, "all done"));
    let script = s.script("cc-one", json!([actions]));
    s.config_for(&s.missing_cc(), &script, "");
    let started = s.start("cc-1", "the brief", &repo, &["--worker", "cc"]);
    assert_eq!(started["state"], "running", "{started}");
    let (exit, rows, summary) = s.wait(&["cc-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["worker"], "Command Code", "{rows:?}");

    let argv = std::fs::read_to_string(&args).unwrap();
    for flag in [
        "-p",
        "--output-format",
        "json",
        "-m",
        MODEL,
        "--effort",
        "max",
        "--yolo",
        "--skip-onboarding",
        "-t",
        "--no-auto-update",
        "--max-turns",
        "200",
        "-n",
        "kompitum-cc-1",
    ] {
        assert!(argv.lines().any(|line| line == flag), "{flag} in {argv}");
    }
    assert!(!argv.contains("the brief"), "the brief must not be on argv: {argv}");
    let prompt = std::fs::read_to_string(&stdin).unwrap();
    assert!(prompt.starts_with("the brief"), "{prompt}");
    assert!(prompt.contains("# Limits on this task (added by Kompitum)"), "{prompt}");
    assert!(!prompt.contains("You are taking over from another worker"), "no note: {prompt}");
    assert!(std::fs::read_to_string(&env).unwrap().lines().any(|line| line == "COMMANDCODE_SKIP_UPDATES=1"));

    let view = s.task("cc-1");
    let turn = &view["task"]["turns"][0];
    assert_eq!(turn["session"], "s-1", "{view}");
    assert_eq!(turn["text"], "all done", "{view}");
    assert_eq!(turn["usage"]["steps"], 2, "{view}");
    assert_eq!(turn["usage"]["input"], (17639 - 8192) + (17775 - 17536), "{view}");
    assert_eq!(turn["usage"]["output"], 91, "{view}");
    assert_eq!(turn["usage"]["largest_request"], 17775, "{view}");
    assert!((cost_of(&view) - 0.25).abs() < 1e-9, "{view}");

    let workers = s.json(&["workers"]);
    assert_eq!(workers["workers"][0]["key"], "cc", "{workers}");
    assert!((workers["workers"][0]["spend"]["five_hours"]["cost_usd"].as_f64().unwrap() - 0.25).abs() < 1e-9);
}

#[test]
fn a_missing_transcript_leaves_a_warning_and_no_made_up_cost() {
    let s = Session::new();
    let repo = s.repo();
    let projects = s.root.join("empty-projects");
    std::fs::create_dir_all(&projects).unwrap();
    s.env("KOMPITUM_CC_PROJECTS", &projects);
    let script = s.script("cc-no-cost", json!([cc_actions("s-2", MODEL, "done anyway")]));
    s.config_for(&s.missing_cc(), &script, "");
    s.start("cc-2", "brief", &repo, &["--worker", "cc"]);
    let (exit, rows, _) = s.wait(&["cc-2"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    let view = s.task("cc-2");
    assert_eq!(cost_of(&view), 0.0, "{view}");
    let warnings = view["task"]["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|warning| warning.as_str().unwrap()
            == "Command Code's transcript for session s-2 was not found, so this turn's cost is unknown"),
        "{view}"
    );
}

#[test]
fn a_credits_refusal_blocks_the_worker_and_the_next_task_goes_on_with_go() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("credits", json!([refusal_actions(), say(&stream("ses_go", "go did it"))]));
    s.config(&script, "");
    let started = s.start("cc-out", "brief", &repo, &[]);
    assert_eq!(started["worker"], "Command Code", "{started}");
    let (exit, rows, _) = s.wait(&["cc-out"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "the refused task goes on with Go: {rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["worker"], "opencode Go", "{rows:?}");
    assert_eq!(rows[0]["workers"], json!(["cc", "go"]), "{rows:?}");

    let workers = s.json(&["workers"]);
    assert_eq!(workers["next"], "go", "{workers}");
    assert_eq!(workers["workers"][0]["state"], "out", "{workers}");
    assert_eq!(workers["workers"][0]["window"], "credits", "{workers}");
    let sentence = workers["workers"][0]["sentence"].as_str().unwrap().to_string();
    assert!(sentence.contains("out of credits"), "{sentence}");
    assert!(sentence.contains("reset unknown"), "{sentence}");
    assert!(sentence.contains("next try"), "{sentence}");

    let second = s.start("auto-next", "brief", &repo, &[]);
    assert_ne!(second["state"], "failed", "{second}");
    let (exit, rows, _) = s.wait(&["auto-next"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["worker"], "opencode Go", "{rows:?}");

    // Command Code is still out, so the task waits for it to come back.
    let blocked = s.start("cc-again", "brief", &repo, &["--worker", "cc"]);
    assert_eq!(blocked["state"], "stranded", "{blocked}");
    assert_eq!(blocked["error"], format!("QUOTA EXHAUSTED: {sentence}"), "{blocked}");
}

#[test]
fn a_usage_limit_with_a_named_reset_blocks_until_that_time() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("weekly", json!([weekly_refusal_actions()]));
    s.config_for(&s.missing_cc(), &script, "");
    s.start("weekly-1", "brief", &repo, &["--worker", "cc"]);
    let (exit, rows, _) = s.wait(&["weekly-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    let workers = s.json(&["workers"]);
    let cc = &workers["workers"][0];
    assert_eq!(cc["state"], "out", "{workers}");
    assert_eq!(cc["source"], "named", "{workers}");
    assert_eq!(cc["window"], "weekly", "{workers}");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let until = cc["until_ms"].as_u64().unwrap();
    assert!(until > now + 4 * 86_400_000, "{workers}");
    assert!(until < now + 5 * 86_400_000, "{workers}");
    assert!(cc["sentence"].as_str().unwrap().contains("weekly"), "{workers}");
}

#[test]
fn a_persons_block_survives_a_core_restart_and_clear_ends_it() {
    let s = Session::new();
    let repo = s.repo();
    let go_refusal = vec![
        json!({"out": json!({"type": "error", "sessionID": "ses_go", "error": {"name": "ProviderError",
            "status": 429, "message": "reached your weekly usage limit"}}).to_string()}),
        json!({"out": stream("ses_go", "")[3]}),
        json!({"exit": 0}),
    ];
    let script = s.script("person", json!([go_refusal, cc_actions("s-cc", MODEL, "cc is back")]));
    s.config(&script, "");
    let set = s.json(&["workers", "--block", "cc", "--until", "2099-01-01", "--reason", "test"]);
    assert_eq!(set["workers"][0]["source"], "person", "{set}");
    assert_eq!(set["next"], "go", "{set}");

    let (code, _, err) = s.out(&["server", "restart"]);
    assert_eq!(code, 0, "{err}");
    let after = s.json(&["workers"]);
    assert_eq!(after["workers"][0]["state"], "out", "{after}");
    assert_eq!(after["workers"][0]["source"], "person", "{after}");
    assert_eq!(after["next"], "go", "{after}");

    s.start("go-refused", "brief", &repo, &[]);
    let (exit, rows, _) = s.wait(&["go-refused"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    let still = s.json(&["workers"]);
    assert_eq!(still["workers"][0]["source"], "person", "a refusal did not replace it: {still}");
    assert_eq!(still["workers"][0]["until_ms"], after["workers"][0]["until_ms"], "{still}");
    assert!(still["next"].is_null(), "both are out: {still}");
    assert!(still["all_out"].as_str().unwrap().starts_with("ALL WORKERS OUT:"), "{still}");

    let cleared = s.json(&["workers", "--clear", "cc"]);
    assert_eq!(cleared["workers"][0]["state"], "ready", "{cleared}");
    assert_eq!(cleared["next"], "cc", "{cleared}");
    // The stranded task goes first, then this one; both are Command Code's now.
    let back = s.start("cc-back", "brief", &repo, &[]);
    assert_ne!(back["state"], "failed", "{back}");
    let (exit, rows, _) = s.wait(&["cc-back"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["worker"], "Command Code", "{rows:?}");
}

#[test]
fn every_worker_out_strands_the_start_and_names_each_reset() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("out", json!([cc_actions("s-never", MODEL, "never")]));
    s.config(&script, "");
    s.json(&["workers", "--block", "cc", "--until", "2099-01-01", "--reason", "a"]);
    s.json(&["workers", "--block", "go", "--until", "2099-01-02", "--reason", "b"]);
    let workers = s.json(&["workers"]);
    assert!(workers["next"].is_null(), "{workers}");
    assert!(workers["all_out"].as_str().unwrap().starts_with("ALL WORKERS OUT:"), "{workers}");
    let started = s.start("nothing", "brief", &repo, &[]);
    assert_eq!(started["state"], "stranded", "{started}");
    let error = started["error"].as_str().unwrap();
    assert!(error.starts_with("ALL WORKERS OUT: "), "{error}");
    assert!(error.contains("Command Code is out by a person's word (a), back on"), "{error}");
    assert!(error.contains("opencode Go is out by a person's word (b), back on"), "{error}");
}

#[test]
fn the_default_lookup_finds_command_code() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("default", json!([cc_actions("s-default", MODEL, "found me")]));
    let projects = s.root.join("empty-projects");
    std::fs::create_dir_all(&projects).unwrap();
    s.env("KOMPITUM_CC_PROJECTS", &projects);
    let bin = s.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    #[cfg(windows)]
    default_cc_windows(&s, &script, &bin);
    #[cfg(not(windows))]
    default_cc_unix(&s, &script, &bin);
    let workers = s.json(&["workers"]);
    assert_eq!(workers["workers"][0]["state"], "ready", "{workers}");
    let started = s.start("default-1", "brief", &repo, &[]);
    assert_eq!(started["worker"], "Command Code", "{started}");
    let (exit, rows, _) = s.wait(&["default-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
}

#[test]
fn with_nothing_installed_the_default_lookup_says_so() {
    let s = Session::new();
    let repo = s.repo();
    let empty = s.root.join("empty-appdata");
    std::fs::create_dir_all(&empty).unwrap();
    s.env("APPDATA", &empty);
    s.env("PATH", git_dir().display().to_string());
    let workers = s.json(&["workers"]);
    for worker in workers["workers"].as_array().unwrap() {
        assert_eq!(worker["state"], "not_installed", "{workers}");
    }
    assert!(workers["next"].is_null(), "{workers}");
    let brief = s.brief("nothing", "brief");
    let (code, _, stderr) =
        s.out(&["task", "start", "nothing", "--brief", &brief, "--in", &repo.display().to_string()]);
    assert_eq!(code, 1, "{stderr}");
    let failed: Value = serde_json::from_str(&stderr).unwrap_or_else(|e| panic!("{e}: {stderr}"));
    assert_eq!(failed["error"]["kind"], "worker_missing", "{failed}");
    assert!(!failed["error"]["message"].as_str().unwrap().contains("ALL WORKERS OUT"), "{failed}");
}

#[test]
fn a_bare_program_name_is_found_on_the_path() {
    let s = Session::new();
    let worker = Path::new(TEST_WORKER);
    let name = worker.file_stem().unwrap().to_str().unwrap();
    s.env("PATH", worker.parent().unwrap());
    let dir = s.root.join("config").join("kompitum-core");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), format!("[workers]\nopencode = [{name:?}]\n")).unwrap();
    let workers = s.json(&["workers"]);
    let go = workers["workers"].as_array().unwrap().iter().find(|worker| worker["key"] == "go").unwrap();
    assert_eq!(go["state"], "ready", "{workers}");
}

#[test]
fn a_damaged_config_and_a_missing_program_are_said_in_words() {
    let s = Session::new();
    let dir = s.root.join("config").join("kompitum-core");
    std::fs::create_dir_all(&dir).unwrap();
    let script = s.root.join("no-opencode.exe");
    let script = serde_json::to_string(script.to_str().unwrap()).unwrap();
    std::fs::write(dir.join("config.toml"), format!("[workers]\nopencode = [{script}]\n")).unwrap();
    let workers = s.json(&["workers"]);
    let go = workers["workers"].as_array().unwrap().iter().find(|worker| worker["key"] == "go").unwrap();
    assert_eq!(go["state"], "not_installed", "{workers}");
    assert!(go["message"].as_str().unwrap().contains("no-opencode"), "{workers}");

    std::fs::write(dir.join("config.toml"), "this is = = not toml").unwrap();
    let workers = s.json(&["workers"]);
    assert!(workers["warnings"].to_string().contains("cannot read the config"), "{workers}");
}

#[test]
fn until_and_reason_are_refused_without_a_block() {
    let s = Session::new();
    for args in [["workers", "--until", "3h"], ["workers", "--reason", "why"]] {
        let (code, _, stderr) = s.out(&args);
        assert_ne!(code, 0, "{args:?} {stderr}");
    }
}

#[test]
fn an_until_that_is_not_ascii_is_refused_in_words() {
    let s = Session::new();
    for until in ["2026-11-23T12:3\u{e9}", "2026-11-23T15:36+a\u{e9}1"] {
        let (code, _, stderr) = s.out(&["workers", "--block", "cc", "--until", until]);
        assert_ne!(code, 0, "{until} {stderr}");
        assert!(stderr.contains("cannot read"), "{stderr}");
    }
    let (code, out, stderr) = s.out(&["workers"]);
    assert_eq!(code, 0, "the core still answers: {stderr} {out}");
}

#[test]
fn two_command_code_tasks_start_eight_seconds_apart() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("spacing", json!([cc_actions("s-a", MODEL, "first"), cc_actions("s-b", MODEL, "second")]));
    s.config_for(&s.missing_cc(), &script, "");
    s.start("sp-1", "one", &repo, &["--worker", "cc", "--scope", "src/**"]);
    let second = s.start("sp-2", "two", &repo, &["--worker", "cc", "--scope", "docs/**"]);
    assert_eq!(second["state"], "queued", "{second}");
    assert_eq!(second["waiting"], "waiting for a start slot: workers start 8 s apart", "{second}");
    let (exit, rows, summary) = s.wait(&["sp-1", "sp-2"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    let first = s.task("sp-1");
    let second = s.task("sp-2");
    let first_started = first["task"]["turns"][0]["started_ms"].as_u64().unwrap();
    let second_started = second["task"]["turns"][0]["started_ms"].as_u64().unwrap();
    assert!(second_started >= first_started + 8_000, "{first_started} then {second_started}");
}

#[test]
fn a_route_that_differs_fails_the_turn() {
    let s = Session::new();
    let repo = s.repo();
    let cc = s.script("cc-route", json!([cc_actions("s-route", "deepseek/another-model", "wrong route")]));
    s.config_for(&s.missing_cc(), &cc, "");
    s.start("cc-route", "brief", &repo, &["--worker", "cc"]);
    let (exit, rows, _) = s.wait(&["cc-route"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert!(rows[0]["error"].as_str().unwrap().starts_with("model verification mismatch:"), "{rows:?}");
    assert!(rows[0]["error"].as_str().unwrap().contains("deepseek/another-model"), "{rows:?}");

    let export = r#"{"messages":[{"model":{"providerID":"opencode","id":"another-model","variant":"max"}}]}"#;
    let opencode = script_with(&s, "oc-route", json!([say(&stream("ses_route", "wrong too"))]), Some(export));
    s.config_for(&opencode, &s.missing_cc(), "");
    s.start("oc-route", "brief", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["oc-route"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert!(rows[0]["error"].as_str().unwrap().starts_with("model verification mismatch:"), "{rows:?}");
}

#[test]
fn an_export_that_cannot_say_leaves_the_work_done_and_says_so() {
    let s = Session::new();
    let repo = s.repo();
    let opencode = script_with(&s, "oc-bad-export", json!([say(&stream("ses_bad_export", "done"))]), Some("not json"));
    s.config_for(&opencode, &s.missing_cc(), "");
    s.start("oc-bad-export", "brief", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["oc-bad-export"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    let warnings = rows[0]["warnings"].as_array().unwrap();
    let words = warnings
        .iter()
        .filter_map(|warning| warning.as_str())
        .find(|warning| warning.starts_with("which model ran could not be checked: "))
        .unwrap_or_else(|| panic!("no warning in {rows:?}"));
    assert!(words.contains("export was not JSON"), "{words}");
    assert!(words.contains("4 export attempts"), "{words}");
}

#[test]
fn the_model_check_starts_no_background_service() {
    // Without `--standalone` the test worker fails the export as the real CLI does.
    let s = Session::new();
    let repo = s.repo();
    let opencode = script_with(&s, "oc-export-flag", json!([say(&stream("ses_export_flag", "done"))]), None);
    s.config_for(&opencode, &s.missing_cc(), "");
    s.start("oc-export-flag", "brief", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["oc-export-flag"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    let warnings = rows[0]["warnings"].to_string();
    assert!(!warnings.contains("could not be checked"), "{warnings}");
}

#[test]
fn the_mode_survives_a_core_restart() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("mode", json!([say(&stream("ses_mode", "mode kept"))]));
    s.config(&script, "");
    let set = s.json(&["workers", "--mode", "go"]);
    assert_eq!(set["mode"], "go", "{set}");
    assert_eq!(set["next"], "go", "{set}");
    let (code, _, err) = s.out(&["server", "restart"]);
    assert_eq!(code, 0, "{err}");
    let workers = s.json(&["workers"]);
    assert_eq!(workers["mode"], "go", "{workers}");
    assert_eq!(workers["next"], "go", "{workers}");
    let started = s.start("mode-1", "brief", &repo, &[]);
    assert_eq!(started["worker"], "opencode Go", "{started}");
    let (exit, rows, _) = s.wait(&["mode-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
}

#[test]
fn a_slow_route_check_is_not_the_worker_being_idle() {
    let s = Session::new();
    let repo = s.repo();
    // The worker has ended by the time the export runs, and the idle limit is the worker's.
    let script = s.root.join("slow-export.json");
    let body = json!({"turns": [say(&stream("ses_slow", "all done"))], "export_sleep_ms": 6000});
    std::fs::write(&script, serde_json::to_vec(&body).unwrap()).unwrap();
    s.config(&script, "");
    s.start("slow-route", "brief", &repo, &["--worker", "go", "--idle", "2"]);
    let (exit, rows, _) = s.wait(&["slow-route"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
}
