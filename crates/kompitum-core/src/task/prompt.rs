//! What a turn is told: the hand-off note, the brief, this repository's rules, and any follow-up.

use crate::rpc::RpcError;
use crate::task::brief::{self, Footer, Plan, TestPlan};
use crate::task::checking;
use crate::task::checks::{ChecksFile, Workshop, glob_match};
use crate::task::git;
use crate::task::types::{Asked, Task};
use crate::task::worker::Worker;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A turn's prompt, and what is worth saying about how it was built.
#[derive(Clone, Debug, Default)]
pub struct TurnPrompt {
    pub text: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Where a turn's prompt file lives: its task folder and turn number.
pub fn prompt_path(folder: &Path, n: u32) -> PathBuf {
    folder.join(format!("turn-{n}.prompt.md"))
}

/// This turn's prompt file as the worker sees it, when the core's state is known: forward slashes.
fn this_turn_prompt_file(task: &Task) -> Option<String> {
    let paths = crate::paths::Paths::from_env().ok()?;
    let n = u32::try_from(task.turns.len()).unwrap_or(u32::MAX - 1).saturating_add(1);
    let path = prompt_path(&crate::task::task_folder(&paths.state, &task.name), n);
    Some(path.display().to_string().replace('\\', "/"))
}

const FOLLOWUP: &str = "\n\n---\n\n# What is asked now\n\nAn earlier turn of this task has ended, and its changes are in \
                        your folder. This comes on top of the brief above:\n\n";

/// The session a follow-up continues: the last turn's, when this worker ran it and reported one.
pub fn resume_session(task: &Task, worker: &Worker, has_followup: bool) -> Option<String> {
    if !has_followup {
        return None;
    }
    let last = task.turns.last()?;
    if last.worker != worker.key() {
        return None;
    }
    last.session.clone()
}

/// A file as committed at the base, decoded lossily; a read that fails is a warning.
async fn base_text(repo: &str, base: &str, path: &str, warnings: &mut Vec<String>) -> Option<String> {
    match git::show_file(repo, base, path).await {
        Ok(Some(bytes)) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Ok(None) => None,
        Err(error) => {
            warnings.push(format!("{path} could not be read from the task's base: {}", error.message));
            None
        }
    }
}

/// The scope's plain paths that look like test files, in order and without duplicates.
fn focused_files(scope: &[String], globs: &[String]) -> Vec<String> {
    let mut files = Vec::new();
    for entry in scope {
        if entry.chars().any(|ch| matches!(ch, '*' | '?' | '[')) {
            continue;
        }
        let path = entry.replace('\\', "/");
        if files.contains(&path) {
            continue;
        }
        if globs.iter().any(|glob| glob_match(&path, glob)) {
            files.push(path);
        }
    }
    files
}

/// What the worker runs itself and what the server runs after it, as `suite.py` plans it.
pub fn test_plan(spec: &ChecksFile, scope: &[String]) -> TestPlan {
    let full = spec.test_cmd.clone().filter(|cmd| !cmd.trim().is_empty());
    // `suite.py` strips the template before it uses it.
    let Some(template) = spec.focused_cmd.as_deref().map(str::trim).filter(|cmd| !cmd.is_empty()) else {
        return TestPlan { worker: Plan::Full, full, focused: None };
    };
    let files = focused_files(scope, &spec.focused_glob);
    if files.is_empty() {
        return TestPlan { worker: Plan::NoFiles, full, focused: None };
    }
    let quoted: Vec<String> =
        files.iter().map(|file| if file.contains(' ') { format!("\"{file}\"") } else { file.clone() }).collect();
    let focused = Some(template.replace("{files}", &quoted.join(" ")));
    TestPlan { worker: Plan::Focused, full, focused }
}

/// What this turn's worker reads. Resuming, the follow-up alone: its session has the rest.
pub async fn turn_prompt(
    task: &Task,
    brief: &str,
    note: Option<&[u8]>,
    followup: Option<&[u8]>,
    resuming: bool,
) -> Result<TurnPrompt, RpcError> {
    if resuming {
        return Ok(TurnPrompt { text: followup.unwrap_or_default().to_vec(), warnings: Vec::new() });
    }
    let mut warnings = Vec::new();
    let (mut spec, problems) = match checking::spec(&task.repo, &task.base, &task.asked).await {
        Ok((spec, problems)) => (spec, problems),
        Err(error) => (ChecksFile::default(), vec![format!("the checks could not be read: {}", error.message)]),
    };
    // The footer names the test that will really run.
    spec.test_cmd = checking::test_cmd(task, &spec);
    warnings.extend(problems);
    let notes = base_text(&task.repo, &task.base, ".kompitum/brief.md", &mut warnings).await;
    let rules = match base_text(&task.repo, &task.base, ".kompitum/rules.md", &mut warnings).await {
        Some(text) => {
            let (rules, problems) = brief::parse_rules(&text);
            warnings.extend(problems);
            brief::rules_for_scope(&rules, &task.scope).into_iter().cloned().collect()
        }
        None => Vec::new(),
    };
    let agents = base_text(&task.repo, &task.base, "AGENTS.md", &mut warnings).await;
    let languages =
        match git::git_for(&task.repo, &["ls-tree", "--name-only", &task.base], Duration::from_secs(30)).await {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                let names: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
                brief::languages(&names)
            }
            Err(error) => {
                warnings.push(format!("the root of the task's base could not be listed: {}", error.message));
                Vec::new()
            }
        };
    let plan = test_plan(&spec, &task.scope);
    let footer = brief::footer(&Footer {
        scope: task.scope.clone(),
        plan,
        batch: false,
        languages,
        windows: cfg!(windows),
        notes,
        rules,
        agents,
        shared: task.shared,
        prompt_file: this_turn_prompt_file(task),
    });
    let mut text = note.unwrap_or_default().to_vec();
    text.extend_from_slice(brief.trim_end().as_bytes());
    if !task.parts.is_empty()
        && let Ok(paths) = crate::paths::Paths::from_env()
    {
        let folder = crate::task::task_folder(&paths.state, &task.name);
        text.extend_from_slice(b"\n\n");
        text.extend_from_slice(crate::task::parts::coordinator_section(&folder, &task.parts).as_bytes());
    }
    text.extend_from_slice(footer.as_bytes());
    if let Some(followup) = followup {
        text.extend_from_slice(FOLLOWUP.as_bytes());
        text.extend_from_slice(followup);
    }
    Ok(TurnPrompt { text, warnings })
}

/// The path-like tokens a brief names: relative, forward slashes, no globs or URLs.
fn named_paths(brief: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let split =
        |ch: char| ch.is_whitespace() || matches!(ch, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';');
    for token in brief.split(split) {
        let path = token.trim_end_matches(['.', ':', '!']);
        if !path.contains('/')
            || path.starts_with('/')
            || path.starts_with('-')
            || path.contains(':')
            || path.split('/').any(|part| part == "..")
            || path.chars().any(|ch| matches!(ch, '*' | '?' | '{' | '}' | '\\' | '$' | '='))
        {
            continue;
        }
        if !paths.iter().any(|named| named.as_str() == path) {
            paths.push(path.to_string());
        }
    }
    paths
}

/// The paths the brief names that exist in the checkout but git ignores: the worktree won't have them.
async fn ignored_paths(repo: &str, brief: &str) -> Vec<String> {
    let paths = named_paths(brief);
    let existing: Vec<&str> =
        paths.iter().filter(|path| Path::new(repo).join(path).exists()).map(String::as_str).collect();
    if existing.is_empty() {
        return Vec::new();
    }
    let mut args = vec!["check-ignore", "--"];
    args.extend(existing.iter().copied());
    let Ok(out) = git::git_raw(repo, &args).await else { return Vec::new() };
    let ignored = String::from_utf8_lossy(&out.stdout);
    existing
        .into_iter()
        .filter(|path| ignored.lines().any(|line| line == *path))
        .map(|path| {
            format!(
                "the brief names {path}, which git ignores, so the worktree won't have it: put what it needs in the brief"
            )
        })
        .collect()
}

/// What is worth saying before a worker starts, and the committed checks' one-at-a-time rule.
/// These warn and never refuse.
pub async fn start_warnings(
    repo: &str,
    base: &str,
    brief: &str,
    _scope: &[String],
    asked: &Asked,
) -> (Vec<String>, bool, Option<Workshop>) {
    let mut warnings = Vec::new();
    let spec = match checking::spec(repo, base, asked).await {
        Ok((spec, problems)) => {
            warnings.extend(problems);
            spec
        }
        Err(error) => {
            warnings.push(format!("the checks could not be read: {}", error.message));
            ChecksFile::default()
        }
    };
    let one_at_a_time = spec.one_at_a_time;
    let workshop = spec.workshop.clone();
    let has_any_check = spec.test_cmd.as_deref().is_some_and(|cmd| !cmd.trim().is_empty())
        || !spec.forbid.is_empty()
        || !spec.require.is_empty();
    warnings.extend(brief::start_warnings(brief, &spec.require, has_any_check));
    warnings.extend(ignored_paths(repo, brief).await);
    let Some(rules) = base_text(repo, base, ".kompitum/rules.md", &mut warnings).await else {
        return (warnings, one_at_a_time, workshop);
    };
    let (rules, problems) = brief::parse_rules(&rules);
    warnings.extend(problems);
    match git::git_for(repo, &["ls-tree", "-r", "--name-only", base], Duration::from_secs(30)).await {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let files: Vec<String> = text.lines().filter(|line| !line.is_empty()).map(str::to_string).collect();
            warnings.extend(brief::stale_rules(&rules, &files));
        }
        Err(error) => warnings.push(format!("the files of the task's base could not be listed: {}", error.message)),
    }
    (warnings, one_at_a_time, workshop)
}
