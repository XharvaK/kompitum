//! The lead: one conversation per CLI (`audit/REDESIGN.md`, "One stream"),
//! the Claude settings file that keeps the app's copy of its session, and the
//! argv it starts with. Decisions live here, away from the window and the
//! process, so they test on their own.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The compaction window each Claude terminal the app starts gets
/// (the Owner, 2026-11-08). Part 5b's settings file uses it too.
pub const AUTO_COMPACT_WINDOW: u64 = 444_000;

/// One program's entry in `programs.json`, as the lead needs it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct CatalogEntry {
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub flags: Option<Flags>,
    #[serde(default)]
    pub session: Option<Sessions>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Flags {
    #[serde(default)]
    pub model: Vec<String>,
    #[serde(default)]
    pub effort: Vec<String>,
}

/// Up to three argv templates: a new conversation, one resumed by id, and the
/// latest one in the folder for a CLI that takes no id up front.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Sessions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<Vec<String>>,
    #[serde(default, rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continuation: Option<Vec<String>>,
}

/// What `lead.streams` holds for one program: the folder its conversation is
/// in, and its id where the CLI takes one.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Stream {
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Resume { id: String },
    Continue,
    New { id: String },
    Plain,
}

impl Choice {
    pub fn is_resume_or_continue(&self) -> bool {
        matches!(self, Self::Resume { .. } | Self::Continue)
    }
}

/// The record for `program`, when it has one.
pub fn stream_of(streams: &Value, program: &str) -> Option<Stream> {
    serde_json::from_value(streams.get(program)?.clone()).ok()
}

/// Which start this program gets, checked in the order the brief names:
/// resume the same folder's conversation, continue it, start a new one, else
/// a plain start.
pub fn choose(
    catalog: &CatalogEntry,
    stream: Option<&Stream>,
    folder: &str,
    fresh: &str,
) -> Choice {
    if let Some(stream) = stream.filter(|s| s.folder == folder) {
        let sessions = catalog.session.as_ref();
        if let (Some(id), Some(_)) = (
            stream.session.as_deref().filter(|id| !id.is_empty()),
            sessions.and_then(|s| s.resume.as_ref()),
        ) {
            return Choice::Resume { id: id.to_owned() };
        }
        if sessions.and_then(|s| s.continuation.as_ref()).is_some() {
            return Choice::Continue;
        }
    }
    if catalog
        .session
        .as_ref()
        .and_then(|s| s.new.as_ref())
        .is_some()
    {
        return Choice::New {
            id: fresh.to_owned(),
        };
    }
    Choice::Plain
}

/// The record to write before this start.
pub fn stream_after_choice(choice: &Choice, folder: &str, stream: Option<&Stream>) -> Stream {
    match choice {
        Choice::New { id } => Stream {
            folder: folder.to_owned(),
            session: Some(id.clone()),
        },
        Choice::Plain => Stream {
            folder: folder.to_owned(),
            session: None,
        },
        Choice::Resume { .. } | Choice::Continue => Stream {
            folder: folder.to_owned(),
            session: stream.and_then(|s| s.session.clone()),
        },
    }
}

/// The argv a start runs: the program, the session template for this choice,
/// the rest of `run`, then the model and reasoning flags.
pub fn argv(
    catalog: &CatalogEntry,
    choice: &Choice,
    model: Option<&str>,
    effort: Option<&str>,
) -> Vec<String> {
    let Some(first) = catalog.run.first() else {
        return Vec::new();
    };
    let mut out = vec![first.clone()];
    let template = match choice {
        Choice::Resume { .. } => catalog.session.as_ref().and_then(|s| s.resume.as_ref()),
        Choice::Continue => catalog
            .session
            .as_ref()
            .and_then(|s| s.continuation.as_ref()),
        Choice::New { .. } => catalog.session.as_ref().and_then(|s| s.new.as_ref()),
        Choice::Plain => None,
    };
    if let Some(template) = template {
        let id = match choice {
            Choice::Resume { id } | Choice::New { id } => id.as_str(),
            _ => "",
        };
        out.extend(template.iter().map(|word| word.replace("{id}", id)));
    }
    out.extend(catalog.run.iter().skip(1).cloned());
    if let Some(model) = model.filter(|m| !m.is_empty())
        && let Some(flags) = &catalog.flags
    {
        out.extend(flags.model.iter().map(|word| {
            word.replace("{model}", model)
                .replace("{effort}", effort.unwrap_or(""))
        }));
    }
    if let Some(effort) = effort.filter(|e| !e.is_empty())
        && let Some(flags) = &catalog.flags
    {
        out.extend(flags.effort.iter().map(|word| {
            word.replace("{effort}", effort)
                .replace("{model}", model.unwrap_or(""))
        }));
    }
    out
}

/// A terminal is the lead when it carries the label and still runs the lead's
/// program: a kept terminal that a reboot replaced with a shell keeps the label
/// but must be resumed, not focused.
pub fn is_lead_record(record: &Value, program: &str) -> bool {
    record
        .pointer("/labels/lead")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
        && record.pointer("/program/key").and_then(Value::as_str) == Some(program)
}

/// Nanoseconds since the Unix epoch, the clock a fresh id mixes in.
pub fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// A version 4 UUID in lower case, from 16 bytes of SHA-256 over the time,
/// process and folder. Unique, not secret.
pub fn fresh_id(now_ns: u128, pid: u32, folder: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(now_ns.to_le_bytes());
    digest.update(pid.to_le_bytes());
    digest.update(folder.as_bytes());
    let bytes = digest.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&bytes[..16]);
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    let hex = |part: &[u8]| part.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        hex(&id[..4]),
        hex(&id[4..6]),
        hex(&id[6..8]),
        hex(&id[8..10]),
        hex(&id[10..])
    )
}

/// Whether a resume or continue ended within ten seconds with a non-zero exit
/// code, and so gets one fresh start.
pub fn retry_needed(record: &Value, already_retried: bool) -> bool {
    if already_retried || record.get("reason").and_then(Value::as_str) != Some("exited") {
        return false;
    }
    if record
        .get("exit_code")
        .and_then(Value::as_i64)
        .is_none_or(|code| code == 0)
    {
        return false;
    }
    let Some(elapsed) = record
        .get("finished_ms")
        .and_then(Value::as_u64)
        .zip(record.get("created_ms").and_then(Value::as_u64))
        .map(|(finished, created)| finished.saturating_sub(created))
    else {
        return false;
    };
    elapsed <= 10_000
}

/// The settings file a Claude lead starts with: one memory (the vault's), the
/// compaction window for each model, and the two SessionStart handlers.
pub fn claude_settings(app: &Path, core: &Path) -> Value {
    let app = app.to_string_lossy();
    let core = core.to_string_lossy();
    let mut models = Map::new();
    for model in [
        "claude-fable-5-1",
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
    ] {
        models.insert(
            model.into(),
            json!({ "autoCompactWindow": AUTO_COMPACT_WINDOW }),
        );
    }
    json!({
        "autoMemoryEnabled": false,
        "modelSettings": models,
        "hooks": {
            "SessionStart": [{
                "hooks": [
                    {
                        "type": "command",
                        "command": app,
                        "args": ["hook", "SessionStart"],
                        "timeout": 10,
                    },
                    {
                        "type": "command",
                        "command": core,
                        "args": ["vault", "memory", "--hook"],
                        "timeout": 30,
                    },
                ],
            }],
        },
    })
}

/// Writes the lead's Claude settings file, made fresh every start.
pub fn write_claude_settings(app_dir: &Path, app: &Path, core: &Path) -> Result<PathBuf, String> {
    let path = app_dir.join("lead").join("claude-settings.json");
    let fail = |error: std::io::Error| {
        format!(
            "Couldn't write the lead's settings file {}: {error}",
            path.display()
        )
    };
    let text = serde_json::to_string_pretty(&claude_settings(app, core)).unwrap_or_default() + "\n";
    std::fs::create_dir_all(path.parent().unwrap_or(app_dir))
        .and_then(|()| crate::persist::write_atomic(&path, text))
        .map_err(fail)?;
    Ok(path)
}

/// A SessionStart from the lead's own terminal records the id the app should
/// resume. Anything else changes nothing.
pub fn record_session_start(
    streams: &mut Value,
    from_lead: bool,
    program_is_claude: bool,
    event: &str,
    session_id: Option<&str>,
) -> bool {
    let Some(id) = session_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return false;
    };
    if !from_lead || !program_is_claude || event != "SessionStart" {
        return false;
    }
    if !streams.is_object() {
        *streams = json!({});
    }
    let all = streams.as_object_mut().expect("made an object");
    let entry = all.entry("claude").or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    let stream = entry.as_object_mut().expect("made an object");
    if stream.get("session").and_then(Value::as_str) == Some(id) {
        return false;
    }
    stream.insert("session".into(), json!(id));
    true
}

#[cfg(test)]
mod tests {
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
                session: Some("old".into())
            })
        );
        assert_eq!(
            stream_of(&records, "codex"),
            Some(Stream {
                folder: "/home/me".into(),
                session: None
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
                session: Some("new".into())
            }
        );
        assert_eq!(
            stream_after_choice(&Choice::Plain, "/home/me", Some(&stream)),
            Stream {
                folder: "/home/me".into(),
                session: None
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
            "labels": {"lead": "1"},
            "program": {"key": "claude"},
        });
        assert!(is_lead_record(&lead, "claude"));
        assert!(!is_lead_record(&lead, "codex"));
        let lost = json!({
            "labels": {"lead": "1"},
            "program": {"key": "shell"},
        });
        assert!(!is_lead_record(&lost, "claude"));
        let unlabelled = json!({"program": {"key": "claude"}});
        assert!(!is_lead_record(&unlabelled, "claude"));
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
    fn claude_settings_carry_the_two_hooks_and_its_memory_off() {
        let value = claude_settings(
            Path::new("/apps/Kompitum/kompitum-app.exe"),
            Path::new("/apps/kom/kom.exe"),
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
                "args": ["hook", "SessionStart"],
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
    fn a_session_start_from_the_lead_records_its_id() {
        let mut streams = json!({"claude": {"folder": "/home/me", "session": "old"}});
        assert!(record_session_start(
            &mut streams,
            true,
            true,
            "SessionStart",
            Some("new-session")
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
            Some("new-session")
        ));
        let mut fresh = json!({});
        assert!(record_session_start(
            &mut fresh,
            true,
            true,
            "SessionStart",
            Some("one")
        ));
        assert_eq!(fresh, json!({"claude": {"session": "one"}}));
        assert!(!record_session_start(
            &mut fresh,
            false,
            true,
            "SessionStart",
            Some("two")
        ));
        assert!(!record_session_start(
            &mut fresh,
            true,
            false,
            "SessionStart",
            Some("two")
        ));
        assert!(!record_session_start(
            &mut fresh,
            true,
            true,
            "Stop",
            Some("two")
        ));
        assert_eq!(fresh, json!({"claude": {"session": "one"}}));
    }
}
