//! A batch task's parts: what a part is, the checks and unions a start makes,
//! the grouping of changed files, and the coordinator's prompt section.

use crate::rpc::RpcError;
use crate::task::decide::in_scope;
use crate::task::types::Task;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The longest part brief `task.start` accepts.
pub const MAX_PART_BRIEF: usize = 60_000;

/// One part of a batch: its own brief, files and require lines.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct TaskPart {
    /// The part's name, using the task-name rules.
    pub name: String,
    /// The part's whole brief text, at most 60000 characters.
    pub brief: String,
    /// Its paths, at least one.
    pub scope: Vec<String>,
    /// glob::regex lines, as the task's own.
    #[serde(default)]
    pub require: Vec<String>,
}

/// What became of one part.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PartState {
    #[default]
    Open,
    Accepted,
    Discarded,
}

impl PartState {
    pub fn word(self) -> &'static str {
        match self {
            PartState::Open => "open",
            PartState::Accepted => "accepted",
            PartState::Discarded => "discarded",
        }
    }
}

/// One require line as the review shows it.
#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
pub struct PartRequire {
    /// The glob::regex line.
    pub entry: String,
    /// The last checks found no matching line.
    pub missing: bool,
}

/// One part as the review shows it.
#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct PartReview {
    pub name: String,
    pub state: PartState,
    /// The changed files that belong to it, first part wins.
    pub files: Vec<String>,
    pub require: Vec<PartRequire>,
}

/// Why a batch can't start: a bad name, a repeated name, no scope, or no brief.
pub fn check(parts: &[TaskPart]) -> Result<(), RpcError> {
    let mut names: Vec<&str> = Vec::new();
    for part in parts {
        crate::addr::check_name(&part.name)
            .map_err(|e| RpcError::invalid_params(format!("part {}: {}", part.name, e.message)))?;
        if names.contains(&part.name.as_str()) {
            return Err(RpcError::invalid_params(format!(
                "two parts are named {:?}: each part needs its own name",
                part.name
            )));
        }
        names.push(&part.name);
        if part.scope.is_empty() {
            return Err(RpcError::invalid_params(format!(
                "part {:?} has no scope: give it at least one path",
                part.name
            )));
        }
        if part.brief.trim().is_empty() {
            return Err(RpcError::invalid_params(format!("part {:?} has an empty brief", part.name)));
        }
        let length = part.brief.chars().count();
        if length > MAX_PART_BRIEF {
            return Err(RpcError::invalid_params(format!(
                "part {:?}'s brief is {length} characters; the limit is {MAX_PART_BRIEF}",
                part.name
            )));
        }
    }
    Ok(())
}

/// The task's own scope, then each part's, in order and without repeats.
pub fn union_scope(scope: &[String], parts: &[TaskPart]) -> Vec<String> {
    let mut union: Vec<String> = Vec::new();
    for entry in scope.iter().chain(parts.iter().flat_map(|part| part.scope.iter())) {
        if !union.contains(entry) {
            union.push(entry.clone());
        }
    }
    union
}

/// The task's own require lines, then each part's, in order.
pub fn union_require(require: &[String], parts: &[TaskPart]) -> Vec<String> {
    let mut union = require.to_vec();
    for part in parts {
        union.extend(part.require.iter().cloned());
    }
    union
}

/// Each changed file under the first part whose scope matches it, then the files
/// no part claims that match the task's own scope.
pub fn group(parts: &[TaskPart], task_scope: &[String], changed: &[String]) -> (Vec<Vec<String>>, Vec<String>) {
    let mut groups = vec![Vec::new(); parts.len()];
    let mut shared = Vec::new();
    for path in changed {
        let claimed = parts.iter().position(|part| in_scope(path, &part.scope));
        match claimed {
            Some(index) => groups[index].push(path.clone()),
            None if in_scope(path, task_scope) => shared.push(path.clone()),
            None => {}
        }
    }
    (groups, shared)
}

/// The part's state; a record from before states keeps every part open.
pub fn state(task: &Task, index: usize) -> PartState {
    task.part_states.get(index).copied().unwrap_or_default()
}

/// Set one part's state, filling in the states a record from before states lacks.
pub fn set_state(task: &mut Task, index: usize, value: PartState) {
    task.part_states.resize(task.parts.len(), PartState::Open);
    task.part_states[index] = value;
}

/// The index of the part with this name.
pub fn find(parts: &[TaskPart], name: &str) -> Option<usize> {
    parts.iter().position(|part| part.name == name)
}

/// How many parts are still open.
pub fn open_count(task: &Task) -> usize {
    (0..task.parts.len()).filter(|index| state(task, *index) == PartState::Open).count()
}

/// The changed files by part, and the ones no part claims that match the task's scope.
pub fn review(task: &Task) -> (Vec<PartReview>, Vec<String>) {
    let changed: Vec<String> = task.changed.iter().map(|file| file.path.clone()).collect();
    let (groups, shared) = group(&task.parts, &task.scope, &changed);
    let missing = task.checks.last().map(|report| report.require_missing.as_slice()).unwrap_or(&[]);
    let parts = task
        .parts
        .iter()
        .zip(groups)
        .enumerate()
        .map(|(index, (part, files))| PartReview {
            name: part.name.clone(),
            state: state(task, index),
            files,
            require: part
                .require
                .iter()
                .map(|entry| PartRequire { entry: entry.clone(), missing: missing.contains(entry) })
                .collect(),
        })
        .collect();
    (parts, shared)
}

/// The part brief files, beside `brief.md`; a Windows line ending becomes LF.
pub fn write_briefs(folder: &Path, parts: &[TaskPart]) -> Result<(), RpcError> {
    for part in parts {
        let path = folder.join(format!("part-{}.md", part.name));
        let text = part.brief.replace("\r\n", "\n").replace('\r', "\n");
        crate::paths::write_atomic(&path, text.as_bytes())
            .map_err(|e| RpcError::new("unavailable", format!("cannot write {}: {e}", path.display()), ""))?;
    }
    Ok(())
}

const COORDINATOR: &str = "\
# This task is a batch

You are the coordinator. Each part below is its own job with its own brief and files. Give each part to a subagent
with this prompt, filled in:

\"Read <brief path> and do everything it says. Change only these files: <scope>. Others are working in this folder at
the same time: touch nothing else, and never run git add, checkout, restore, reset, clean or stash. End with the
report your brief asks for: What changed, Acceptance, Gaps.\"

- Run parts whose files don't overlap at the same time. Parts that share a file run one after another, in the order
  listed.
- Keep only each subagent's report. Read the code yourself only to judge a report or settle a gap.
- When a report shows the part unfinished, or a check it names failing, start a new subagent for that part with the
  same prompt plus what is missing. At most two tries per part.
- When every part is done, run the tests the briefs name, then end with one report: a \"## Part <name>\" section per
  part (its What changed, Acceptance and Gaps), then your own \"## Gaps\".
- If you have no subagent tool, do the parts yourself, one after another, in the order listed.

The parts:
";

/// The coordinator's section: the shared rules and one line per part.
pub fn coordinator_section(folder: &Path, parts: &[TaskPart]) -> String {
    let mut text = String::from(COORDINATOR);
    for part in parts {
        let path = folder.join(format!("part-{}.md", part.name)).display().to_string().replace('\\', "/");
        let scope: Vec<String> = part.scope.iter().map(|glob| format!("`{glob}`")).collect();
        text.push_str(&format!(" - **{}**: brief `{path}`, files {}\n", part.name, scope.join(", ")));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str, scope: &[&str], brief: &str) -> TaskPart {
        TaskPart {
            name: name.into(),
            brief: brief.into(),
            scope: scope.iter().map(|glob| (*glob).to_string()).collect(),
            require: Vec::new(),
        }
    }

    #[test]
    fn a_batch_refuses_a_repeated_name_no_scope_and_an_empty_brief() {
        assert_eq!(check(&[part("a", &["a.txt"], "do a")]).unwrap(), ());
        let repeated = check(&[part("a", &["a.txt"], "one"), part("a", &["b.txt"], "two")]).unwrap_err();
        assert_eq!(repeated.kind, "invalid_params");
        assert!(repeated.message.contains("two parts are named"));
        let no_scope = check(&[part("a", &[], "do a")]).unwrap_err();
        assert_eq!(no_scope.kind, "invalid_params");
        assert!(no_scope.message.contains("no scope"));
        let no_brief = check(&[part("a", &["a.txt"], " \n")]).unwrap_err();
        assert_eq!(no_brief.kind, "invalid_params");
        assert!(no_brief.message.contains("empty brief"));
        let long = check(&[part("a", &["a.txt"], &"x".repeat(MAX_PART_BRIEF + 1))]).unwrap_err();
        assert!(long.message.contains("the limit is 60000"));
        let bad_name = check(&[part("A", &["a.txt"], "do a")]).unwrap_err();
        assert_eq!(bad_name.kind, "invalid_params");
    }

    #[test]
    fn the_scope_and_require_are_the_tasks_then_each_parts() {
        let mut first = part("a", &["src/**", "a.txt"], "do a");
        first.require = vec!["a.txt::done".into()];
        let second = part("b", &["a.txt", "b.txt"], "do b");
        let scope = union_scope(&["README.md".into(), "src/**".into()], &[first.clone(), second]);
        assert_eq!(scope, ["README.md", "src/**", "a.txt", "b.txt"], "in order, without repeats");
        assert_eq!(union_require(&["task::ok".into()], &[first]), ["task::ok", "a.txt::done"]);
    }

    #[test]
    fn a_file_belongs_to_the_first_part_that_matches_and_the_rest_are_shared() {
        let parts = [part("a", &["src/**"], "do a"), part("b", &["src/lib.rs", "b.txt"], "do b")];
        let changed: Vec<String> = ["src/lib.rs", "src/main.rs", "b.txt", "docs/x.md", "outside.txt"]
            .iter()
            .map(|file| (*file).to_string())
            .collect();
        let (groups, shared) = group(&parts, &["docs/**".into()], &changed);
        assert_eq!(groups[0], ["src/lib.rs", "src/main.rs"], "the first match wins");
        assert_eq!(groups[1], ["b.txt"]);
        assert_eq!(shared, ["docs/x.md"], "unclaimed only when the task's own scope takes it");
    }

    #[test]
    fn the_review_shows_each_parts_state_files_and_require_lines() {
        let mut task = Task {
            scope: vec!["shared/**".into()],
            parts: vec![part("a", &["a.txt"], "do a"), part("b", &["b.txt"], "do b")],
            ..Default::default()
        };
        task.parts[0].require = vec!["a.txt::done".into()];
        task.parts[1].require = vec!["b.txt::done".into()];
        task.changed = ["a.txt", "b.txt", "shared/x.md"]
            .iter()
            .map(|path| crate::task::types::ChangedFile { path: (*path).into(), in_scope: true, ..Default::default() })
            .collect();
        task.checks.push(crate::task::types::CheckReport {
            require_missing: vec!["b.txt::done".into()],
            ..Default::default()
        });
        set_state(&mut task, 0, PartState::Accepted);
        let (parts, shared) = review(&task);
        assert_eq!(parts[0].state, PartState::Accepted);
        assert_eq!(parts[0].files, ["a.txt"]);
        assert_eq!(parts[0].require[0], PartRequire { entry: "a.txt::done".into(), missing: false });
        assert_eq!(parts[1].require[0], PartRequire { entry: "b.txt::done".into(), missing: true });
        assert_eq!(shared, ["shared/x.md"]);
    }

    #[test]
    fn a_parts_state_defaults_to_open_and_is_kept_in_order() {
        let mut task =
            Task { parts: vec![part("a", &["a.txt"], "do a"), part("b", &["b.txt"], "do b")], ..Default::default() };
        assert_eq!(open_count(&task), 2);
        assert_eq!(state(&task, 1), PartState::Open, "a record from before states is open");
        set_state(&mut task, 1, PartState::Discarded);
        assert_eq!(task.part_states, [PartState::Open, PartState::Discarded]);
        assert_eq!(open_count(&task), 1);
        assert_eq!(find(&task.parts, "b"), Some(1));
        assert_eq!(find(&task.parts, "c"), None);
    }

    #[test]
    fn the_coordinators_section_is_word_for_word_and_names_each_part_file() {
        let parts = [part("a", &["a.txt", "src/**"], "do a"), part("b", &["b.txt"], "do b")];
        let text = coordinator_section(Path::new("C:/state/tasks/fix-1"), &parts);
        let expected = "\
# This task is a batch

You are the coordinator. Each part below is its own job with its own brief and files. Give each part to a subagent
with this prompt, filled in:

\"Read <brief path> and do everything it says. Change only these files: <scope>. Others are working in this folder at
the same time: touch nothing else, and never run git add, checkout, restore, reset, clean or stash. End with the
report your brief asks for: What changed, Acceptance, Gaps.\"

- Run parts whose files don't overlap at the same time. Parts that share a file run one after another, in the order
  listed.
- Keep only each subagent's report. Read the code yourself only to judge a report or settle a gap.
- When a report shows the part unfinished, or a check it names failing, start a new subagent for that part with the
  same prompt plus what is missing. At most two tries per part.
- When every part is done, run the tests the briefs name, then end with one report: a \"## Part <name>\" section per
  part (its What changed, Acceptance and Gaps), then your own \"## Gaps\".
- If you have no subagent tool, do the parts yourself, one after another, in the order listed.

The parts:
 - **a**: brief `C:/state/tasks/fix-1/part-a.md`, files `a.txt`, `src/**`
 - **b**: brief `C:/state/tasks/fix-1/part-b.md`, files `b.txt`
";
        assert_eq!(text, expected);
    }

    #[test]
    fn part_briefs_are_written_beside_the_task_brief_with_lf_line_endings() {
        let folder = std::env::temp_dir().join(format!("kom-parts-{}-{}", std::process::id(), unique()));
        std::fs::create_dir_all(&folder).unwrap();
        write_briefs(&folder, &[part("a", &["a.txt"], "one\r\ntwo\rthree\n")]).unwrap();
        assert_eq!(std::fs::read_to_string(folder.join("part-a.md")).unwrap(), "one\ntwo\nthree\n");
        assert!(!folder.join("part-b.md").exists());
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn unique() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }
}
