//! The lead's memory: one Markdown note per memory in `<agent>/memory/`, the
//! duplicate check `remember` uses, and the packet the lead wakes with. The
//! notes are ordinary vault notes; only `remember` writes them.

use super::clock::{self, Moment};
use super::git;
use super::listing::{clean_path, inside, read_text, stem};
use super::notes::{self, Meta, Value};
use super::pen::valid_name;
use super::privacy;
use super::store::{Vault, Written, unavailable, with_lock, written};
use crate::rpc::RpcError;
use std::collections::BTreeMap;
use std::path::Path;

/// A memory title is one line, at most this many characters.
pub(super) const TITLE_MAX: usize = 120;
/// A memory body is at most this many characters.
pub(super) const BODY_MAX: usize = 4000;
/// The whole packet is budgeted at this many characters.
const BUDGET: usize = 12_000;

/// What `remember` is asked to write.
#[derive(Clone, Debug, Default)]
pub struct MemoryDraft {
    pub title: String,
    pub body: String,
    pub project: Option<String>,
    pub standing: bool,
    pub review_after: Option<String>,
    pub replace: Option<String>,
    pub new: bool,
}

/// One memory note, as the duplicate check and the packet read it.
#[derive(Clone, Debug)]
pub struct Memory {
    pub path: String,
    pub title: String,
    pub body: String,
    pub project: Option<String>,
    pub standing: bool,
    pub status: String,
    pub review_after: Option<String>,
    pub updated: String,
}

/// What `kom vault memory` answers.
#[derive(Clone, Debug)]
pub struct MemoryReport {
    pub agent: String,
    pub text: String,
    pub problems: Vec<String>,
}

impl MemoryReport {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"agent": self.agent, "packet": self.text, "problems": self.problems})
    }
}

impl Vault {
    /// Write one memory, then commit and push it.
    pub fn remember(&self, draft: MemoryDraft) -> Result<Written, RpcError> {
        with_lock(&self.root, "nothing was written", || {
            let pulled = git::pull_under_lock(&self.root, git::TIMEOUT);
            if !pulled.ok {
                return Err(unavailable(format!(
                    "The vault could not be pulled, so nothing was written: {}",
                    pulled.error
                )));
            }
            let path = write_memory(self, &draft, &Moment::now())?;
            let mut paths = vec![path.clone()];
            let marks_path = format!("{}/reads.jsonl", self.agent);
            if self.root.join(&marks_path).exists() {
                paths.push(marks_path);
            }
            let result =
                git::commit_and_push_under_lock(&self.root, &paths, &format!("{}: memory {}", self.agent, stem(&path)));
            Ok(written(&self.root, path, result, "Wrote"))
        })
    }

    /// Mark one of my memory notes superseded, then commit and push it.
    pub fn forget(&self, path: &str, why: &str) -> Result<Written, RpcError> {
        let path = own_memory(&self.agent, path)?;
        with_lock(&self.root, "nothing was changed", || {
            let pulled = git::pull_under_lock(&self.root, git::TIMEOUT);
            if !pulled.ok {
                return Err(unavailable(format!(
                    "The vault could not be pulled, so nothing was changed: {}",
                    pulled.error
                )));
            }
            forget_note(&self.root, &path, why, &Moment::now())?;
            let mut paths = vec![path.clone()];
            let marks_path = format!("{}/reads.jsonl", self.agent);
            if self.root.join(&marks_path).exists() {
                paths.push(marks_path);
            }
            let result =
                git::commit_and_push_under_lock(&self.root, &paths, &format!("{}: forget {}", self.agent, stem(&path)));
            Ok(written(&self.root, path, result, "Forgot"))
        })
    }

    /// The packet the lead wakes with, or the memories due for review.
    pub fn memory(&self, project: Option<&str>, due: bool) -> Result<MemoryReport, RpcError> {
        let behind = git::pull_for_reading(&self.root);
        let (memories, problems) = load_memories(&self.root, &self.agent)?;
        let today = Moment::now().date();
        let lead = (!behind.is_empty()).then_some(behind.as_str());
        let text = if due {
            due_packet(&self.agent, &memories, project, &today, lead, &problems)
        } else {
            packet(&self.agent, &memories, project, &today, lead, &problems)
        };
        Ok(MemoryReport { agent: self.agent.clone(), text, problems })
    }
}

/// Write one memory and answer its vault path. The duplicate check runs here,
/// so a caller can see its refusal before anything is written.
pub(super) fn write_memory(vault: &Vault, draft: &MemoryDraft, now: &Moment) -> Result<String, RpcError> {
    if !valid_name(&vault.agent) {
        return Err(RpcError::new(
            "usage",
            format!("\"{}\" is not an agent name: lower-case letters, digits and dashes.", vault.agent),
            "",
        ));
    }
    let title = clean_title(&draft.title)?;
    if draft.body.chars().count() > BODY_MAX {
        return Err(RpcError::new(
            "usage",
            format!("A memory body is at most {BODY_MAX} characters. Nothing was written."),
            "",
        ));
    }
    let project = draft.project.as_deref().map(str::trim).filter(|project| !project.is_empty()).map(str::to_string);
    let review_after = match draft.review_after.as_deref().map(str::trim).filter(|date| !date.is_empty()) {
        Some(date) if clock::valid_date(date) => Some(date.to_string()),
        Some(_) => {
            return Err(RpcError::new(
                "usage",
                "--review-after is a real date in YYYY-MM-DD form. Nothing was written.",
                "",
            ));
        }
        None => None,
    };
    let kinds = privacy::private_kinds(&format!("{title}\n{}", draft.body));
    if !kinds.is_empty() {
        return Err(RpcError::new(
            "refused",
            format!("The memory holds what looks like {}. Nothing was written.", kinds.join(", ")),
            "",
        ));
    }

    if let Some(raw) = &draft.replace {
        let path = own_memory(&vault.agent, raw)?;
        let full = inside(&vault.root, &path)?;
        let old = notes::parse_note(&read_text(&full)?);
        if old.meta.text("kind").as_deref() != Some(notes::MEMORY) {
            return Err(RpcError::new("refused", format!("{path} is not a memory note."), ""));
        }
        let date = old.meta.text("date").filter(|date| !date.is_empty()).unwrap_or_else(|| now.note_stamp());
        let meta = memory_meta(vault, &title, &project, &date, now, draft.standing, &review_after);
        std::fs::write(&full, notes::format_note(&meta, &draft.body).as_bytes())
            .map_err(|e| unavailable(format!("cannot write {}: {e}", full.display())))?;
        return Ok(path);
    }

    if !draft.new {
        let (memories, _) = load_memories(&vault.root, &vault.agent)?;
        let close: Vec<&Memory> = memories
            .iter()
            .filter(|memory| memory.status == "open" && close_titles(&title, &memory.title))
            .take(5)
            .collect();
        if !close.is_empty() {
            let list = close
                .iter()
                .map(|memory| format!("- {} ({})", memory.title, memory.path))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(RpcError::new(
                "refused",
                format!(
                    "Close to memories you already have:\n{list}\n\
                     Rewrite one with --replace <path>, or pass --new to keep both.\nNothing was written."
                ),
                "",
            ));
        }
    }

    let folder = vault.root.join(&vault.agent).join("memory");
    std::fs::create_dir_all(&folder).map_err(|e| unavailable(format!("cannot make {}: {e}", folder.display())))?;
    let base = notes::slug(&title);
    let mut name = format!("{base}.md");
    let mut number = 2;
    while folder.join(&name).exists() {
        name = format!("{base}-{number}.md");
        number += 1;
    }
    let date = now.note_stamp();
    let meta = memory_meta(vault, &title, &project, &date, now, draft.standing, &review_after);
    std::fs::write(folder.join(&name), notes::format_note(&meta, &draft.body).as_bytes())
        .map_err(|e| unavailable(format!("cannot write {}: {e}", folder.join(&name).display())))?;
    Ok(format!("{}/memory/{name}", vault.agent))
}

/// Mark one of my memory notes superseded, with why, in place.
pub(super) fn forget_note(root: &Path, path: &str, why: &str, now: &Moment) -> Result<(), RpcError> {
    let full = inside(root, path)?;
    let note = notes::parse_note(&read_text(&full)?);
    if note.meta.text("kind").as_deref() != Some(notes::MEMORY) {
        return Err(RpcError::new("refused", format!("{path} is not a memory note."), ""));
    }
    let why = why.trim();
    if why.is_empty() {
        return Err(RpcError::new("usage", "A forgotten memory needs a reason in --why.", ""));
    }
    let kinds = privacy::private_kinds(why);
    if !kinds.is_empty() {
        return Err(RpcError::new(
            "refused",
            format!("The reason holds what looks like {}. Nothing was changed.", kinds.join(", ")),
            "",
        ));
    }
    let mut meta = note.meta.clone();
    meta.set("status", Value::Text("superseded".into()));
    meta.set("why", Value::Text(why.to_string()));
    meta.set("updated", Value::Text(now.note_stamp()));
    std::fs::write(&full, notes::format_note(&meta, &note.body).as_bytes())
        .map_err(|e| unavailable(format!("cannot write {}: {e}", full.display())))
}

fn memory_meta(
    vault: &Vault,
    title: &str,
    project: &Option<String>,
    date: &str,
    now: &Moment,
    standing: bool,
    review_after: &Option<String>,
) -> Meta {
    let mut meta = Meta::default();
    meta.set("from", Value::Text(vault.agent.clone()));
    meta.set("kind", Value::Text(notes::MEMORY.into()));
    if let Some(project) = project {
        meta.set("project", Value::Text(project.clone()));
    }
    meta.set("title", Value::Text(title.to_string()));
    meta.set("date", Value::Text(date.to_string()));
    meta.set("updated", Value::Text(now.note_stamp()));
    if standing {
        meta.set("standing", Value::Text("true".into()));
    }
    if let Some(date) = review_after {
        meta.set("review_after", Value::Text(date.clone()));
    }
    meta.set("status", Value::Text("open".into()));
    meta
}

fn clean_title(raw: &str) -> Result<String, RpcError> {
    if raw.contains('\n') || raw.contains('\r') {
        return Err(RpcError::new("usage", "A memory title is one line. Nothing was written.", ""));
    }
    let title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(RpcError::new("usage", "A memory needs a title. Nothing was written.", ""));
    }
    if title.chars().count() > TITLE_MAX {
        return Err(RpcError::new(
            "usage",
            format!("A memory title is at most {TITLE_MAX} characters. Nothing was written."),
            "",
        ));
    }
    Ok(title)
}

/// The path of one of `agent`'s own memory notes: `<agent>/memory/<file>.md`.
fn own_memory(agent: &str, path: &str) -> Result<String, RpcError> {
    let path = clean_path(path);
    let Some((owner, file)) = memory_path(&path) else {
        return Err(RpcError::new(
            "usage",
            format!("{path} is not a memory path: it is <agent>/memory/<file>.md."),
            "",
        ));
    };
    if owner != agent {
        return Err(RpcError::new("refused", format!("{path} is not in {agent}/memory/, so it is not yours."), ""));
    }
    Ok(format!("{owner}/memory/{file}"))
}

fn memory_path(path: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 3 || parts[1] != "memory" {
        return None;
    }
    let file = parts[2];
    if parts[0].is_empty() || !file.ends_with(".md") || file.len() <= 3 {
        return None;
    }
    Some((parts[0].to_string(), file.to_string()))
}

/// Whether two titles name the same memory: at least two words of four or more
/// letters in common, and those at least half of the new title's such words.
pub(super) fn close_titles(new_title: &str, old_title: &str) -> bool {
    let new = title_words(new_title);
    if new.len() < 2 {
        return false;
    }
    let old = title_words(old_title);
    let shared = new.iter().filter(|word| old.contains(word)).count();
    shared >= 2 && shared * 2 >= new.len()
}

fn title_words(title: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    for ch in title.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            word.push(ch);
        } else if !word.is_empty() {
            if word.chars().count() >= 4 {
                words.push(std::mem::take(&mut word));
            } else {
                word.clear();
            }
        }
    }
    if word.chars().count() >= 4 {
        words.push(word);
    }
    words.sort();
    words.dedup();
    words
}

/// Every memory note in the agent's folder, with the notes too damaged to read.
fn load_memories(root: &Path, agent: &str) -> Result<(Vec<Memory>, Vec<String>), RpcError> {
    if !valid_name(agent) {
        return Err(RpcError::new(
            "usage",
            format!("\"{agent}\" is not an agent name: lower-case letters, digits and dashes."),
            "",
        ));
    }
    let folder = root.join(agent).join("memory");
    let listing = match std::fs::read_dir(&folder) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(error) => return Err(unavailable(format!("cannot read {}: {error}", folder.display()))),
    };
    let mut memories = Vec::new();
    let mut problems = Vec::new();
    for entry in listing {
        let entry = entry.map_err(|e| unavailable(format!("cannot read {}: {e}", folder.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") || name.len() <= 3 {
            continue;
        }
        let kind =
            entry.file_type().map_err(|e| unavailable(format!("cannot read {}: {e}", entry.path().display())))?;
        if !kind.is_file() {
            let path = format!("{agent}/memory/{name}");
            problems.push(format!("{path}: is not a regular file"));
            continue;
        }
        let path = format!("{agent}/memory/{name}");
        let text = match read_text(&entry.path()) {
            Ok(text) => text,
            Err(error) => {
                problems.push(format!("{path}: {}", error.message));
                continue;
            }
        };
        let note = notes::parse_note(&text);
        if note.meta.text("kind").as_deref() != Some(notes::MEMORY) {
            let kind = note.meta.text("kind").unwrap_or_else(|| "(none)".into());
            problems.push(format!("{path}: kind is {kind}, not memory"));
            continue;
        }
        let status = notes::status_of(&note.meta);
        if !matches!(status.as_str(), "open" | "superseded") {
            problems.push(format!("{path}: has an unknown memory status: {status}"));
            continue;
        }
        let review_after =
            note.meta.text("review_after").map(|date| date.trim().to_string()).filter(|date| !date.is_empty());
        if let Some(date) = &review_after
            && !clock::valid_date(date)
        {
            problems.push(format!("{path}: review_after is not YYYY-MM-DD: {date}"));
            continue;
        }
        memories.push(Memory {
            path,
            title: notes::title_of(&note, &name),
            body: note.body,
            project: note
                .meta
                .text("project")
                .map(|project| project.trim().to_string())
                .filter(|project| !project.is_empty()),
            standing: note.meta.text("standing").is_some_and(|standing| standing.eq_ignore_ascii_case("true")),
            status,
            review_after,
            updated: note.meta.text("updated").unwrap_or_default(),
        });
    }
    memories.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((memories, problems))
}

/// The packet: standing in full, then one index line per other memory.
pub(super) fn packet(
    agent: &str,
    memories: &[Memory],
    project: Option<&str>,
    today: &str,
    behind: Option<&str>,
    problems: &[String],
) -> String {
    let selected: Vec<&Memory> =
        memories.iter().filter(|memory| memory.status == "open" && included(memory, project)).collect();
    let standing = newest_first(selected.iter().copied().filter(|memory| memory.standing).collect());
    let indexed: Vec<&Memory> = selected.iter().copied().filter(|memory| !memory.standing).collect();
    let due = selected.iter().filter(|memory| is_due(memory, today)).count();

    let mut head = String::new();
    if let Some(line) = behind {
        head.push_str(line);
        head.push('\n');
    }
    head.push_str(&format!("# Your memory ({agent}, {} notes)\n", selected.len()));
    if !problems.is_empty() {
        head.push_str(&format!("\nProblems: {}\n", problems.join("; ")));
    }
    if !standing.is_empty() {
        head.push_str("\n## Standing\n");
        for memory in standing {
            head.push_str(&format!("\n### {}\n\n{}\n", memory.title, memory.body.trim_end()));
        }
    }
    let head_over = head.chars().count() > BUDGET;

    let groups = groups(indexed);
    let flat: Vec<(&str, &Memory)> =
        groups.iter().flat_map(|(name, items)| items.iter().map(move |memory| (name.as_str(), *memory))).collect();
    let total = flat.len();
    let mut text = head;
    let mut accepted = 0;
    let mut started = false;
    for (at, (group, memory)) in flat.iter().enumerate() {
        let mut candidate = text.clone();
        if !started {
            candidate.push_str("\n## Index\n");
        }
        if at == 0 || flat[at - 1].0 != *group {
            candidate.push_str(&format!("\n### {group}\n\n"));
        }
        candidate.push_str(&format!("- {} ({})\n", memory.title, memory.path));
        let left = total - at - 1;
        let close = closing(left, due, left > 0);
        if candidate.chars().count() + close.chars().count() <= BUDGET {
            text = candidate;
            started = true;
            accepted = at + 1;
        } else {
            break;
        }
    }
    let left = total - accepted;
    text.push_str(&closing(left, due, left > 0 || head_over));
    text.trim_end().to_string()
}

/// The memories due for review, oldest `review_after` first.
pub(super) fn due_packet(
    agent: &str,
    memories: &[Memory],
    project: Option<&str>,
    today: &str,
    behind: Option<&str>,
    problems: &[String],
) -> String {
    let mut due: Vec<&Memory> = memories
        .iter()
        .filter(|memory| memory.status == "open" && included(memory, project) && is_due(memory, today))
        .collect();
    due.sort_by(|a, b| {
        a.review_after
            .as_deref()
            .unwrap_or("")
            .cmp(b.review_after.as_deref().unwrap_or(""))
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut text = String::new();
    if let Some(line) = behind {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(&format!("# Your memory due for review ({agent}, {} notes)\n", due.len()));
    if !problems.is_empty() {
        text.push_str(&format!("\nProblems: {}\n", problems.join("; ")));
    }
    if due.is_empty() {
        text.push_str("\nNothing is due for review.\n");
    } else {
        text.push('\n');
        for memory in due {
            text.push_str(&format!(
                "- {} ({}) · review after {}\n",
                memory.title,
                memory.path,
                memory.review_after.as_deref().unwrap_or("")
            ));
        }
    }
    text.trim_end().to_string()
}

fn included(memory: &Memory, project: Option<&str>) -> bool {
    match project {
        None => true,
        Some(name) => memory.standing || memory.project.is_none() || memory.project.as_deref() == Some(name),
    }
}

fn is_due(memory: &Memory, today: &str) -> bool {
    memory.review_after.as_deref().is_some_and(|date| date <= today)
}

fn newest_first(mut memories: Vec<&Memory>) -> Vec<&Memory> {
    memories.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.path.cmp(&b.path)));
    memories
}

fn groups(indexed: Vec<&Memory>) -> Vec<(String, Vec<&Memory>)> {
    let mut groups = Vec::new();
    let general: Vec<&Memory> = indexed.iter().copied().filter(|memory| memory.project.is_none()).collect();
    if !general.is_empty() {
        groups.push(("General".to_string(), newest_first(general)));
    }
    let mut projects: BTreeMap<String, Vec<&Memory>> = BTreeMap::new();
    for memory in indexed {
        if let Some(project) = &memory.project {
            projects.entry(project.clone()).or_default().push(memory);
        }
    }
    let mut named: Vec<(String, Vec<&Memory>)> =
        projects.into_iter().map(|(project, items)| (project, newest_first(items))).collect();
    named.sort_by(|a, b| {
        let left = a.1.first().map(|memory| memory.updated.as_str()).unwrap_or("");
        let right = b.1.first().map(|memory| memory.updated.as_str()).unwrap_or("");
        right.cmp(left).then_with(|| a.0.cmp(&b.0))
    });
    groups.extend(named);
    groups
}

fn closing(left: usize, due: usize, over: bool) -> String {
    if left == 0 && due == 0 && !over {
        return String::new();
    }
    let mut text = String::from("\n");
    if left > 0 {
        text.push_str(&format!("{left} more: kom vault find --kind memory\n"));
    }
    if due > 0 {
        text.push_str(&format!("{due} due for review: kom vault memory --due\n"));
    }
    if over {
        text.push_str("Over budget: merge or forget memories (kom vault remember --replace, kom vault forget).\n");
    }
    text
}

#[cfg(test)]
mod tests;
