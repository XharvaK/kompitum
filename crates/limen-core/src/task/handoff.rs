//! The hand-off from a worker that stopped to the next one: what its turn did,
//! the note it leaves in the worktree, and the one sentence a wake carries.
//! Pure: `task.rs` reads the disk and hands this module plain values.

use crate::task::decide::TerminalNow;
use crate::task::types::{Task, cut};
use crate::task::worker::TurnOutcome;

/// One changed file as the note lists it: its diff and what is on disk now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub added: u64,
    pub removed: u64,
    /// Its size now; None means it was deleted.
    pub bytes: Option<u64>,
}

/// The longest list a note carries before it counts the rest.
const LIST_LIMIT: usize = 200;

/// What happened to a worker, in the words a note and a wake use.
pub fn stopped_words(window: &str, steps: u64, cost_usd: f64) -> String {
    if steps == 0 {
        return "refused it at once".into();
    }
    let what = match window {
        "credits" => "ran out of credits",
        "5-hour" => "hit its 5-hour limit",
        "weekly" => "hit its weekly limit",
        "monthly" => "hit its monthly limit",
        "daily" => "hit its daily limit",
        "rate" => "hit its rate limit",
        "auth" => "lost its login",
        _ => "hit a limit",
    };
    let steps = if steps == 1 { "1 step".to_string() } else { format!("{steps} steps") };
    if !cost_usd.is_finite() || cost_usd == 0.0 {
        format!("{what} after {steps}")
    } else {
        format!("{what} after {steps} and ${cost_usd:.2}")
    }
}

/// The one sentence the wake carries: who stopped, why, and where the next
/// worker starts.
pub fn sentence(from: &str, stopped: &str, to: &str, changed: usize, has_note: bool) -> String {
    if changed > 0 {
        let files = if changed == 1 { "1 changed file".to_string() } else { format!("{changed} changed files") };
        format!("{from} {stopped}, so {to} went on from {files}.")
    } else if has_note {
        format!("{from} {stopped}, so {to} went on from its notes.")
    } else {
        format!("{from} {stopped}, so {to} started it fresh.")
    }
}

/// A turn that used no step and changed no file did nothing to hand on.
pub fn did_nothing(steps: u64, changed_count: usize) -> bool {
    steps == 0 && changed_count == 0
}

/// The words a task fails with once every turn is used, or None while one is left.
pub fn stopped_at_turn_limit(turns: usize, words: &str) -> Option<String> {
    (turns >= crate::task::MAX_TURNS).then(|| format!("stopped after {turns} turns: {words}"))
}

/// The path under `worktree` with forward slashes, comparing case-insensitively
/// and either slash; a path outside it is left as it is.
pub fn relative(path: &str, worktree: &str) -> String {
    let path = path.replace('\\', "/");
    let worktree = worktree.replace('\\', "/");
    let worktree = worktree.trim_end_matches('/');
    if worktree.is_empty() {
        return path;
    }
    let Some(head) = path.get(..worktree.len()).filter(|head| head.eq_ignore_ascii_case(worktree)) else {
        return path;
    };
    if head.len() == path.len() {
        return String::new();
    }
    match path.get(worktree.len()..).and_then(|rest| rest.strip_prefix('/')) {
        Some(rest) => rest.to_string(),
        None => path,
    }
}

/// The note the next worker reads before the brief. A section with nothing in
/// it is left out; each list stops at 200 lines and then counts the rest.
pub fn note(
    from: &str,
    stopped: &str,
    changed: &[FileChange],
    read: &[String],
    reasoning: &str,
    message: &str,
) -> String {
    let mut text = format!(
        "# You are taking over from another worker\n\n\
         {from} {stopped}.\n\n\
         Its work is in this worktree, uncommitted. Assume it is right unless a check fails. Read only what you \
         need in order to finish: reading it all again costs more than anything else you could do.\n\n"
    );
    let changes: Vec<String> = changed
        .iter()
        .map(|file| match file.bytes {
            Some(bytes) => format!("- `{}` +{} -{}, {bytes} bytes", file.path, file.added, file.removed),
            None => format!("- `{}` deleted", file.path),
        })
        .collect();
    section(&mut text, "## The files it changed", &changes);
    let reads: Vec<String> = read.iter().map(|path| format!("- `{path}`")).collect();
    section(&mut text, "## The files it read", &reads);
    paragraph(&mut text, "## The end of its reasoning", reasoning);
    paragraph(&mut text, "## Its last message", message);
    text.push_str("THE BRIEF FOLLOWS. Do what is left of it.\n\n---\n\n");
    text
}

fn section(text: &mut String, title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    text.push_str(title);
    text.push_str("\n\n");
    for line in lines.iter().take(LIST_LIMIT) {
        text.push_str(line);
        text.push('\n');
    }
    if lines.len() > LIST_LIMIT {
        text.push_str(&format!("...and {} more\n", lines.len() - LIST_LIMIT));
    }
    text.push('\n');
}

fn paragraph(text: &mut String, title: &str, body: &str) {
    if body.trim().is_empty() {
        return;
    }
    text.push_str(title);
    text.push_str("\n\n");
    text.push_str(body);
    text.push_str("\n\n");
}

/// A worker that exited with an error in its first minute having written
/// nothing never began: nobody's quota was spent, and a second try is free.
pub fn died_at_start(
    task: &Task,
    terminal: Option<&TerminalNow>,
    code: Option<i32>,
    outcome: &TurnOutcome,
    now_ms: u64,
) -> bool {
    if task.ending.is_some() {
        return false;
    }
    if code.is_none_or(|code| code == 0) {
        return false;
    }
    if terminal.and_then(|terminal| terminal.reason.as_deref()).is_some_and(|reason| reason != "exited") {
        return false;
    }
    let wrote_nothing = outcome.session.is_none()
        && outcome.text.is_empty()
        && outcome.tools.is_empty()
        && outcome.usage.steps == 0
        && outcome.errors.is_empty()
        && outcome.parse_errors == 0
        && outcome.reported.is_none();
    if !wrote_nothing {
        return false;
    }
    let Some(turn) = task.turns.last() else { return false };
    if now_ms.saturating_sub(turn.started_ms) >= 60_000 {
        return false;
    }
    // A worker gets three starts: two earlier deaths at its start are enough.
    let earlier: Vec<bool> = task.turns.iter().rev().skip(1).take(2).map(|turn| turn.died_at_start).collect();
    earlier.len() < 2 || earlier.iter().any(|died| !died)
}

/// The first line of a worker's stderr, cut for a warning.
pub fn first_line(stderr: &str) -> String {
    let line = stderr.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    cut(line, 200)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::types::Turn;

    fn change(path: &str, added: u64, removed: u64, bytes: Option<u64>) -> FileChange {
        FileChange { path: path.into(), added, removed, bytes }
    }

    #[test]
    fn the_note_lists_files_and_leaves_out_what_is_empty() {
        let changed = [change("src/a.rs", 12, 3, Some(4210)), change("src/b.rs", 0, 5, None)];
        let text = note(
            "Command Code",
            "ran out of credits after 68 steps and $0.13",
            &changed,
            &["src/c.rs".to_string()],
            "I checked the tests.",
            "",
        );
        assert_eq!(
            text,
            "# You are taking over from another worker\n\n\
             Command Code ran out of credits after 68 steps and $0.13.\n\n\
             Its work is in this worktree, uncommitted. Assume it is right unless a check fails. Read only what you \
             need in order to finish: reading it all again costs more than anything else you could do.\n\n\
             ## The files it changed\n\n\
             - `src/a.rs` +12 -3, 4210 bytes\n\
             - `src/b.rs` deleted\n\n\
             ## The files it read\n\n\
             - `src/c.rs`\n\n\
             ## The end of its reasoning\n\n\
             I checked the tests.\n\n\
             THE BRIEF FOLLOWS. Do what is left of it.\n\n---\n\n"
        );
        let bare = note("Command Code", "refused it at once", &[], &[], "", "  ");
        assert!(!bare.contains("## "), "{bare}");
        assert!(bare.ends_with("THE BRIEF FOLLOWS. Do what is left of it.\n\n---\n\n"), "{bare}");
    }

    #[test]
    fn the_note_stops_a_long_list_at_200() {
        let changed: Vec<FileChange> = (0..207).map(|n| change(&format!("src/f{n}.rs"), 1, 0, Some(1))).collect();
        let text = note("Command Code", "hit its weekly limit after 3 steps", &changed, &[], "", "");
        assert!(text.contains("- `src/f199.rs` +1 -0, 1 bytes\n"), "{text}");
        assert!(!text.contains("src/f200.rs"), "{text}");
        assert!(text.contains("...and 7 more\n"), "{text}");
    }

    #[test]
    fn stopped_words_say_what_happened() {
        assert_eq!(stopped_words("credits", 0, 0.0), "refused it at once");
        assert_eq!(stopped_words("credits", 68, 0.13), "ran out of credits after 68 steps and $0.13");
        assert_eq!(stopped_words("weekly", 68, 0.13), "hit its weekly limit after 68 steps and $0.13");
        assert_eq!(stopped_words("5-hour", 3, 0.0), "hit its 5-hour limit after 3 steps");
        assert_eq!(stopped_words("monthly", 3, f64::NAN), "hit its monthly limit after 3 steps");
        assert_eq!(stopped_words("daily", 3, f64::INFINITY), "hit its daily limit after 3 steps");
        assert_eq!(stopped_words("rate", 1, 0.0), "hit its rate limit after 1 step");
        assert_eq!(stopped_words("auth", 1, 0.01), "lost its login after 1 step and $0.01");
        assert_eq!(stopped_words("surprise", 2, 0.0), "hit a limit after 2 steps");
    }

    #[test]
    fn the_sentence_says_what_the_next_worker_starts_from() {
        let stopped = stopped_words("credits", 68, 0.13);
        assert_eq!(
            sentence("Command Code", &stopped, "opencode Go", 1, false),
            "Command Code ran out of credits after 68 steps and $0.13, so opencode Go went on from 1 changed file."
        );
        assert_eq!(
            sentence("Command Code", &stopped, "opencode Go", 2, false),
            "Command Code ran out of credits after 68 steps and $0.13, so opencode Go went on from 2 changed files."
        );
        assert_eq!(
            sentence("Command Code", &stopped, "opencode Go", 0, true),
            "Command Code ran out of credits after 68 steps and $0.13, so opencode Go went on from its notes."
        );
        assert_eq!(
            sentence("Command Code", "refused it at once", "opencode Go", 0, false),
            "Command Code refused it at once, so opencode Go started it fresh."
        );
    }

    #[test]
    fn relative_handles_either_slash_and_case() {
        assert_eq!(relative(r"C:\Work\Tree\src\a.rs", "c:/work/tree"), "src/a.rs");
        assert_eq!(relative("/wt/src/a.rs", "/WT/"), "src/a.rs");
        assert_eq!(relative("src/a.rs", "/wt"), "src/a.rs");
        assert_eq!(relative("/wt2/a.rs", "/wt"), "/wt2/a.rs");
        assert_eq!(relative("/wt", "/wt"), "");
    }

    #[test]
    fn a_death_at_the_start_needs_a_clean_stream_and_two_earlier_starts() {
        let exited = TerminalNow { live: false, exit_code: Some(1), reason: Some("exited".into()) };
        let now = 10_000;
        let mut task = Task { turns: vec![Turn { started_ms: 1_000, ..Default::default() }], ..Default::default() };
        let quiet = TurnOutcome::default();
        assert!(died_at_start(&task, Some(&exited), Some(1), &quiet, now));
        assert!(!died_at_start(&task, Some(&exited), Some(0), &quiet, now), "a clean exit is not a failed start");
        let killed = TerminalNow { live: false, exit_code: Some(1), reason: Some("killed".into()) };
        assert!(!died_at_start(&task, Some(&killed), Some(1), &quiet, now));
        task.ending = Some(crate::task::types::Ending::Cancel);
        assert!(!died_at_start(&task, Some(&exited), Some(1), &quiet, now));
        task.ending = None;
        let spoken = TurnOutcome { text: "hello".into(), ..Default::default() };
        assert!(!died_at_start(&task, Some(&exited), Some(1), &spoken, now));
        assert!(!died_at_start(&task, Some(&exited), Some(1), &quiet, 61_000), "past its first minute");

        task.turns[0].died_at_start = true;
        task.turns.push(Turn { started_ms: 9_000, ..Default::default() });
        task.turns[1].died_at_start = true;
        task.turns.push(Turn { started_ms: 9_500, ..Default::default() });
        assert!(!died_at_start(&task, Some(&exited), Some(1), &quiet, now), "three starts are enough");
        task.turns[1].died_at_start = false;
        assert!(died_at_start(&task, Some(&exited), Some(1), &quiet, now));
    }
}
