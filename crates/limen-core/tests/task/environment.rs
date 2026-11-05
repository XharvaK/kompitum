//! What a worker finds around it: its environment, the commit guard, denied tools.

use super::*;

/// One value out of an `env_to` file, however Windows spells its name.
fn env_value(env: &str, key: &str) -> Option<String> {
    env.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        name.eq_ignore_ascii_case(key).then(|| value.to_string())
    })
}

#[test]
fn the_workers_environment_is_clean() {
    let s = Session::new();
    let repo = s.repo();
    // The core's own environment, as a launcher and another agent left it.
    let hidden = [
        "OLDPWD",
        "PYTHONHOME",
        "UV_INTERNAL__PYTHONHOME",
        "CLAUDE_CODE_ENTRYPOINT",
        "ANTHROPIC_API_KEY",
        "LIMEN_POLL_SECONDS",
    ];
    for key in hidden {
        s.env(key, format!("leftover-{key}"));
    }
    // The control: set the same way, and no rule takes it out.
    s.env("A_PLAIN_VARIABLE", "reaches the worker");
    let env = s.root.join("env.txt");
    let mut actions = vec![json!({"env_to": env.display().to_string()})];
    actions.extend(say(&stream("ses_clean", "looked")));
    let script = s.script("clean", json!([actions]));
    s.config(&script, "");
    let started = s.start("clean-1", "look at the environment", &repo, &["--worker", "go"]);
    assert_eq!(started["state"], "running", "{started}");
    let (exit, rows, _) = s.wait(&["clean-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");

    let text = std::fs::read_to_string(&env).unwrap();
    for key in hidden {
        assert!(env_value(&text, key).is_none(), "{key} must not reach the worker: {text}");
    }
    assert_eq!(env_value(&text, "A_PLAIN_VARIABLE").as_deref(), Some("reaches the worker"), "{text}");
    assert_eq!(env_value(&text, "LIMEN_WORKER").as_deref(), Some("1"), "the mark is the worker's own: {text}");
    assert!(
        env_value(&text, "LIMEN_SCOPE").is_some_and(|scope| !scope.is_empty()),
        "the scope travels with the turn: {text}"
    );
    // Where it found a real Python, the shims come first on the worker's PATH.
    let view = s.task("clean-1");
    let warnings = view["task"]["warnings"].as_array().unwrap();
    let no_python = warnings.iter().any(|warning| warning.as_str().unwrap().starts_with("no real Python"));
    if limen_core::platform::PYTHON_SHIMS && !no_python {
        let path = env_value(&text, "PATH").unwrap();
        let first = std::env::split_paths(&path).next().unwrap();
        assert!(first.join("python.cmd").is_file() && first.join("python").is_file(), "{path}");
    }
}

/// An `lc` run as a worker's own: the mark is on the command, everything else is the session's.
fn as_a_worker(s: &Session, args: &[&str]) -> (i32, String, String) {
    let out = s.cmd().env("LIMEN_WORKER", "1").args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_worker_cannot_start_a_task_or_change_the_workers() {
    let s = Session::new();
    let repo = s.repo();
    let brief = s.brief("refused", "do nothing");
    let folder = repo.display().to_string();
    let cases: [(Vec<&str>, &str); 13] = [
        (vec!["task", "start", "nope", "--brief", brief.as_str(), "--in", folder.as_str()], "task start"),
        (vec!["task", "cancel", "nope"], "task cancel"),
        (vec!["task", "discard", "nope"], "task discard"),
        (vec!["workers", "--mode", "go"], "workers"),
        (vec!["workers", "--block", "cc", "--until", "2d"], "workers"),
        (vec!["workers", "--clear", "cc"], "workers"),
        (vec!["server", "stop"], "server stop"),
        (vec!["server", "restart"], "server restart"),
        (vec!["kill", "t1"], "kill"),
        (vec!["send", "t1", "hi"], "send"),
        (vec!["keys", "t1", "enter"], "keys"),
        (vec!["interrupt", "t1"], "interrupt"),
        (vec!["run", "echo", "hi"], "run"),
    ];
    for (args, what) in &cases {
        let (code, out, err) = as_a_worker(&s, args);
        let said = format!("{out}{err}");
        assert_eq!(code, 1, "{args:?}: {said}");
        let error: Value = serde_json::from_str(err.trim()).unwrap_or_else(|e| panic!("{args:?}: {e}: {said}"));
        assert_eq!(error["error"]["kind"], "worker_refused", "{args:?}: {said}");
        assert_eq!(
            error["error"]["message"],
            format!("a worker may only look: lc {what} is refused here"),
            "{args:?}: {said}"
        );
        assert_eq!(error["error"]["hint"], "say what you need in your report", "{args:?}: {said}");
    }
    // What a worker may still do, and the core is still up to answer it.
    for args in [vec!["ls"], vec!["task", "ls"], vec!["workers"]] {
        let (code, out, err) = as_a_worker(&s, &args);
        assert_eq!(code, 0, "{args:?}: {out}{err}");
    }
    let list = s.json(&["task", "ls"]);
    assert!(list["tasks"].as_array().unwrap().is_empty(), "a refused start made no task: {list}");
    let status = s.json(&["server", "status"]);
    assert!(status["core"]["id"].is_string(), "the core is still up: {status}");
}

/// A `run` action's file: the code on the first line, then what the program said.
fn ran(text: &str) -> (i32, String) {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    (first.trim().trim_start_matches("exit=").parse().unwrap_or(-1), rest.to_string())
}

#[test]
fn the_guard_refuses_commit_and_push_in_the_repo_and_not_elsewhere() {
    let s = Session::new();
    let repo = s.repo();
    // A bare repository to push to, and a scratch repository of the worker's own to commit in.
    git(&s.root, &["init", "-q", "--bare", "elsewhere.git"]);
    git(&s.root, &["init", "-q", "scratch"]);
    let bare = s.root.join("elsewhere.git").display().to_string();
    let scratch = s.root.join("scratch").display().to_string();
    let commit_file = s.root.join("commit.txt").display().to_string();
    let push_file = s.root.join("push.txt").display().to_string();
    let scratch_file = s.root.join("scratch.txt").display().to_string();
    let identity = ["-c", "user.name=t", "-c", "user.email=t@t"];

    let mut commit = vec![commit_file.clone(), "git".to_string()];
    commit.extend(identity.map(String::from));
    commit.extend(["commit", "--allow-empty", "-m", "x"].map(String::from));
    let push = vec![push_file.clone(), "git".to_string(), "push".to_string(), bare, "HEAD:refs/heads/x".to_string()];
    let mut scratch_commit = vec![scratch_file.clone(), "git".to_string(), "-C".to_string(), scratch];
    scratch_commit.extend(identity.map(String::from));
    scratch_commit.extend(["commit", "--allow-empty", "-m", "x"].map(String::from));

    let mut actions = vec![json!({"run": commit}), json!({"run": push}), json!({"run": scratch_commit})];
    actions.extend(say(&stream("ses_guard", "tried")));
    let script = s.script("guard", json!([actions]));
    s.config(&script, "");
    let started = s.start("guard-1", "try to commit", &repo, &["--worker", "go"]);
    assert_eq!(started["state"], "running", "{started}");
    let (exit, rows, _) = s.wait(&["guard-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");

    let (code, said) = ran(&std::fs::read_to_string(&commit_file).unwrap());
    assert_ne!(code, 0, "a commit in the worktree is refused: {said}");
    assert!(said.contains("lc: git commit/push is blocked for workers"), "{said}");
    let (code, said) = ran(&std::fs::read_to_string(&push_file).unwrap());
    assert_ne!(code, 0, "a push from the worktree is refused: {said}");
    assert!(said.contains("lc: git commit/push is blocked for workers"), "{said}");
    let (code, said) = ran(&std::fs::read_to_string(&scratch_file).unwrap());
    assert_eq!(code, 0, "a scratch repository commits normally: {said}");
}

/// A Command Code turn's three lines: its session, the model it used and its last words.
fn cc_says(session: &str, text: &str) -> Vec<String> {
    let model = "deepseek/deepseek-v4.1-flash";
    vec![
        json!({"type": "event", "event": {"type": "run_start", "sessionId": session}}).to_string(),
        json!({"type": "event", "event": {"type": "model_request_start", "model": model}}).to_string(),
        json!({"type": "result", "subtype": "success", "sessionId": session, "finalText": text}).to_string(),
    ]
}

/// Command Code's deny list, as `CC_DENY` has it.
fn cc_deny() -> Vec<&'static str> {
    vec![
        "Read(.env*)",
        "Read(*.env)",
        "Read(*.pem)",
        "Read(~/.ssh/**)",
        "Read(auth.json)",
        "Read(~/.commandcode/**)",
        "Read(~/.local/share/opencode/**)",
        "Edit(.env*)",
        "Edit(*.pem)",
        "Shell(*.env*)",
        "Shell(*.pem*)",
        "Shell(*.ssh*)",
        "Shell(*auth.json*)",
        "Shell(*git commit*)",
        "Shell(*git push*)",
    ]
}

#[test]
fn command_codes_settings_are_there_for_the_turn_and_gone_after() {
    let s = Session::new();
    let repo = s.repo();
    let copy = s.root.join("during-the-turn.json").display().to_string();
    let mut actions = vec![json!({"copy": [".commandcode/settings.local.json", copy]})];
    actions.extend(say(&cc_says("s-settings", "looked")));
    let script = s.script("cc-settings", json!([actions.clone(), actions]));
    s.config_for(&s.missing_cc(), &script, "");
    let started = s.start("cc-settings-1", "look at the settings", &repo, &["--worker", "cc"]);
    assert_eq!(started["state"], "running", "{started}");
    let (exit, rows, _) = s.wait(&["cc-settings-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");

    let settings: Value = serde_json::from_slice(&std::fs::read(&copy).unwrap()).unwrap();
    let deny: Vec<&str> =
        settings["permissions"]["deny"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(deny, cc_deny(), "{settings}");
    assert_eq!(settings["tasteLearning"], false, "{settings}");
    assert_eq!(settings["attribution"]["commit"], "", "{settings}");

    let view = s.task("cc-settings-1");
    let worktree = PathBuf::from(view["task"]["worktree"].as_str().unwrap());
    assert!(!worktree.join(".commandcode").exists(), "the settings are gone after the turn");
    assert_eq!(view["task"]["changed"], json!([]), "and never counted as a change: {view}");

    // A settings file of the repository's own comes back byte for byte.
    git(&repo, &["config", "core.autocrlf", "false"]);
    let original = "{\"kept\": true}\n";
    let folder = repo.join(".commandcode");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("settings.local.json"), original).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "settings"]);
    let copy = s.root.join("during-the-second-turn.json").display().to_string();
    let mut actions = vec![json!({"copy": [".commandcode/settings.local.json", copy]})];
    actions.extend(say(&cc_says("s-settings-2", "looked again")));
    let script = s.script("cc-settings-2", json!([actions]));
    s.config_for(&s.missing_cc(), &script, "");
    s.start("cc-settings-2", "look again", &repo, &["--worker", "cc"]);
    let (exit, rows, _) = s.wait(&["cc-settings-2"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    let during: Value = serde_json::from_slice(&std::fs::read(&copy).unwrap()).unwrap();
    assert_eq!(during["permissions"]["deny"].as_array().unwrap().len(), cc_deny().len(), "{during}");
    let view = s.task("cc-settings-2");
    let worktree = PathBuf::from(view["task"]["worktree"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(worktree.join(".commandcode").join("settings.local.json")).unwrap(),
        original,
        "its own bytes are back"
    );
    assert_eq!(view["task"]["changed"], json!([]), "and never counted as a change: {view}");
}

#[test]
fn opencode_gets_its_denied_tools() {
    let s = Session::new();
    let repo = s.repo();
    let env = s.root.join("env.txt");
    let mut actions = vec![json!({"env_to": env.display().to_string()})];
    actions.extend(say(&stream("ses_tools", "looked")));
    let script = s.script("tools", json!([actions]));
    s.config(&script, "");
    s.start("tools-1", "look at the tools", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["tools-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");

    let text = std::fs::read_to_string(&env).unwrap();
    let content = env_value(&text, "OPENCODE_CONFIG_CONTENT").expect("the config reaches the worker");
    let config: Value = serde_json::from_str(&content).unwrap_or_else(|e| panic!("{e}: {content}"));
    let secret = ["*.env", "*.env.*", ".env", "*.pem", "*/.ssh/*", "*.ssh*", "*auth.json*"];
    let deep = ["**/.env", "**/.env.*", "**/*.pem", "**/.ssh/**", "**/auth.json"];
    let mut read = serde_json::Map::new();
    read.insert("*".into(), json!("allow"));
    for glob in secret.iter().chain(deep.iter()) {
        read.insert((*glob).to_string(), json!("deny"));
    }
    assert_eq!(config["permission"]["read"], Value::Object(read), "{config}");
    let bash_deny = [
        "git push*",
        "*git push*",
        "git commit*",
        "*git commit*",
        "*git -c*commit*",
        "*.env*",
        "*.pem*",
        "*.ssh*",
        "*auth.json*",
    ];
    let mut bash = serde_json::Map::new();
    bash.insert("*".into(), json!("allow"));
    for glob in bash_deny {
        bash.insert(glob.to_string(), json!("deny"));
    }
    assert_eq!(config["permission"]["bash"], Value::Object(bash), "{config}");
    assert_eq!(
        config["permission"]["edit"],
        json!({"*": "allow", "*.env": "deny", "**/.env": "deny", "*.pem": "deny"}),
        "{config}"
    );
}
