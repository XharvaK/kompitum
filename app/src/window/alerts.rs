//! The app's two background alerts (workers back, Claude Code updated) and
//! Windows notifications for what needs the Owner. The decisions are pure so
//! they test without a window or a real notification.

use crate::lead::AskKind;
use serde_json::Value;
use std::collections::HashMap;

/// What a Windows notification says, and the tag that answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemNote {
    pub tag: String,
    pub title: String,
    pub body: String,
}

/// Whether something that needs the Owner gets a Windows notification now.
pub(crate) fn wants_system_notification(setting: bool, active: bool, quiet: bool, blocked: bool) -> bool {
    setting && !active && !quiet && blocked
}

/// An agent's "needs you" toast, as the system notification repeats it.
pub(crate) fn agent_note(addr: &str, alert: &Value) -> Option<SystemNote> {
    let title = alert.get("title").and_then(Value::as_str)?.to_string();
    let body = alert.get("body").and_then(Value::as_str).unwrap_or("").to_string();
    Some(SystemNote { tag: format!("needs-{addr}"), title, body })
}

/// The lead's ask, as a Windows notification.
pub(crate) fn ask_note(id: &str, kind: &AskKind) -> SystemNote {
    SystemNote {
        tag: format!("needs-{id}"),
        title: crate::strings::text("app.ask-title"),
        body: match kind {
            AskKind::Permission => crate::strings::text("app.ask-permission"),
            AskKind::Plan => crate::strings::text("app.ask-plan"),
            AskKind::Question => crate::strings::text("app.ask-question"),
        },
    }
}

/// Each worker's last state, by key.
pub(crate) fn worker_states(view: &Value) -> HashMap<String, String> {
    view.get("workers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|worker| {
            let key = worker.get("key").and_then(Value::as_str)?;
            let state = worker.get("state").and_then(Value::as_str)?;
            Some((key.to_string(), state.to_string()))
        })
        .collect()
}

/// The labels of workers whose state went from `out` to `ready`.
pub(crate) fn workers_back(previous: &HashMap<String, String>, view: &Value) -> Vec<String> {
    view.get("workers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|worker| {
            let key = worker.get("key").and_then(Value::as_str).unwrap_or("");
            worker.get("state").and_then(Value::as_str) == Some("ready")
                && previous.get(key).map(String::as_str) == Some("out")
        })
        .filter_map(|worker| {
            let key = worker.get("key").and_then(Value::as_str).unwrap_or("");
            Some(
                worker
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|label| !label.is_empty())
                    .unwrap_or(key)
                    .to_string(),
            )
        })
        .collect()
}

/// The toast for a worker that came back.
pub(crate) fn worker_back_toast(label: &str) -> Value {
    serde_json::json!({
        "title": crate::strings::text_with("app.worker-back", &[("label", label)]),
        "body": crate::strings::text("app.worker-back-body"),
        "level": "info",
    })
}

/// The first token that looks like `N.N.N`.
pub(crate) fn claude_version(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let version = token.trim_matches(|c: char| !(c.is_ascii_digit() || c == '.'));
        let parts: Vec<&str> = version.split('.').collect();
        let looks = parts.len() == 3
            && parts.iter().all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
        looks.then(|| version.to_string())
    })
}

/// `old → new` when both are known and differ; nothing on a first start or the same version.
pub(crate) fn claude_version_change(previous: Option<&str>, next: &str) -> Option<String> {
    match previous {
        Some(previous) if previous != next => Some(format!("{previous} \u{2192} {next}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn worker(key: &str, label: &str, state: &str) -> Value {
        json!({"key": key, "label": label, "state": state})
    }

    #[test]
    fn only_a_worker_that_went_from_out_to_ready_came_back() {
        let previous = HashMap::from([
            ("cc".to_string(), "out".to_string()),
            ("go".to_string(), "ready".to_string()),
            ("codex".to_string(), "ready".to_string()),
        ]);
        let next = json!({"workers": [
            worker("cc", "Command Code", "ready"),
            worker("go", "opencode Go", "out"),
            worker("codex", "Codex", "ready"),
            worker("new", "New", "ready"),
        ]});
        assert_eq!(workers_back(&previous, &next), ["Command Code"]);
        assert_eq!(worker_states(&next)["cc"], "ready");
        assert_eq!(worker_states(&next)["go"], "out");

        let ready = HashMap::from([("cc".to_string(), "ready".to_string())]);
        let first = json!({"workers": [worker("cc", "Command Code", "ready")]});
        assert!(workers_back(&HashMap::new(), &first).is_empty(), "the first read is not news");
        assert!(workers_back(&ready, &first).is_empty(), "ready to ready is not news");
        let gone = json!({"workers": [worker("cc", "Command Code", "out")]});
        assert!(workers_back(&ready, &gone).is_empty(), "ready to out is not news");
        assert_eq!(
            worker_back_toast("Command Code"),
            json!({"title": "Command Code is back", "body": "It can take tasks again.", "level": "info"})
        );
    }

    #[test]
    fn the_version_is_the_first_token_that_looks_like_three_numbers() {
        assert_eq!(claude_version("1.2.3 (Claude Code)"), Some("1.2.3".into()));
        assert_eq!(claude_version("Claude Code v2.0.14\n"), Some("2.0.14".into()));
        assert_eq!(claude_version("version 3.4 (build 5)"), None);
        assert_eq!(claude_version("no version here"), None);
    }

    #[test]
    fn a_first_version_or_the_same_one_is_not_an_update() {
        assert_eq!(claude_version_change(None, "1.2.3"), None);
        assert_eq!(claude_version_change(Some("1.2.3"), "1.2.3"), None);
        assert_eq!(
            claude_version_change(Some("1.2.3"), "1.2.4"),
            Some("1.2.3 \u{2192} 1.2.4".into())
        );
    }

    #[test]
    fn a_notification_needs_the_setting_a_background_window_and_no_quiet() {
        assert!(wants_system_notification(true, false, false, true));
        assert!(!wants_system_notification(false, false, false, true), "the setting is off");
        assert!(!wants_system_notification(true, true, false, true), "Kompitum is in front");
        assert!(!wants_system_notification(true, false, true, true), "quiet");
        assert!(!wants_system_notification(true, false, false, false), "nothing needs the Owner");
    }

    #[test]
    fn the_tags_and_words_name_the_agent_or_the_ask() {
        let agent = agent_note("t1", &json!({"title": "Pi needs you", "body": "Local / Build"})).unwrap();
        assert_eq!(agent.tag, "needs-t1");
        assert_eq!(agent.title, "Pi needs you");
        assert_eq!(agent.body, "Local / Build");

        for (kind, body) in [
            (AskKind::Permission, "A permission"),
            (AskKind::Plan, "A plan to approve"),
            (AskKind::Question, "A question"),
        ] {
            let ask = ask_note("a1", &kind);
            assert_eq!(ask.tag, "needs-a1");
            assert_eq!(ask.title, "Claude needs you");
            assert_eq!(ask.body, body);
        }
    }
}
