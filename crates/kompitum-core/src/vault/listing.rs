//! Reading the vault: every note, the messages at `sender/to/recipient`,
//! the open list, search, threads, the change feed, and one note by path.

use super::git;
use super::marks;
use super::notes::{self, Meta, STATUSES};
use super::store::{Message, Query, SHOWN, unavailable};
use crate::paths::Paths;
use crate::rpc::RpcError;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(super) struct Entry {
    path: String,
    meta: Meta,
    body: String,
    title: String,
}

/// Every note in the vault, in path order, without following symlinks.
fn list_entries(root: &Path) -> Result<Vec<Entry>, RpcError> {
    let mut entries = Vec::new();
    walk(root, "", &mut entries)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn walk(dir: &Path, prefix: &str, entries: &mut Vec<Entry>) -> Result<(), RpcError> {
    let listing = std::fs::read_dir(dir).map_err(|e| unavailable(format!("cannot read {}: {e}", dir.display())))?;
    for entry in listing {
        let entry = entry.map_err(|e| unavailable(format!("cannot read {}: {e}", dir.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if marks::SKIP.contains(&name.as_str()) {
            continue;
        }
        let path = format!("{prefix}{name}");
        let kind =
            entry.file_type().map_err(|e| unavailable(format!("cannot read {}: {e}", entry.path().display())))?;
        if kind.is_dir() {
            walk(&entry.path(), &format!("{path}/"), entries)?;
        } else if kind.is_file() && name.ends_with(".md") {
            let text = read_text(&entry.path())?;
            let note = notes::parse_note(&text);
            let title = notes::title_of(&note, &name);
            entries.push(Entry { path, meta: note.meta, body: note.body, title });
        }
    }
    Ok(())
}

pub(super) fn read_text(path: &Path) -> Result<String, RpcError> {
    let bytes = std::fs::read(path).map_err(|e| unavailable(format!("cannot read {}: {e}", path.display())))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The sender and recipient a message path names.
pub(super) fn message_path(path: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 || parts[1] != "to" {
        return None;
    }
    let file = parts[3];
    if parts[0].is_empty() || parts[2].is_empty() || !file.ends_with(".md") || file.len() <= 3 {
        return None;
    }
    Some((parts[0].to_string(), parts[2].to_string()))
}

pub(super) fn message_of(entry: &Entry, index: &marks::Index, agent: &str) -> Message {
    let parsed = message_path(&entry.path);
    let origin = entry.meta.text("from");
    let recipients = entry.meta.list("to");
    let (sender, recipient) = match &parsed {
        Some((sender, recipient)) => (sender.clone(), recipient.clone()),
        None => (
            origin.clone().unwrap_or_else(|| entry.path.split('/').next().unwrap_or_default().to_string()),
            recipients.join(", "),
        ),
    };
    let status = notes::status_of(&entry.meta);
    let mut problems = Vec::new();
    if let Some((sender, recipient)) = &parsed {
        if let Some(origin) = &origin
            && origin != sender
        {
            problems.push(format!("says from: {origin} but sits in {sender}/"));
        }
        if !recipients.is_empty() && !recipients.iter().any(|to| to == recipient) {
            problems.push(format!("says to: {} but sits in to/{recipient}/", recipients.join(", ")));
        }
        if !STATUSES.contains(&status.as_str()) {
            problems.push(format!("has an unknown status: {status}"));
        }
    }
    Message {
        path: entry.path.clone(),
        meta: entry.meta.clone(),
        body: entry.body.clone(),
        title: entry.title.clone(),
        sender,
        recipient,
        status,
        problems,
        read_by: index.read_by(&entry.path),
        done: index.done(&entry.path, agent),
        snippet: None,
    }
}

pub(super) fn open_messages(
    root: &Path,
    index: &marks::Index,
    agent: &str,
) -> Result<(Vec<Message>, Vec<String>), RpcError> {
    let entries = list_entries(root)?;
    let mut answered = BTreeSet::new();
    for entry in &entries {
        if entry.path.starts_with(&format!("{agent}/")) {
            answered.extend(entry.meta.list("re"));
        }
    }
    let mut notes = Vec::new();
    for entry in &entries {
        let Some((sender, recipient)) = message_path(&entry.path) else { continue };
        if sender == agent || (recipient != agent && recipient != "all") {
            continue;
        }
        if answered.contains(&entry.path) {
            continue;
        }
        let message = message_of(entry, index, agent);
        if message.status == "answered" || message.status == "superseded" {
            continue;
        }
        let kind = message.kind();
        if matches!(kind.as_deref(), Some("report") | Some("answer")) {
            continue; // never wait
        }
        let closed_by_read = matches!(kind.as_deref(), Some("decision") | Some("tried"));
        if (closed_by_read && index.marked_by(&entry.path, agent)) || index.done_by(&entry.path, agent) {
            continue;
        }
        notes.push(message);
    }
    sort_newest_first(&mut notes);
    notes.sort_by_key(|note| !note.loud());
    let problems = note_problems(&notes);
    Ok((notes, problems))
}

pub(super) fn thread_notes(
    root: &Path,
    index: &marks::Index,
    me: &str,
    other: &str,
) -> Result<(Vec<Message>, Vec<String>), RpcError> {
    let entries = list_entries(root)?;
    let mut notes = Vec::new();
    for entry in &entries {
        let Some((sender, recipient)) = message_path(&entry.path) else { continue };
        let mine = sender == me && (recipient == other || recipient == "all");
        let theirs = sender == other && (recipient == me || recipient == "all");
        if mine || theirs {
            notes.push(message_of(entry, index, me));
        }
    }
    notes.sort_by(|a, b| {
        let left = (a.stamp().unwrap_or_default(), &a.path);
        let right = (b.stamp().unwrap_or_default(), &b.path);
        left.cmp(&right)
    });
    let problems = note_problems(&notes);
    Ok((notes, problems))
}

pub(super) fn find_notes(
    root: &Path,
    index: &marks::Index,
    agent: &str,
    query: &Query,
) -> Result<Vec<Message>, RpcError> {
    let entries = list_entries(root)?;
    let words: Vec<String> = query.words.to_lowercase().split_whitespace().map(str::to_string).collect();
    let mut notes = Vec::new();
    for entry in entries {
        if let Some(project) = &query.project
            && entry.meta.text("project").as_deref() != Some(project)
            && !entry.path.starts_with(&format!("{project}/"))
        {
            continue;
        }
        if let Some(kind) = &query.kind
            && entry.meta.text("kind").as_deref() != Some(kind)
        {
            continue;
        }
        if let Some(status) = &query.status
            && notes::status_of(&entry.meta) != *status
        {
            continue;
        }
        if let Some(origin) = &query.from
            && entry.meta.text("from").as_deref() != Some(origin)
            && !entry.path.starts_with(&format!("{origin}/"))
        {
            continue;
        }
        let mut values = Vec::new();
        for value in entry.meta.values() {
            match value {
                notes::Value::List(items) => values.extend(items.iter().cloned()),
                notes::Value::Text(text) => values.push(text.clone()),
            }
        }
        let haystack = format!("{}\n{}\n{}\n{}", entry.path, entry.title, values.join(" "), entry.body).to_lowercase();
        if !words.iter().all(|word| haystack.contains(word)) {
            continue;
        }
        let mut message = message_of(&entry, index, agent);
        message.snippet = Some(snippet(&entry.body, &words));
        notes.push(message);
    }
    // Path order first, then newest date first, as the Python's two stable sorts.
    notes.sort_by(|a, b| a.path.cmp(&b.path));
    notes.sort_by_key(|note| std::cmp::Reverse(note.date().unwrap_or_default()));
    Ok(notes)
}

pub(super) fn changes_notes(
    root: &Path,
    index: &marks::Index,
    agent: &str,
) -> Result<(Vec<Message>, Vec<String>), RpcError> {
    let mut problems = Vec::new();
    let seen_file = seen_file(agent)?;
    let head = match git::head(root) {
        Ok(head) => head,
        Err(error) => {
            problems.push(error);
            return Ok((Vec::new(), problems));
        }
    };
    let seen = match std::fs::read_to_string(&seen_file) {
        Ok(text) => text.trim().to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            problems.push(format!("cannot read {}: {error}", seen_file.display()));
            return Ok((Vec::new(), problems));
        }
    };
    if seen.is_empty() {
        remember(&seen_file, &head).map_err(unavailable)?;
        return Ok((Vec::new(), problems));
    }
    let added = match git::added_since(root, &seen) {
        Ok(paths) => paths,
        Err(error) => {
            problems.push(format!("the changes could not be read: {error}"));
            return Ok((Vec::new(), problems));
        }
    };
    let entries = list_entries(root)?;
    let by_path: std::collections::BTreeMap<&str, &Entry> =
        entries.iter().map(|entry| (entry.path.as_str(), entry)).collect();
    let mut notes = Vec::new();
    let mut listed = BTreeSet::new();
    for path in added {
        if message_path(&path).is_none() || !listed.insert(path.clone()) {
            continue;
        }
        match by_path.get(path.as_str()) {
            Some(entry) => notes.push(message_of(entry, index, agent)),
            None => problems.push(format!("{path} was added but is not in the vault now")),
        }
    }
    sort_newest_first(&mut notes);
    remember(&seen_file, &head).map_err(unavailable)?;
    let note_problems = note_problems(&notes);
    problems.extend(note_problems);
    Ok((notes, problems))
}

fn seen_file(agent: &str) -> Result<PathBuf, RpcError> {
    let paths = Paths::from_env()?;
    std::fs::create_dir_all(&paths.state)
        .map_err(|e| unavailable(format!("cannot make {}: {e}", paths.state.display())))?;
    let safe: String =
        agent.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '-' }).collect();
    Ok(paths.state.join(format!("vault-seen-{safe}")))
}

fn remember(path: &Path, head: &str) -> Result<(), String> {
    std::fs::write(path, format!("{head}\n")).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn note_problems(notes: &[Message]) -> Vec<String> {
    notes.iter().flat_map(|note| note.problems.iter().map(|problem| format!("{}: {problem}", note.path))).collect()
}

fn sort_newest_first(notes: &mut [Message]) {
    notes.sort_by(|a, b| {
        let left = (a.stamp().unwrap_or_default(), &a.path);
        let right = (b.stamp().unwrap_or_default(), &b.path);
        right.cmp(&left)
    });
}

pub(super) fn truncate(notes: &mut Vec<Message>, limit: usize) {
    notes.truncate(limit);
}

/// The note at a vault path, refusing anything that leaves the vault or is a link.
pub(super) fn read_one(root: &Path, path: &str) -> Result<Entry, RpcError> {
    inside(root, path)?;
    let full = root.join(path);
    let text = read_text(&full)?;
    let note = notes::parse_note(&text);
    let name = path.rsplit('/').next().unwrap_or(path);
    let title = notes::title_of(&note, name);
    Ok(Entry { path: path.to_string(), meta: note.meta, body: note.body, title })
}

pub(super) fn inside(root: &Path, path: &str) -> Result<PathBuf, RpcError> {
    if path.is_empty() || path.contains("..") || path.starts_with('/') || drive_path(path) {
        return Err(RpcError::new("usage", format!("{path} is not a path inside the vault."), ""));
    }
    let real_root =
        std::fs::canonicalize(root).map_err(|e| unavailable(format!("cannot read {}: {e}", root.display())))?;
    let full = root.join(path);
    let real = std::fs::canonicalize(&full)
        .map_err(|_| RpcError::new("not_found", format!("{path} is not in the vault."), ""))?;
    if !real.starts_with(&real_root) {
        return Err(RpcError::new("usage", format!("{path} is not a path inside the vault."), ""));
    }
    let metadata = std::fs::symlink_metadata(&full)
        .map_err(|_| RpcError::new("not_found", format!("{path} is not in the vault."), ""))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RpcError::new("not_found", format!("{path} is not in the vault."), ""));
    }
    Ok(full)
}

pub(super) fn drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

pub(super) fn clean_path(path: &str) -> String {
    path.replace('\\', "/")
}

pub(super) fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// `read` or `done` says the note as the Python tool does: its path, its front
/// matter, then its body.
pub fn read_text_of(note: &Message) -> String {
    let meta = note
        .meta
        .0
        .iter()
        .map(|(key, value)| match value {
            notes::Value::List(items) => format!("{key}: {}", items.join(", ")),
            notes::Value::Text(text) => format!("{key}: {text}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n{meta}\n\n{}", note.path, note.body)
}

/// The first 4000 characters of a body, and whether there was more.
pub(super) fn cut_body(body: &str) -> (String, bool) {
    let mut cut = body.chars();
    let shown: String = cut.by_ref().take(SHOWN).collect();
    (shown, cut.next().is_some())
}

/// A flat, 200-character window around the first word, with ellipses when it
/// does not reach the body's ends.
fn snippet(body: &str, words: &[String]) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = flat.chars().collect();
    let lower: String = flat.to_lowercase();
    let at = match words.first() {
        None => 0,
        Some(word) => {
            let byte = lower.find(word).unwrap_or(0);
            let character = lower[..byte].chars().count();
            character.saturating_sub(60)
        }
    };
    let piece: String = chars.iter().skip(at).take(200).collect();
    let mut out = String::new();
    if at > 0 {
        out.push('…');
    }
    out.push_str(&piece);
    if at + 200 < chars.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-listing-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }

    fn note(from: &str, to: &str, kind: &str, date: &str, title: &str) -> String {
        format!(
            "---\nfrom: {from}\nto: [{to}]\nkind: {kind}\ndate: {date}\ntitle: {title}\n---\n\n# {title}\n\nBody.\n"
        )
    }

    #[test]
    fn a_message_is_a_note_at_sender_to_recipient_and_disagreement_is_named() {
        assert_eq!(message_path("manager/to/opus/a.md"), Some(("manager".into(), "opus".into())));
        assert_eq!(message_path("manager/notes/a.md"), None);
        assert_eq!(message_path("manager/to/opus/deep/a.md"), None);
        assert_eq!(message_path("manager/to/opus/.md"), None);

        let root = scratch("messages");
        write(&root, "manager/to/opus/a.md", &note("manager", "opus", "request", "2026-11-01", "A"));
        write(&root, "manager/to/opus/b.md", &note("thinker", "opus", "request", "2026-11-02", "B"));
        let entries = list_entries(&root).unwrap();
        let index = marks::read_all(&root);
        let messages: Vec<Message> = entries
            .iter()
            .filter(|entry| message_path(&entry.path).is_some())
            .map(|entry| message_of(entry, &index, "opus"))
            .collect();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].problems, Vec::<String>::new());
        assert_eq!(messages[1].problems, ["says from: thinker but sits in manager/"]);
        let mut marked = messages[0].clone();
        marked.read_by = vec![marks::ReadBy { agent: "opus".into(), at: "2026-11-05T09:10:00+03:00".into() }];
        assert!(marked.line().contains("read 09:10"), "{}", marked.line());
        marked.done = Some(marks::Done {
            agent: "opus".into(),
            at: "2026-11-05T09:40:00+03:00".into(),
            why: Some("no reply".into()),
        });
        assert!(marked.line().contains("done 09:40: no reply"), "{}", marked.line());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn open_goes_by_the_kind_and_by_marks_but_a_request_stays_until_closed() {
        let root = scratch("open");
        write(&root, "manager/to/opus/request.md", &note("manager", "opus", "request", "2026-11-01", "Ask"));
        write(&root, "manager/to/opus/replied.md", &note("manager", "opus", "request", "2026-11-01", "Old ask"));
        write(&root, "manager/to/opus/decision.md", &note("manager", "opus", "decision", "2026-11-02", "Decide"));
        write(&root, "manager/to/opus/read-decision.md", &note("manager", "opus", "decision", "2026-11-03", "Read"));
        write(&root, "manager/to/opus/question.md", &note("manager", "opus", "question", "2026-11-04", "Question"));
        write(&root, "manager/to/opus/report.md", &note("manager", "opus", "report", "2026-11-05", "Report"));
        write(&root, "manager/to/all/all.md", &note("manager", "all", "question", "2026-11-06", "All"));
        write(
            &root,
            "opus/to/manager/reply.md",
            "---\nfrom: opus\nto: [manager]\nkind: answer\nre: manager/to/opus/replied.md\n---\n\nDone.\n",
        );
        write(
            &root,
            "opus/reads.jsonl",
            "{\"note\":\"manager/to/opus/read-decision.md\",\"read_at\":\"2026-11-05T09:10:00+03:00\"}\n\
             {\"note\":\"manager/to/opus/question.md\",\"done_at\":\"2026-11-05T09:40:00+03:00\",\"why\":\"no reply\"}\n",
        );
        let index = marks::read_all(&root);
        let (notes, problems) = open_messages(&root, &index, "opus").unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let paths: Vec<&str> = notes.iter().map(|note| note.path.as_str()).collect();
        assert_eq!(
            paths,
            ["manager/to/all/all.md", "manager/to/opus/request.md", "manager/to/opus/decision.md"],
            "requests and questions first, newest first, then the rest; a read decision and a done question leave"
        );
        assert!(notes[0].loud() && notes[1].loud() && !notes[2].loud());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn find_needs_every_word_newest_first_with_a_snippet_and_the_filters() {
        let root = scratch("find");
        write(
            &root,
            "myapp/tried/2026-10-13_json-string.md",
            "---\nkind: tried\nproject: myapp\ndate: 2026-10-13\n---\n\n# json as a string\n\n\
             web_request json given as a string got a 400.\n",
        );
        write(
            &root,
            "myapp/decisions/2026-10-17_git-vault.md",
            "---\nkind: decision\nproject: myapp\ndate: 2026-10-17\nstatus: open\n---\n\n\
             # One Git vault\n\nThe Owner chose a Git vault.\n",
        );
        write(&root, "thinker/ideas.md", "# Ideas\n\nA vault of ideas.\n");
        write(&root, "README.md", "# Test vault\n");
        let index = marks::read_all(&root);
        let query = |words: &str| Query { words: words.into(), limit: 20, ..Query::default() };
        let hits = find_notes(&root, &index, "opus", &query("vault")).unwrap();
        let paths: Vec<&str> = hits.iter().map(|hit| hit.path.as_str()).collect();
        assert_eq!(
            paths,
            ["myapp/decisions/2026-10-17_git-vault.md", "README.md", "thinker/ideas.md"],
            "newest date first, ties in path order"
        );
        let hit = &find_notes(&root, &index, "opus", &query("JSON 400")).unwrap()[0];
        assert_eq!(hit.title, "json as a string");
        assert!(hit.snippet.as_deref().unwrap().contains("got a 400"));

        let myapp = Query { project: Some("myapp".into()), limit: 20, ..Query::default() };
        assert_eq!(find_notes(&root, &index, "opus", &myapp).unwrap().len(), 2);
        let tried = Query { kind: Some("tried".into()), limit: 20, ..Query::default() };
        let kinds = find_notes(&root, &index, "opus", &tried).unwrap();
        assert_eq!(kinds.len(), 1);
        assert_eq!(kinds[0].kind().as_deref(), Some("tried"));
        let open = Query { kind: Some("decision".into()), status: Some("open".into()), limit: 20, ..Query::default() };
        assert_eq!(find_notes(&root, &index, "opus", &open).unwrap().len(), 1);
        let from = Query { from: Some("thinker".into()), limit: 20, ..Query::default() };
        assert_eq!(find_notes(&root, &index, "opus", &from).unwrap()[0].path, "thinker/ideas.md");
        assert!(find_notes(&root, &index, "opus", &query("nothing-like-this")).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn thread_is_oldest_first_both_ways_and_cut_body_keeps_the_head() {
        let root = scratch("thread");
        write(&root, "manager/to/opus/b.md", &note("manager", "opus", "report", "2026-11-02", "B"));
        write(&root, "opus/to/manager/a.md", &note("opus", "manager", "answer", "2026-11-01", "A"));
        write(&root, "manager/to/opus/c.md", &note("manager", "opus", "report", "2026-11-03", "C"));
        write(&root, "other/to/opus/x.md", &note("other", "opus", "report", "2026-11-01", "X"));
        let index = marks::read_all(&root);
        let (notes, _) = thread_notes(&root, &index, "opus", "manager").unwrap();
        let paths: Vec<&str> = notes.iter().map(|note| note.path.as_str()).collect();
        assert_eq!(paths, ["opus/to/manager/a.md", "manager/to/opus/b.md", "manager/to/opus/c.md"]);

        let (shown, cut) = cut_body(&"x".repeat(SHOWN));
        assert_eq!(shown.len(), SHOWN);
        assert!(!cut);
        let (shown, cut) = cut_body(&"x".repeat(SHOWN + 1));
        assert_eq!(shown.len(), SHOWN);
        assert!(cut);
        std::fs::remove_dir_all(root).unwrap();
    }
}
