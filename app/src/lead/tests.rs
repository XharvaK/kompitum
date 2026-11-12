use super::*;

fn catalog(session: Value, flags: Option<Value>) -> CatalogEntry {
    serde_json::from_value(json!({
        "run": ["prog", "--rest"],
        "flags": flags,
        "session": session,
    }))
    .unwrap()
}

#[test]
fn the_argv_puts_the_session_right_after_the_program() {
    let entry = catalog(
        json!({
            "new": ["--session-id", "{id}"],
            "resume": ["--resume", "{id}"],
            "continue": ["resume", "--last"],
        }),
        Some(json!({"model": ["-m", "{model}#{effort}"], "effort": []})),
    );
    assert_eq!(
        argv(&entry, &Choice::New { id: "abc".into() }, None, None),
        ["prog", "--session-id", "abc", "--rest"]
    );
    assert_eq!(
        argv(&entry, &Choice::Resume { id: "abc".into() }, None, None),
        ["prog", "--resume", "abc", "--rest"]
    );
    assert_eq!(
        argv(&entry, &Choice::Continue, None, None),
        ["prog", "resume", "--last", "--rest"]
    );
    assert_eq!(
        argv(
            &entry,
            &Choice::Continue,
            Some("opus"),
            Some("high")
        ),
        ["prog", "resume", "--last", "--rest", "-m", "opus#high"]
    );
    assert_eq!(argv(&entry, &Choice::Plain, None, None), ["prog", "--rest"]);
    assert_eq!(
        argv(
            &entry,
            &Choice::Resume { id: "abc".into() },
            Some("opus"),
            Some("high")
        ),
        ["prog", "--resume", "abc", "--rest", "-m", "opus#high"]
    );
    assert_eq!(
        argv(&entry, &Choice::Plain, Some("opus"), None),
        ["prog", "--rest", "-m", "opus#"]
    );
    let both = catalog(
        json!({"new": ["--session-id", "{id}"]}),
        Some(json!({"model": ["--model", "{model}"], "effort": ["--effort", "{effort}"]})),
    );
    assert_eq!(
        argv(
            &both,
            &Choice::New { id: "one".into() },
            Some("opus"),
            Some("high")
        ),
        [
            "prog",
            "--session-id",
            "one",
            "--rest",
            "--model",
            "opus",
            "--effort",
            "high"
        ]
    );
    assert_eq!(
        argv(&both, &Choice::New { id: "one".into() }, None, Some("high")),
        ["prog", "--session-id", "one", "--rest", "--effort", "high"]
    );
}

#[test]
fn the_choice_follows_the_record_and_the_folder() {
    let entry = catalog(
        json!({
            "new": ["--session-id", "{id}"],
            "resume": ["--resume", "{id}"],
            "continue": ["resume", "--last"],
        }),
        None,
    );
    let stream = Stream {
        folder: "/home/me".into(),
        session: Some("old".into()),
        transcript: None,
    };
    assert_eq!(
        choose(&entry, Some(&stream), "/home/me", "fresh"),
        Choice::Resume { id: "old".into() }
    );
    assert_eq!(
        choose(&entry, Some(&stream), "/home/projects", "fresh"),
        Choice::New { id: "fresh".into() }
    );
    // Another program's record is not this one's.
    assert_eq!(
        choose(&entry, None, "/home/me", "fresh"),
        Choice::New { id: "fresh".into() }
    );
    let records = json!({
        "claude": {"folder": "/home/me", "session": "old"},
        "codex": {"folder": "/home/me"},
    });
    assert_eq!(
        stream_of(&records, "claude"),
        Some(Stream {
            folder: "/home/me".into(),
            session: Some("old".into()),
            transcript: None,
        })
    );
    assert_eq!(
        stream_of(&records, "codex"),
        Some(Stream {
            folder: "/home/me".into(),
            session: None,
            transcript: None,
        })
    );
    assert_eq!(stream_of(&records, "grok"), None);
    let by_folder = catalog(json!({"continue": ["resume", "--last"]}), None);
    assert_eq!(
        choose(&by_folder, Some(&stream), "/home/me", "fresh"),
        Choice::Continue
    );
    assert_eq!(choose(&by_folder, None, "/home/me", "fresh"), Choice::Plain);
    let plain = catalog(Value::Null, None);
    assert_eq!(
        choose(&plain, Some(&stream), "/home/me", "fresh"),
        Choice::Plain
    );
    assert_eq!(
        stream_after_choice(&Choice::New { id: "new".into() }, "/home/me", Some(&stream)),
        Stream {
            folder: "/home/me".into(),
            session: Some("new".into()),
            transcript: None,
        }
    );
    assert_eq!(
        stream_after_choice(&Choice::Plain, "/home/me", Some(&stream)),
        Stream {
            folder: "/home/me".into(),
            session: None,
            transcript: None,
        }
    );
    assert_eq!(
        stream_after_choice(&Choice::Continue, "/home/me", Some(&stream)),
        stream
    );
}

#[test]
fn a_fresh_id_is_a_lowercase_version_4_uuid() {
    let id = fresh_id(1_700_000_000_000_000_000, 4242, "/home/me");
    assert_eq!(id.len(), 36);
    let fields: Vec<&str> = id.split('-').collect();
    assert_eq!(
        fields.iter().map(|f| f.len()).collect::<Vec<_>>(),
        [8, 4, 4, 4, 12]
    );
    assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    assert!(id.chars().all(|c| !c.is_ascii_uppercase()));
    assert_eq!(&id[14..15], "4");
    assert!(["8", "9", "a", "b"].contains(&&id[19..20]));
    assert_ne!(id, fresh_id(1_700_000_000_000_000_001, 4242, "/home/me"));
    assert_ne!(id, fresh_id(1_700_000_000_000_000_000, 4242, "/home/you"));
}

#[test]
fn a_labelled_shell_is_not_the_lead() {
    let lead = json!({
        "state": "live",
        "labels": {"lead": "claude"},
    });
    assert!(is_lead_record(&lead, "claude"));
    assert!(!is_lead_record(&lead, "codex"));
    let ended = json!({
        "state": "exited",
        "labels": {"lead": "claude"},
    });
    assert!(
        !is_lead_record(&ended, "claude"),
        "an ended terminal is not the lead"
    );
    let lost = json!({
        "state": "live",
        "labels": {"lead": "shell"},
    });
    assert!(!is_lead_record(&lost, "claude"));
    let unlabelled = json!({"state": "live", "labels": {}});
    assert!(!is_lead_record(&unlabelled, "claude"));
    let old_label = json!({"state": "live", "labels": {"lead": "1"}});
    assert!(!is_lead_record(&old_label, "claude"));
    let in_its_wrapper = json!({
        "state": "live",
        "labels": {"lead": "claude"},
        "program": {"key": "shell", "shell": true},
    });
    assert!(
        is_lead_record(&in_its_wrapper, "claude"),
        "on Windows the core sees the lead's wrapper shell"
    );
}

#[test]
fn a_failed_resume_is_retried_once() {
    let failed = json!({
        "reason": "exited", "exit_code": 7, "created_ms": 1000, "finished_ms": 9000
    });
    assert!(retry_needed(&failed, false));
    assert!(!retry_needed(&failed, true), "only one retry");
    let mut late = failed.clone();
    late["finished_ms"] = json!(12_000);
    assert!(!retry_needed(&late, false));
    let mut fine = failed.clone();
    fine["exit_code"] = json!(0);
    assert!(!retry_needed(&fine, false));
    let mut killed = failed.clone();
    killed["reason"] = json!("killed");
    assert!(!retry_needed(&killed, false));
    assert!(!retry_needed(&json!({}), false));
}

#[test]
fn claude_settings_carry_its_hooks_and_memory_off() {
    let value = claude_settings(
        Path::new("/apps/Kompitum/kompitum-app.exe"),
        Path::new("/apps/kom/kom.exe"),
        Path::new("/home/kompitum"),
    );
    assert_eq!(value["autoMemoryEnabled"], json!(false));
    let entries = value["hooks"]["SessionStart"]
        .as_array()
        .expect("SessionStart is an array");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].get("matcher").is_none(), "no matcher");
    let hooks = entries[0]["hooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 2);
    assert_eq!(
        hooks[0],
        json!({
            "type": "command",
            "command": "/apps/Kompitum/kompitum-app.exe",
            "args": ["hook", "SessionStart", "/home/kompitum"],
            "timeout": 10,
        })
    );
    assert_eq!(
        hooks[1],
        json!({
            "type": "command",
            "command": "/apps/kom/kom.exe",
            "args": ["vault", "memory", "--hook"],
            "timeout": 30,
        })
    );
    let command = |event: &str| {
        json!({
            "type": "command",
            "command": "/apps/Kompitum/kompitum-app.exe",
            "args": ["hook", event, "/home/kompitum"],
        })
    };
    let first = |event: &str| {
        let list = value["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("{event} is an array"));
        list[0].clone()
    };
    let permission = first("PermissionRequest");
    assert!(permission.get("matcher").is_none());
    let mut hook = command("PermissionRequest");
    hook["timeout"] = json!(3600);
    assert_eq!(permission["hooks"], json!([hook]));

    let question = first("PreToolUse");
    assert_eq!(question["matcher"], json!("AskUserQuestion"));
    let mut hook = command("PreToolUse");
    hook["timeout"] = json!(3600);
    assert_eq!(question["hooks"], json!([hook]));

    for event in ["UserPromptSubmit", "Stop"] {
        let entry = first(event);
        assert!(entry.get("matcher").is_none(), "{event}");
        let mut hook = command(event);
        hook["async"] = json!(true);
        assert_eq!(entry["hooks"], json!([hook]), "{event}");
    }
    let idle = value["hooks"]["Notification"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry.get("matcher") == Some(&json!("idle_prompt")))
        .expect("idle_prompt");
    let mut hook = command("Notification");
    hook["async"] = json!(true);
    assert_eq!(idle["hooks"], json!([hook]));

    assert_eq!(value["modelSettings"], model_settings());
    for model in [
        "claude-fable-5-1",
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
    ] {
        assert_eq!(
            value["modelSettings"][model],
            json!({"autoCompactWindow": AUTO_COMPACT_WINDOW})
        );
    }
    assert_eq!(
        value["modelSettings"]
            .as_object()
            .map(|m| m.len())
            .unwrap_or_default(),
        4
    );
}

#[test]
fn claude_settings_install_the_chat_view_hooks() {
    let value = claude_settings(
        Path::new("/apps/Kompitum/kompitum-app.exe"),
        Path::new("/apps/kom/kom.exe"),
        Path::new("/home/kompitum"),
    );
    let command = |event: &str| {
        json!({
            "type": "command",
            "command": "/apps/Kompitum/kompitum-app.exe",
            "args": ["hook", event, "/home/kompitum"],
        })
    };
    let async_command = |event: &str| {
        let mut hook = command(event);
        hook["async"] = json!(true);
        hook
    };
    let entries = |event: &str| {
        value["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("{event} is an array"))
            .clone()
    };

    let display = &entries("MessageDisplay");
    assert_eq!(display.len(), 1, "{display:?}");
    assert!(display[0].get("matcher").is_none());
    let mut hook = async_command("MessageDisplay");
    hook["timeout"] = json!(10);
    assert_eq!(display[0]["hooks"], json!([hook]));

    let pre = entries("PreToolUse");
    assert_eq!(pre.len(), 2, "AskUserQuestion stays, and every tool is hooked: {pre:?}");
    assert_eq!(pre[0]["matcher"], json!("AskUserQuestion"));
    let mut ask = command("PreToolUse");
    ask["timeout"] = json!(3600);
    assert_eq!(pre[0]["hooks"], json!([ask]));
    assert!(pre[1].get("matcher").is_none(), "every tool, no matcher");
    assert_eq!(pre[1]["hooks"], json!([async_command("PreToolUse")]));

    let post = entries("PostToolUse");
    assert_eq!(post.len(), 1);
    assert!(post[0].get("matcher").is_none());
    assert_eq!(post[0]["hooks"], json!([async_command("PostToolUse")]));

    let failed = entries("PostToolUseFailure");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].get("matcher").is_none(), "every tool, no matcher");
    assert_eq!(failed[0]["hooks"], json!([async_command("PostToolUseFailure")]));

    for event in ["UserPromptSubmit", "Stop", "StopFailure", "Elicitation"] {
        let list = entries(event);
        assert_eq!(list.len(), 1, "{event}");
        assert!(list[0].get("matcher").is_none(), "{event}");
        assert_eq!(list[0]["hooks"], json!([async_command(event)]), "{event}");
    }

    let notes = entries("Notification");
    let matchers: Vec<_> = notes
        .iter()
        .map(|entry| entry.get("matcher").and_then(Value::as_str).unwrap_or(""))
        .collect();
    for matcher in [
        "idle_prompt",
        "elicitation_dialog",
        "elicitation_url_dialog",
        "agent_needs_input",
        "quota_auto_resume_stale",
        "auth_success",
        "auth_storage_failure",
    ] {
        assert!(
            matchers.contains(&matcher),
            "missing Notification matcher {matcher}: {matchers:?}"
        );
    }
    for entry in &notes {
        assert_eq!(entry["hooks"], json!([async_command("Notification")]), "{entry}");
    }
}

#[test]
fn setup_builds_the_two_claude_mcp_argvs() {
    let (get, add) = mcp_argvs(Path::new("/apps/kom"));
    assert_eq!(get, ["mcp", "get", "kompitum"]);
    assert_eq!(
        add,
        [
            "mcp",
            "add",
            "--scope",
            "user",
            "kompitum",
            "--",
            "/apps/kom",
            "mcp"
        ]
    );
}

#[test]
fn the_skill_is_installed_only_when_absent() {
    let home = std::env::temp_dir().join(format!(
        "kompitum-lead-skill-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    let path = home.join(".claude/skills/kompitum-core/SKILL.md");
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(install_skill(&home, SKILL), Ok("installed"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
    std::fs::write(&path, "the person's own").unwrap();
    assert_eq!(install_skill(&home, SKILL), Ok("present"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "the person's own");
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn the_app_claude_settings_hold_only_the_compaction_window() {
    let dir = std::env::temp_dir().join(format!(
        "kompitum-app-settings-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = write_app_claude_settings(&dir).unwrap();
    assert_eq!(path, dir.join("claude-settings.json"));
    let text = std::fs::read_to_string(&path).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value, json!({ "modelSettings": model_settings() }));
    assert_eq!(value.as_object().map(|m| m.len()), Some(1));
    let _ = std::fs::remove_dir_all(&dir);

    let not_a_folder = std::env::temp_dir().join(format!(
        "kompitum-app-settings-file-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    std::fs::write(&not_a_folder, "not a folder").unwrap();
    let error = write_app_claude_settings(&not_a_folder).unwrap_err();
    assert!(
        error.starts_with("Couldn't write Claude Code's settings file "),
        "{error}"
    );
    assert!(
        error.contains(&not_a_folder.join("claude-settings.json").display().to_string()),
        "{error}"
    );
    let _ = std::fs::remove_file(&not_a_folder);
}

#[test]
fn a_local_claude_start_gets_the_app_settings_file() {
    let file = Path::new("/apps/Kompitum/claude-settings.json");
    let gets = |argv: &[&str], on: Option<&str>| {
        with_claude_settings(
            argv.iter().map(|s| s.to_string()).collect(),
            on,
            file,
        )
    };
    assert_eq!(
        gets(&["claude"], None),
        ["claude", "--settings", "/apps/Kompitum/claude-settings.json"]
    );
    assert_eq!(
        gets(&["claude", "--resume", "abc"], None),
        [
            "claude",
            "--settings",
            "/apps/Kompitum/claude-settings.json",
            "--resume",
            "abc"
        ]
    );
    assert_eq!(
        gets(&["claude", "--settings", "/mine.json"], None),
        ["claude", "--settings", "/mine.json"],
        "its own --settings stays"
    );
    assert_eq!(
        gets(&["claude"], Some("vps")),
        ["claude"],
        "another machine keeps its own settings"
    );
    assert_eq!(
        gets(&["codex"], None),
        ["codex"],
        "another program stays as it is"
    );
    assert_eq!(
        gets(&[r"C:\Users\me\AppData\Roaming\npm\claude.exe"], None),
        [
            r"C:\Users\me\AppData\Roaming\npm\claude.exe",
            "--settings",
            "/apps/Kompitum/claude-settings.json"
        ]
    );
    assert_eq!(
        gets(&["CLAUDE.CMD"], None),
        ["CLAUDE.CMD", "--settings", "/apps/Kompitum/claude-settings.json"]
    );
    assert_eq!(gets(&[], None), Vec::<String>::new());
    assert_eq!(gets(&["notclaude"], None), ["notclaude"]);
}

#[test]
fn a_session_start_from_the_lead_records_its_id_and_folder() {
    let mut streams = json!({"claude": {"folder": "/home/me", "session": "old"}});
    assert!(record_session_start(
        &mut streams,
        true,
        true,
        "SessionStart",
        Some("new-session"),
        "/home/me",
        None
    ));
    assert_eq!(
        streams,
        json!({"claude": {"folder": "/home/me", "session": "new-session"}})
    );
    assert!(!record_session_start(
        &mut streams,
        true,
        true,
        "SessionStart",
        Some("new-session"),
        "/home/me",
        None
    ));
    let mut fresh = json!({});
    assert!(record_session_start(
        &mut fresh,
        true,
        true,
        "SessionStart",
        Some("one"),
        "/home/me",
        None
    ));
    assert_eq!(
        fresh,
        json!({"claude": {"folder": "/home/me", "session": "one"}}),
        "a missing record is written with its folder"
    );
    assert!(record_session_start(
        &mut fresh,
        true,
        true,
        "SessionStart",
        Some("two"),
        "/home/work",
        None
    ));
    assert_eq!(
        fresh,
        json!({"claude": {"folder": "/home/work", "session": "two"}})
    );
    assert!(!record_session_start(
        &mut fresh,
        false,
        true,
        "SessionStart",
        Some("three"),
        "/home/me",
        None
    ));
    assert!(!record_session_start(
        &mut fresh,
        true,
        false,
        "SessionStart",
        Some("three"),
        "/home/me",
        None
    ));
    assert!(!record_session_start(
        &mut fresh,
        true,
        true,
        "Stop",
        Some("three"),
        "/home/me",
        None
    ));
    assert_eq!(
        fresh,
        json!({"claude": {"folder": "/home/work", "session": "two"}})
    );
}

fn ask(kind: AskKind, input: Value) -> Ask {
    Ask { id: "a1".into(), terminal: "t1".into(), kind, input }
}

fn permission_input() -> Value {
    json!({
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test"},
        "permission_suggestions": [
            {"type": "addRules", "destination": "session", "rules": [{"toolName": "Bash"}]}
        ],
    })
}

#[test]
fn a_blocking_event_becomes_the_right_kind_of_ask() {
    assert_eq!(
        ask_kind("PermissionRequest", &json!({"tool_name": "ExitPlanMode", "tool_input": {}})),
        Some(AskKind::Plan)
    );
    assert_eq!(
        ask_kind("PermissionRequest", &json!({"tool_name": "Bash", "tool_input": {}})),
        Some(AskKind::Permission)
    );
    assert_eq!(
        ask_kind("PreToolUse", &json!({"tool_name": "AskUserQuestion", "tool_input": {}})),
        Some(AskKind::Question)
    );
    assert_eq!(
        ask_kind("PreToolUse", &json!({"tool_name": "Bash"})),
        None,
        "a regular tool is drawn in the conversation; it is not a permission card"
    );
    assert_eq!(ask_kind("Stop", &json!({})), None);
    assert_eq!(ask_kind("UserPromptSubmit", &json!({})), None);
}

#[test]
fn published_session_gives_conversation_meta_not_the_items() {
    let mut session = Session::default();
    session.state = Some(State::Working);
    session.conversation = fold(
        session.conversation,
        &Event::UserPromptSubmit {
            prompt: "Hello".into(),
        },
    );
    session.conversation = fold(
        session.conversation,
        &Event::MessageDisplay {
            message_id: "m1".into(),
            index: 0,
            last: true,
            delta: "Hi.".into(),
        },
    );
    let value = published_session(&session);
    assert_eq!(value["state"], json!("working"));
    assert_eq!(value["terminal"], json!("hidden"));
    assert_eq!(value["reveal"], Value::Null);
    assert_eq!(value["started"], json!(false));
    assert_eq!(value["conversation"]["length"], json!(2));
    assert!(value["conversation"]["version"].as_u64().unwrap() > 0);
    assert!(value.get("items").is_none(), "{value}");
    assert!(value.get("conversation").and_then(|c| c.get("items")).is_none(), "{value}");
}

#[test]
fn a_message_display_hook_grows_the_assistant_item() {
    let mut session = Session::default();
    assert!(apply_hook_event(
        &mut session,
        "MessageDisplay",
        &json!({"message_id": "m1", "index": 0, "final": false, "delta": "Hi"})
    ));
    assert!(apply_hook_event(
        &mut session,
        "MessageDisplay",
        &json!({"message_id": "m1", "index": 1, "final": true, "delta": " there."})
    ));
    assert_eq!(session.conversation.items.len(), 1);
    match &session.conversation.items[0] {
        Item::Assistant { text, done, .. } => {
            assert_eq!(text, "Hi there.");
            assert!(*done, "the final batch ends the message");
        }
        other => panic!("{other:?}"),
    }
    assert!(apply_hook_event(&mut session, "Stop", &json!({})));
    match &session.conversation.items[0] {
        Item::Assistant { done, .. } => assert!(*done),
        other => panic!("{other:?}"),
    }
}

#[test]
fn permission_and_question_hooks_do_not_fold() {
    let mut session = Session::default();
    assert!(!apply_hook_event(
        &mut session,
        "PermissionRequest",
        &json!({"tool_name": "Bash"})
    ));
    assert!(!apply_hook_event(
        &mut session,
        "PreToolUse",
        &json!({"tool_name": "AskUserQuestion"})
    ));
    assert!(session.conversation.items.is_empty());
}

#[test]
fn a_session_start_records_its_transcript_path_beside_the_session() {
    let mut streams = json!({});
    assert!(record_session_start(
        &mut streams,
        true,
        true,
        "SessionStart",
        Some("s1"),
        "/home/me",
        Some("/home/me/.claude/projects/s1.jsonl"),
    ));
    assert_eq!(
        stream_of(&streams, "claude"),
        Some(Stream {
            folder: "/home/me".into(),
            session: Some("s1".into()),
            transcript: Some("/home/me/.claude/projects/s1.jsonl".into()),
        })
    );
    assert!(
        !record_session_start(
            &mut streams,
            true,
            true,
            "SessionStart",
            Some("s1"),
            "/home/me",
            Some("/home/me/.claude/projects/s1.jsonl"),
        ),
        "the same session, folder and transcript change nothing"
    );
    assert!(
        record_session_start(
            &mut streams,
            true,
            true,
            "SessionStart",
            Some("s1"),
            "/home/me",
            Some("/home/me/.claude/projects/moved.jsonl"),
        ),
        "a new transcript path is recorded"
    );
    assert_eq!(
        stream_of(&streams, "claude").and_then(|stream| stream.transcript).as_deref(),
        Some("/home/me/.claude/projects/moved.jsonl")
    );
}

#[test]
fn a_transcript_file_loads_and_only_an_unreadable_one_is_a_notice() {
    let dir = std::env::temp_dir().join(format!(
        "kompitum-transcript-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("session.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"user","message":{"role":"user","content":"Hello"}}
{"type":"file-history-snapshot"}
"#,
    )
    .unwrap();
    let mut session = Session::default();
    let skipped = apply_transcript_path(&mut session, Some(path.to_str().unwrap()));
    assert_eq!(skipped, 1);
    assert_eq!(session.conversation.items.len(), 1);
    match &session.conversation.items[0] {
        Item::Owner { text, .. } => assert_eq!(text, "Hello"),
        other => panic!("{other:?}"),
    }
    // A transcript the session has not written yet is no history at all: no notice.
    let mut missing = Session::default();
    assert_eq!(
        apply_transcript_path(&mut missing, Some(dir.join("nope.jsonl").to_str().unwrap())),
        0
    );
    assert!(missing.conversation.items.is_empty(), "{:?}", missing.conversation.items);
    // A path that is there but can't be read as a file is the one notice.
    let mut unreadable = Session::default();
    assert_eq!(apply_transcript_path(&mut unreadable, Some(dir.to_str().unwrap())), 0);
    match &unreadable.conversation.items[0] {
        Item::Notice { text, .. } => assert_eq!(text, "Couldn't read the conversation history."),
        other => panic!("{other:?}"),
    }
    assert_eq!(apply_transcript_path(&mut Session::default(), None), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_nonzero_auth_status_sets_a_login_reveal_on_the_session() {
    let mut conversation = Conversation::default();
    apply_auth_status(&mut conversation, 1);
    assert_eq!(conversation.terminal, TerminalVis::Shown);
    assert_eq!(conversation.reveal, Some(Reveal::Login));
    assert_eq!(
        conversation.reveal_why.as_deref(),
        Some("Claude needs you to sign in.")
    );
    apply_auth_status(&mut conversation, 0);
    assert_eq!(conversation.terminal, TerminalVis::Hidden);
    assert_eq!(conversation.reveal, None);
    assert_eq!(conversation.reveal_why, None);
}

#[test]
fn a_permission_allow_answers_the_hook() {
    let ask = ask(AskKind::Permission, permission_input());
    assert_eq!(
        hook_output(&ask, &Answer::Allow),
        Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "allow"},
            },
        }))
    );
}

#[test]
fn allow_always_carries_the_permission_suggestions_unchanged() {
    let input = permission_input();
    let ask = ask(AskKind::Permission, input.clone());
    assert_eq!(
        hook_output(&ask, &Answer::AllowAlways),
        Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "allow",
                    "updatedPermissions": input["permission_suggestions"],
                },
            },
        }))
    );
}

#[test]
fn a_permission_deny_carries_its_note_or_the_default() {
    let ask = ask(AskKind::Permission, permission_input());
    let deny = |note: Option<&str>| {
        Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "deny",
                    "message": note.unwrap_or("The Owner declined."),
                },
            },
        }))
    };
    assert_eq!(
        hook_output(&ask, &Answer::Deny { note: Some("Not that file.".into()) }),
        deny(Some("Not that file."))
    );
    assert_eq!(hook_output(&ask, &Answer::Deny { note: None }), deny(None));
    assert_eq!(
        hook_output(&ask, &Answer::Deny { note: Some("   ".into()) }),
        deny(None),
        "a blank note is no note"
    );
}

#[test]
fn a_plan_approval_returns_its_tool_input_unchanged() {
    let input = json!({
        "tool_name": "ExitPlanMode",
        "tool_input": {"plan": "Do the thing.", "steps": ["one", "two"]},
    });
    let ask = ask(AskKind::Plan, input.clone());
    assert_eq!(
        hook_output(&ask, &Answer::Approve),
        Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "allow",
                    "updatedInput": input["tool_input"],
                },
            },
        }))
    );
}

#[test]
fn keep_planning_carries_its_note_as_the_message() {
    let input = json!({"tool_name": "ExitPlanMode", "tool_input": {"plan": "x"}});
    let ask = ask(AskKind::Plan, input);
    let keep = |note: &str| {
        Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "deny", "message": note},
            },
        }))
    };
    assert_eq!(
        hook_output(&ask, &Answer::KeepPlanning { note: Some("Check the tests first.".into()) }),
        keep("Check the tests first.")
    );
    assert_eq!(hook_output(&ask, &Answer::KeepPlanning { note: None }), keep(""));
}

#[test]
fn answers_to_a_question_are_joined_and_returned_with_it() {
    let questions = json!([
        {
            "question": "Which database?",
            "header": "Database",
            "multiSelect": false,
            "options": [{"label": "Postgres", "description": "The server's"}, {"label": "SQLite", "description": "A file"}],
        },
        {
            "question": "Which checks?",
            "header": "Checks",
            "multiSelect": true,
            "options": [{"label": "Tests", "description": "Run them"}, {"label": "Lint", "description": "Keep it tidy"}],
        },
    ]);
    let input = json!({"tool_name": "AskUserQuestion", "tool_input": {"questions": questions}});
    let ask = ask(AskKind::Question, input.clone());
    let answers = Map::from_iter([
        ("Which database?".into(), json!("Postgres")),
        ("Which checks?".into(), json!(["Tests", "Lint"])),
    ]);
    assert_eq!(
        hook_output(&ask, &Answer::Answers(answers)),
        Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {
                    "questions": input["tool_input"]["questions"],
                    "answers": {
                        "Which database?": "Postgres",
                        "Which checks?": "Tests, Lint",
                    },
                },
            },
        }))
    );
}

#[test]
fn answering_in_the_terminal_prints_nothing() {
    for kind in [AskKind::Permission, AskKind::Plan, AskKind::Question] {
        assert_eq!(hook_output(&ask(kind, permission_input()), &Answer::Terminal), None);
    }
}

#[test]
fn an_answer_is_read_from_what_the_composer_sends() {
    assert_eq!(Answer::from_json(&json!({"action": "allow"})), Some(Answer::Allow));
    assert_eq!(Answer::from_json(&json!({"action": "allow_always"})), Some(Answer::AllowAlways));
    assert_eq!(
        Answer::from_json(&json!({"action": "deny", "note": "no"})),
        Some(Answer::Deny { note: Some("no".into()) })
    );
    assert_eq!(Answer::from_json(&json!({"action": "approve"})), Some(Answer::Approve));
    assert_eq!(
        Answer::from_json(&json!({"action": "keep_planning", "note": "wait"})),
        Some(Answer::KeepPlanning { note: Some("wait".into()) })
    );
    assert_eq!(
        Answer::from_json(&json!({"action": "answers", "answers": {"Q?": "A"}})),
        Some(Answer::Answers(Map::from_iter([("Q?".into(), json!("A"))])))
    );
    assert_eq!(Answer::from_json(&json!({"action": "terminal"})), Some(Answer::Terminal));
    assert_eq!(Answer::from_json(&json!({"action": "what"})), None);
    assert_eq!(Answer::from_json(&json!({})), None);
}

#[test]
fn a_failed_check_before_start_says_so_and_keeps_the_terminal_open() {
    let mut conversation = Conversation::default();
    let text = "Couldn't check whether Claude is signed in: boom";
    note_lead_check_failed(&mut conversation, false, text.into());
    match conversation.items.last() {
        Some(Item::Notice { text: shown, .. }) => assert_eq!(shown, text),
        other => panic!("{other:?}"),
    }
    assert_eq!(conversation.terminal, TerminalVis::Shown);
    assert_eq!(conversation.reveal, Some(Reveal::Startup));
}

#[test]
fn a_failed_check_after_start_only_says_so() {
    let mut conversation = Conversation::default();
    note_lead_check_failed(&mut conversation, true, "Couldn't check whether Claude is signed in: boom".into());
    assert!(matches!(conversation.items.last(), Some(Item::Notice { .. })));
    assert_eq!(conversation.terminal, TerminalVis::Hidden);
    assert_eq!(conversation.reveal, None);
}

#[test]
fn a_failed_check_keeps_a_login_reveal() {
    let mut conversation = Conversation::default();
    apply_auth_status(&mut conversation, 1);
    note_lead_check_failed(&mut conversation, false, "Couldn't check whether Claude is signed in: boom".into());
    assert_eq!(conversation.reveal, Some(Reveal::Login));
    assert!(matches!(conversation.items.last(), Some(Item::Notice { .. })));
}

#[test]
fn a_reattached_lead_is_started_only_after_its_own_session_start() {
    assert!(started_on_reattach(None, "t9"));
    assert!(!started_on_reattach(Some(""), "t9"));
    assert!(started_on_reattach(Some("t9"), "t9"));
    assert!(!started_on_reattach(Some("t8"), "t9"));
}

#[test]
fn a_path_is_a_shell_word_with_forward_slashes_and_quotes_only_for_a_space() {
    assert_eq!(shell_word(r"C:\a\b.exe"), "C:/a/b.exe");
    assert_eq!(shell_word(r"C:\My Apps\b.exe"), r#""C:/My Apps/b.exe""#);
}

#[test]
fn claude_settings_name_the_status_line_and_the_compaction_hooks() {
    let value = claude_settings(
        Path::new(r"C:\My Apps\kompitum-app.exe"),
        Path::new(r"C:\kom\kom.exe"),
        Path::new(r"C:\Users\me\kompitum-app"),
    );
    let command = value["statusLine"]["command"].as_str().unwrap();
    assert_eq!(value["statusLine"]["type"], json!("command"));
    assert!(!command.contains('\\'), "{command}");
    assert!(command.starts_with(r#""C:/My Apps/kompitum-app.exe" hook StatusLine "#), "{command}");
    assert!(command.ends_with("C:/Users/me/kompitum-app"), "{command}");

    let pre = &value["hooks"]["PreCompact"][0]["hooks"][0];
    assert_eq!(pre["args"], json!(["hook", "PreCompact", r"C:\Users\me\kompitum-app"]));
    assert_eq!(pre["timeout"], json!(10));
    assert!(pre.get("async").is_none(), "{pre}");
    let post = &value["hooks"]["PostCompact"][0]["hooks"][0];
    assert_eq!(post["args"][1], json!("PostCompact"));
    assert_eq!(post["async"], json!(true));
}

#[test]
fn a_compaction_is_a_notice_and_its_summary_and_meta_lines_are_not_owner_words() {
    let text = [
        r#"{"type":"user","message":{"role":"user","content":"First"},"uuid":"u1"}"#,
        r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted","timestamp":"2026-11-11T15:27:19.119Z","uuid":"b1","compactMetadata":{"trigger":"manual","preTokens":353854}}"#,
        r#"{"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"role":"user","content":"This session is being continued from a previous conversation."},"uuid":"s1"}"#,
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>meta</local-command-caveat>"},"uuid":"m1"}"#,
        r#"{"type":"user","message":{"role":"user","content":"Second"},"uuid":"u2"}"#,
    ]
    .join("\n");
    let loaded = items_from_transcript(&text);
    assert_eq!(loaded.skipped, 0);
    assert_eq!(loaded.items.len(), 3, "{:?}", loaded.items);
    assert!(matches!(&loaded.items[0], Item::Owner { text, .. } if text == "First"));
    assert!(matches!(&loaded.items[2], Item::Owner { text, .. } if text == "Second"));
    match &loaded.items[1] {
        Item::Notice { id, text } => {
            assert_eq!(id, "b1");
            assert!(text.starts_with("Tidied up the conversation"), "{text}");
        }
        other => panic!("expected a notice, got {other:?}"),
    }
}

#[test]
fn a_compact_command_makes_no_owner_item_and_keeps_the_terminal_as_it_is() {
    let owners = |prompt: &str| {
        let mut conversation = Conversation::default();
        conversation.show(Reveal::Auto, Some("Claude is showing its menu.".into()));
        let after = fold(
            conversation,
            &Event::UserPromptSubmit {
                prompt: prompt.into(),
            },
        );
        (after.items.len(), after.reveal)
    };
    assert_eq!(owners("/compact"), (0, Some(Reveal::Auto)));
    assert_eq!(owners("/compact keep the schema"), (0, Some(Reveal::Auto)));
    assert_eq!(owners("/compacting is slow"), (1, None));
}
