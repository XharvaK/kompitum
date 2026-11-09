//! Read and done marks: `<agent>/reads.jsonl`, append-only, one JSON object to
//! a line. A mark lives in the reader's own folder, never in another agent's
//! note, and it never rings anyone: only `*/to/*` paths ring.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Folders the vault walk never enters.
pub const SKIP: [&str; 4] = [".git", ".obsidian", ".trash", "node_modules"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReadBy {
    pub agent: String,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Done {
    pub agent: String,
    pub at: String,
    pub why: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub agent: String,
    pub at: String,
    pub done: bool,
    pub why: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Index {
    by_note: BTreeMap<String, Vec<Mark>>,
    /// A damaged mark file is said in words, never skipped in silence.
    pub problems: Vec<String>,
}

impl Index {
    pub fn marks(&self, note: &str) -> &[Mark] {
        self.by_note.get(note).map(Vec::as_slice).unwrap_or_default()
    }

    /// Every agent that read it (a done mark counts as read too), first mark each.
    pub fn read_by(&self, note: &str) -> Vec<ReadBy> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for mark in self.marks(note) {
            if seen.insert(mark.agent.clone()) {
                out.push(ReadBy { agent: mark.agent.clone(), at: mark.at.clone() });
            }
        }
        out
    }

    /// The reader's own done mark, else the first one anybody wrote.
    pub fn done(&self, note: &str, reader: &str) -> Option<Done> {
        let marks = self.marks(note);
        let chosen =
            marks.iter().find(|mark| mark.done && mark.agent == reader).or_else(|| marks.iter().find(|mark| mark.done));
        chosen.map(|mark| Done { agent: mark.agent.clone(), at: mark.at.clone(), why: mark.why.clone() })
    }

    pub fn done_by(&self, note: &str, agent: &str) -> bool {
        self.marks(note).iter().any(|mark| mark.done && mark.agent == agent)
    }

    /// Any mark at all: a done mark already says the note was read.
    pub fn marked_by(&self, note: &str, agent: &str) -> bool {
        self.marks(note).iter().any(|mark| mark.agent == agent)
    }
}

/// Every agent's marks. A file that is not readable is a problem for the command
/// that asked; the notes still list.
pub fn read_all(root: &Path) -> Index {
    let mut index = Index::default();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            index.problems.push(format!("cannot read {}: {error}", root.display()));
            return index;
        }
    };
    let mut agents: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| !SKIP.contains(&entry.file_name().to_string_lossy().as_ref()))
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
        .collect();
    agents.sort_by(|a, b| a.0.cmp(&b.0));
    for (agent, dir) in agents {
        let path = dir.join("reads.jsonl");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                index.problems.push(format!("cannot read {}: {error}", path.display()));
                continue;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                index.problems.push(format!("{} line {} is not a mark", path.display(), number + 1));
                continue;
            };
            let Some(note) = value["note"].as_str().filter(|note| !note.is_empty()) else {
                index.problems.push(format!("{} line {} names no note", path.display(), number + 1));
                continue;
            };
            if let Some(at) = value["read_at"].as_str() {
                index.by_note.entry(note.to_string()).or_default().push(Mark {
                    agent: agent.clone(),
                    at: at.to_string(),
                    done: false,
                    why: None,
                });
            } else if let Some(at) = value["done_at"].as_str() {
                index.by_note.entry(note.to_string()).or_default().push(Mark {
                    agent: agent.clone(),
                    at: at.to_string(),
                    done: true,
                    why: value["why"].as_str().map(str::to_string),
                });
            } else {
                index.problems.push(format!("{} line {} has neither read_at nor done_at", path.display(), number + 1));
            }
        }
    }
    index
}

/// Append one mark to the reader's file, making its folder when needed. The
/// returned path is vault-relative, in forward slashes, for Git.
pub fn append(root: &Path, agent: &str, mark: &Value) -> Result<String, String> {
    if agent.is_empty() || agent == "." || agent == ".." || agent.contains(['/', '\\', ':']) {
        return Err(format!("{agent:?} cannot name an agent's folder in the vault"));
    }
    let dir = root.join(agent);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let path = dir.join("reads.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let line = serde_json::to_string(mark).map_err(|e| format!("cannot write a mark: {e}"))?;
    file.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(format!("{agent}/reads.jsonl"))
}

pub fn read_mark(note: &str, at: String) -> Value {
    json!({"note": note, "read_at": at})
}

pub fn done_mark(note: &str, at: String, why: Option<String>) -> Value {
    match why {
        Some(why) => json!({"note": note, "done_at": at, "why": why}),
        None => json!({"note": note, "done_at": at}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-marks-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_read_and_done_marks_and_names_a_damaged_line() {
        let root = scratch("read");
        std::fs::create_dir_all(root.join("opus")).unwrap();
        std::fs::write(
            root.join("opus").join("reads.jsonl"),
            "{\"note\":\"manager/to/opus/a.md\",\"read_at\":\"2026-11-05T09:10:00+03:00\"}\n\
             not json\n\
             {\"note\":\"manager/to/opus/b.md\",\"done_at\":\"2026-11-05T09:40:00+03:00\",\"why\":\"done\"}\n",
        )
        .unwrap();
        let index = read_all(&root);
        assert_eq!(
            index.read_by("manager/to/opus/a.md"),
            [ReadBy { agent: "opus".into(), at: "2026-11-05T09:10:00+03:00".into() }]
        );
        assert_eq!(
            index.done("manager/to/opus/b.md", "opus"),
            Some(Done { agent: "opus".into(), at: "2026-11-05T09:40:00+03:00".into(), why: Some("done".into()) })
        );
        assert!(index.marked_by("manager/to/opus/b.md", "opus"));
        assert_eq!(index.problems.len(), 1);
        assert!(index.problems[0].contains("line 2"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn appends_one_line_and_keeps_other_marks() {
        let root = scratch("append");
        let path = append(&root, "opus", &read_mark("a.md", "2026-11-05T09:10:00+03:00".into())).unwrap();
        assert_eq!(path, "opus/reads.jsonl");
        assert!(append(&root, "../evil", &read_mark("a.md", "2026-11-05T09:10:00+03:00".into())).is_err());
        append(&root, "opus", &done_mark("a.md", "2026-11-05T09:40:00+03:00".into(), Some("no reply".into()))).unwrap();
        let text = std::fs::read_to_string(root.join(&path)).unwrap();
        assert_eq!(text.lines().count(), 2);
        let index = read_all(&root);
        assert!(index.marked_by("a.md", "opus"));
        assert!(index.done_by("a.md", "opus"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn append_refuses_a_symlinked_agent_folder_and_writes_nothing_outside() {
        let root = scratch("link-append");
        let outside = scratch("link-append-out");
        std::os::unix::fs::symlink(&outside, root.join("opus")).unwrap();
        let error = append(&root, "opus", &read_mark("a.md", "2026-11-05T09:10:00+03:00".into())).unwrap_err();
        assert!(error.contains("refused:"), "{error}");
        assert!(error.contains("opus/reads.jsonl"), "{error}");
        assert!(error.contains("is a link out of the vault"), "{error}");
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none(), "wrote outside: {outside:?}");
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
}
