//! New notes from the vault, once a minute: which ones pop up, and which wait.
//!
//! The IO is one watcher for the whole app, off the UI thread: `kom vault
//! changes --json --as owner` through the app's headless spawn. It raises the
//! same toast `notify` does, with the mailbox at that note as its click. What
//! pops is decided apart from the IO, so it tests without a vault.

use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const CHECK_EVERY: Duration = Duration::from_secs(60);
const FADE_MS: f64 = 5000.0;

/// A note from `kom vault changes --json`, as much as a pop-up needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Note {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: String,
}

/// What a new note shows: the note to open, what the toast says, and whether it
/// waits for the person instead of fading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pop {
    pub path: String,
    pub title: String,
    pub body: String,
    pub stays: bool,
}

#[derive(Deserialize)]
struct Changes {
    #[serde(default)]
    notes: Vec<Note>,
}

/// The notes that pop up, in the order they came. The person's
/// `notes_between_agents` decides whether notes to other agents show.
pub fn pops(notes: &[Note]) -> Vec<Pop> {
    pops_with(notes, crate::settings::notes_between_agents())
}

fn pops_with(notes: &[Note], between_agents: bool) -> Vec<Pop> {
    // The vault addresses a note to `all` to the Owner too, and the Owner's own notes never pop.
    let for_owner = |note: &Note| note.to == "owner" || note.to == "all";
    notes
        .iter()
        .filter(|note| !note.path.is_empty() && note.from != "owner")
        .filter(|note| for_owner(note) || between_agents)
        .map(|note| {
            let kind = note.kind.as_deref().unwrap_or("");
            Pop {
                path: note.path.clone(),
                title: format!("{} → {}", note.from, note.to),
                body: note.title.clone(),
                // What needs an answer waits; everything else passes.
                stays: for_owner(note) && matches!(kind, "" | "request" | "question"),
            }
        })
        .collect()
}

/// Starts the one watcher. Later calls do nothing.
pub fn start() {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    if let Err(e) = std::thread::Builder::new().name("mail".into()).spawn(watch) {
        tracing::warn!("cannot start the vault watcher: {e}");
    }
}

fn watch() {
    let mut failed: HashSet<String> = HashSet::new();
    loop {
        match check() {
            Ok(notes) => {
                failed.clear();
                for pop in pops(&notes) {
                    // Passing notes wait out a fullscreen game; the ones that
                    // need the person stay put and show.
                    if !pop.stays && crate::quiet::quiet() {
                        continue;
                    }
                    raise(&pop);
                }
            }
            Err(kind) => {
                // Once per kind: a vault that stays broken doesn't fill the log.
                if failed.insert(kind.clone()) {
                    tracing::warn!("kom vault changes: {kind}");
                }
            }
        }
        std::thread::sleep(CHECK_EVERY);
    }
}

fn check() -> Result<Vec<Note>, String> {
    let out = crate::spawn::command(crate::core_bin::path())
        .args(["vault", "changes", "--json", "--as", "owner"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run kom: {e}"))?;
    if !out.status.success() {
        return Err(failure_words(&out));
    }
    let changes: Changes =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("did not answer JSON: {e}"))?;
    Ok(changes.notes)
}

/// What a failed `kom` said: its JSON error message, else its stderr or stdout.
fn failure_words(out: &std::process::Output) -> String {
    for bytes in [&out.stderr, &out.stdout] {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
            && let Some(message) = value
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
        {
            return message.to_owned();
        }
        let text = String::from_utf8_lossy(bytes).trim().to_owned();
        if !text.is_empty() {
            return text;
        }
    }
    format!("kom exited with {}", out.status)
}

fn raise(pop: &Pop) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let id = format!("mail-{}", SEQ.fetch_add(1, Ordering::Relaxed));
    let notification = json!({
        "title": pop.title,
        "body": pop.body,
        "level": "info",
        "timeout": if pop.stays { 0.0 } else { FADE_MS },
        "action": { "name": "app.mailbox", "args": { "note": pop.path } },
    });
    crate::mods::request(crate::mods::Request::OpenOverlay {
        id,
        spec: json!({ "kind": "toast", "notification": notification }),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, from: &str, to: &str, kind: Option<&str>) -> Note {
        Note {
            path: path.into(),
            from: from.into(),
            to: to.into(),
            kind: kind.map(str::to_owned),
            title: format!("{path} title"),
        }
    }

    #[test]
    fn notes_that_need_the_person_stay_and_others_fade() {
        let notes = [
            note("a", "manager", "owner", None),
            note("b", "manager", "owner", Some("request")),
            note("c", "manager", "owner", Some("question")),
            note("d", "manager", "owner", Some("report")),
            note("e", "manager", "owner", Some("decision")),
        ];
        let pops = pops_with(&notes, true);
        assert_eq!(
            pops.iter().map(|p| p.stays).collect::<Vec<_>>(),
            [true, true, true, false, false]
        );
        assert_eq!(pops[1].title, "manager → owner");
        assert_eq!(pops[1].body, "b title");
        assert_eq!(pops[1].path, "b");
        assert!(pops.iter().all(|p| !p.title.is_empty()));
    }

    #[test]
    fn notes_between_agents_only_pop_when_the_setting_is_on() {
        let notes = [
            note("a", "worker", "manager", Some("report")),
            note("b", "manager", "owner", Some("request")),
        ];
        let on = pops_with(&notes, true);
        assert_eq!(on.len(), 2);
        assert!(!on[0].stays, "a note between agents passes");
        let off = pops_with(&notes, false);
        assert_eq!(off.len(), 1, "only the Owner's note shows");
        assert_eq!(off[0].path, "b");
    }

    #[test]
    fn a_note_to_all_is_the_owners_and_the_owners_own_notes_never_pop() {
        let notes = [
            note("a", "manager", "all", Some("question")),
            note("b", "owner", "manager", Some("answer")),
        ];
        let off = pops_with(&notes, false);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].path, "a");
        assert!(off[0].stays, "a question to all waits for the Owner");
        assert_eq!(
            pops_with(&notes, true).len(),
            1,
            "the Owner's answer never pops"
        );
    }

    #[test]
    fn a_note_without_a_path_or_kind_is_handled_plainly() {
        let mut missing = note("", "worker", "owner", Some("request"));
        missing.kind = None;
        assert!(pops_with(&[missing], true).is_empty());
        let no_kind = note("a", "worker", "owner", None);
        assert!(pops_with(&[no_kind], true)[0].stays);
        let empty_kind = note("b", "worker", "owner", Some(""));
        assert!(pops_with(&[empty_kind], true)[0].stays);
    }

    #[test]
    fn a_toast_with_no_timeout_survives_the_sweep() {
        // The sweep closes an overlay only when it has an expiry; timeout 0
        // gives a toast none, so it waits until clicked or closed.
        let now = std::time::Instant::now();
        let zero = crate::window::toast_expiry(true, 0.0);
        assert_eq!(zero, None);
        assert!(!crate::window::sweepable(
            zero.map(|d| now + d),
            now + Duration::from_secs(600)
        ));
        let fading = crate::window::toast_expiry(true, FADE_MS).map(|d| now + d);
        assert!(crate::window::sweepable(
            fading,
            now + Duration::from_millis(FADE_MS as u64) + Duration::from_millis(1)
        ));
        assert!(!crate::window::sweepable(
            fading,
            now + Duration::from_millis(FADE_MS as u64) - Duration::from_millis(1)
        ));
    }
}
