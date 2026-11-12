//! `kom task review`: the whole review of a task in one read.

use crate::api::{TaskReviewParams, TaskReviewResult};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::checks::{Side, Source, diff_path};
use crate::task::types::{ChangedFile, CheckReport, Task, Usage, cut, gaps_and_report};
use crate::task::worker;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The review's size when `max_bytes` is not given.
pub const DEFAULT_MAX_BYTES: usize = 20_000;
/// How much of the report's Gaps section a review shows.
const GAPS_CHARS: usize = 3_000;
/// How much of the report's body a review shows.
const REPORT_CHARS: usize = 2_500;
/// What a review says when the worktree is gone and no kept patch was found.
const GONE: &str = "worktree gone: the patch is no longer available";
/// What a report without a Gaps section is shown as.
const NO_GAPS: &str = "The report has no Gaps section.";
/// The tail line of a review cut to its size.
const TRUNCATED: &str = "TRUNCATED: {bytes} bytes left out; raise max_bytes for more";

/// Everything a review is made from, so `render` stays pure.
#[derive(Clone, Debug, Default)]
pub struct ReviewInput {
    pub task: Task,
    /// The built-ins and the person's workers, for the labels a record stores as keys.
    pub workers: Vec<worker::Worker>,
    pub patch: String,
    pub full: bool,
    pub max_bytes: usize,
    /// The moment the review is made, for a task still running.
    pub now_ms: u64,
    /// The worktree is gone and no kept patch was found.
    pub gone: bool,
    /// Where a patch too big to inline is written, when `full` asked for it.
    pub patch_path: PathBuf,
}

/// The text of a review, and what became of the patch.
#[derive(Clone, Debug, Default)]
pub struct Rendered {
    pub text: String,
    pub patch_file: Option<String>,
    pub truncated: bool,
}

pub async fn review(core: &Arc<Core>, p: TaskReviewParams) -> Result<TaskReviewResult, RpcError> {
    let task = core.tasks.snapshot(&p.task).ok_or_else(|| super::not_found(&p.task))?;
    let state = task.state;
    let mut gone = false;
    let mut patch = String::new();
    let worktree = task.worktree.as_deref().filter(|path| Path::new(path).exists());
    match worktree {
        Some(worktree) if task.shared => {
            let files = super::workshop::files_of(worktree, &task.scope).await?;
            let out = super::workshop::patch(worktree, &files, &[]).await?;
            patch = String::from_utf8_lossy(&out).into_owned();
        }
        Some(worktree) => {
            super::git::git(worktree, &["add", "-N", "."]).await?;
            let out = super::git::git(worktree, &["diff", "--no-renames", &task.base]).await?;
            patch = String::from_utf8_lossy(&out.stdout).into_owned();
        }
        None => {
            let path = core.paths.state.join("patches").join(format!("{}.patch", task.name));
            match tokio::task::spawn_blocking(move || std::fs::read(&path)).await {
                Ok(Ok(bytes)) => patch = String::from_utf8_lossy(&bytes).into_owned(),
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => gone = true,
                Ok(Err(e)) => return Err(super::unavailable(format!("cannot read the kept patch: {e}"))),
                Err(e) => return Err(super::unavailable(format!("cannot read the kept patch: {e}"))),
            }
        }
    }
    let max_bytes = p.max_bytes.map(|value| usize::try_from(value).unwrap_or(usize::MAX)).unwrap_or(DEFAULT_MAX_BYTES);
    let workers = crate::task::workers::all(&core.tasks.workers());
    let input = ReviewInput {
        task,
        workers,
        patch: patch.clone(),
        full: p.full,
        max_bytes,
        now_ms: now_ms(),
        gone,
        patch_path: core.paths.state.join("patches").join(format!("{}.diff", p.task)),
    };
    let rendered = render(&input);
    if rendered.patch_file.is_some() {
        let path = input.patch_path.clone();
        let bytes = patch.into_bytes();
        let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            crate::paths::write_atomic(&path, &bytes)
        })
        .await;
        match written {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(super::unavailable(format!("cannot write {}: {e}", input.patch_path.display())));
            }
            Err(e) => return Err(super::unavailable(format!("cannot write the patch: {e}"))),
        }
    }
    Ok(TaskReviewResult {
        task: input.task.name.clone(),
        state,
        text: rendered.text,
        patch_file: rendered.patch_file,
        truncated: rendered.truncated,
    })
}

/// The whole review's text, from a task and its patch.
pub fn render(input: &ReviewInput) -> Rendered {
    let task = &input.task;
    let max_bytes = input.max_bytes;
    let last = task.turns.last();
    let report = task.checks.last();
    let mut usage = Usage::default();
    for turn in &task.turns {
        usage.add(&turn.usage);
    }

    let mut fixed = String::new();
    fixed.push_str(&format!("# Review: {}\n", task.name));
    let seconds = task.started_ms.map(|start| {
        let end = task.ended_ms.unwrap_or(input.now_ms.max(start));
        end.saturating_sub(start) as f64 / 1000.0
    });
    let parts = [
        task.state.word().to_string(),
        last.map(|turn| worker::label_of(&input.workers, &turn.worker)).unwrap_or_else(|| "unknown".into()),
        duration_text(seconds),
        format!("${:.3}", usage.cost_usd),
        format!("steps {}", usage.steps),
        format!("by {}", crate::task::table::by_word(task.by.as_deref())),
    ];
    fixed.push_str(&parts.join(" · "));
    fixed.push('\n');
    for handoff in &task.handoffs {
        fixed.push_str(handoff);
        fixed.push('\n');
    }
    if input.gone {
        fixed.push_str(GONE);
        fixed.push('\n');
    }
    if let Some(error) = &task.error {
        fixed.push_str(&format!("Error: {error}\n"));
    }
    fixed.push('\n');

    let checks = report.map(CheckReport::body).unwrap_or_else(|| crate::task::types::NO_CHECKS.into());
    fixed.push_str(&section("Checks", &checks));
    let qa = qa_body(task);
    if !qa.is_empty() {
        fixed.push_str(&section("QA rounds", &qa));
    }
    if !task.changed.is_empty() {
        fixed.push_str(&section("Scope", &scope_body(&task.scope, &task.changed)));
    }
    if let Some(report) = report.filter(|report| !report.warn_hits.is_empty()) {
        fixed.push_str(&section("Warns", &warns_body(report)));
    }
    let (gaps, rest) = gaps_and_report(last.map(|turn| turn.text.as_str()).unwrap_or_default());
    let gaps = gaps.map_or_else(|| NO_GAPS.to_string(), |gaps| cut(&gaps, GAPS_CHARS));
    fixed.push_str(&section("Gaps", &gaps));

    let mut blocks: Vec<String> = Vec::new();
    if !rest.is_empty() {
        let body = if rest.chars().count() > REPORT_CHARS {
            format!("{}\n\n[… cut]", cut(&rest, REPORT_CHARS).trim_end())
        } else {
            rest
        };
        blocks.push(section("Report", &body));
    }
    let outside: BTreeSet<String> =
        task.changed.iter().filter(|file| !file.in_scope).map(|file| file.path.replace('\\', "/")).collect();
    let (new_flags, old_flags) = flags(report);
    let flagged = flagged_files(&input.patch, &new_flags, &old_flags, &outside);
    if !flagged.is_empty() {
        blocks.push(section("Flagged hunks", &flagged_body(&flagged)));
    }
    let mut patch_file = None;
    let mut full_text = fixed.clone() + &blocks.concat();
    if input.full && !input.patch.is_empty() {
        let inline = section("Patch", &fenced(&input.patch));
        let with_patch = full_text.clone() + &inline;
        if with_patch.len() > max_bytes {
            let path = input.patch_path.display().to_string();
            fixed.push_str(&section(
                "Patch",
                &format!(
                    "The patch is {} bytes and does not fit in max_bytes: read it from {path}, in parts if it is large.",
                    input.patch.len()
                ),
            ));
            patch_file = Some(path);
            full_text = fixed.clone() + &blocks.concat();
        } else {
            blocks.push(inline);
            full_text = with_patch;
        }
    }

    if full_text.len() <= max_bytes {
        return Rendered { text: full_text, patch_file, truncated: false };
    }
    let text = truncate(&full_text, &fixed, &blocks, max_bytes);
    Rendered { text, patch_file, truncated: true }
}

fn section(title: &str, body: &str) -> String {
    format!("## {title}\n{body}\n\n")
}

fn duration_text(seconds: Option<f64>) -> String {
    match seconds {
        None => "duration unknown".into(),
        Some(seconds) if seconds < 60.0 => format!("{:.0} s", seconds.max(0.0)),
        Some(seconds) => format!("{:.0} min", (seconds / 60.0).max(0.0)),
    }
}

fn qa_body(task: &Task) -> String {
    let lines: Vec<String> = task
        .turns
        .iter()
        .filter(|turn| turn.role == "qa")
        .filter_map(|turn| task.checks.iter().find(|report| report.turn.saturating_add(1) == turn.n))
        .enumerate()
        .map(|(index, report)| format!("- round {}: {}", index + 1, report.words()))
        .collect();
    lines.join("\n")
}

fn scope_body(globs: &[String], files: &[ChangedFile]) -> String {
    let head = if globs.is_empty() {
        "Scope: whole repository".to_string()
    } else {
        let shown: Vec<String> = globs.iter().map(|glob| format!("`{glob}`")).collect();
        format!("Scope: {}", shown.join(", "))
    };
    let mut lines =
        vec![head, String::new(), "| file | + | - | |".to_string(), "| --- | --- | --- | --- |".to_string()];
    for file in files {
        let name = file.path.replace('|', "\\|");
        let mark = if file.in_scope { "in scope" } else { "**OUTSIDE scope**" };
        lines.push(format!("| {name} | {} | {} | {mark} |", file.added, file.removed));
    }
    lines.join("\n")
}

fn warns_body(report: &CheckReport) -> String {
    report
        .warn_hits
        .iter()
        .map(|hit| format!("- {} {}:{} `{}` ({})", hit.rule, hit.file, hit.line, hit.text, source_word(hit.source)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn source_word(source: Source) -> &'static str {
    match source {
        Source::Builtin => "builtin",
        Source::File => "file",
        Source::Call => "call",
    }
}

/// The lines a report points at, on each side of the diff, per file.
fn flags(report: Option<&CheckReport>) -> (BTreeMap<String, BTreeSet<u32>>, BTreeMap<String, BTreeSet<u32>>) {
    let mut new = BTreeMap::new();
    let mut old = BTreeMap::new();
    let Some(report) = report else { return (new, old) };
    let add = |map: &mut BTreeMap<String, BTreeSet<u32>>, path: &str, line: u32| {
        map.entry(path.replace('\\', "/")).or_default().insert(line);
    };
    for hit in &report.forbid_hits {
        add(&mut new, &hit.file, hit.line);
    }
    for hit in &report.warn_hits {
        match hit.side {
            Side::Added => add(&mut new, &hit.file, hit.line),
            Side::Removed => add(&mut old, &hit.file, hit.line),
        }
    }
    (new, old)
}

#[derive(Debug, Default)]
struct Hunk {
    old: (u32, u32),
    new: (u32, u32),
    lines: Vec<String>,
}

#[derive(Debug, Default)]
struct PatchFile {
    old: Option<String>,
    new: Option<String>,
    header: Vec<String>,
    hunks: Vec<Hunk>,
}

impl PatchFile {
    fn path(&self) -> Option<&str> {
        self.new.as_deref().or(self.old.as_deref())
    }
}

fn hunk_header(raw: &str) -> Option<((u32, u32), (u32, u32))> {
    let rest = raw.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let (new, rest) = rest.strip_prefix('+')?.split_once(' ')?;
    if !rest.starts_with("@@") {
        return None;
    }
    Some((range(old)?, range(new)?))
}

fn range(value: &str) -> Option<(u32, u32)> {
    let (start, count) = match value.split_once(',') {
        Some((start, count)) => (start, Some(count)),
        None => (value, None),
    };
    let start = start.parse().ok()?;
    let count = match count {
        Some(count) => count.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

/// The files a patch names, their `---`/`+++` paths, and their hunks in order.
fn split_patch(patch: &str) -> Vec<PatchFile> {
    let mut files: Vec<PatchFile> = Vec::new();
    let mut in_hunk = false;
    for raw in patch.lines() {
        if raw.starts_with("diff --git ") {
            files.push(PatchFile { header: vec![raw.to_string()], ..Default::default() });
            in_hunk = false;
            continue;
        }
        let Some(current) = files.last_mut() else { continue };
        if let Some((old, new)) = hunk_header(raw) {
            current.hunks.push(Hunk { old, new, lines: vec![raw.to_string()] });
            in_hunk = true;
            continue;
        }
        if in_hunk {
            if let Some(hunk) = current.hunks.last_mut() {
                hunk.lines.push(raw.to_string());
            }
            continue;
        }
        current.header.push(raw.to_string());
        if let Some(value) = raw.strip_prefix("+++ ") {
            current.new = diff_path(value, "b/");
        } else if let Some(value) = raw.strip_prefix("--- ") {
            current.old = diff_path(value, "a/");
        }
    }
    files
}

/// A hunk holds a flagged line, on either side.
fn holds(start: u32, count: u32, numbers: &BTreeSet<u32>) -> bool {
    if count == 0 {
        return numbers.contains(&start);
    }
    let end = start.saturating_add(count);
    numbers.iter().any(|number| start <= *number && *number < end)
}

/// Each file's header once, then only the hunks a flag points into.
fn flagged_files(
    patch: &str,
    new_flags: &BTreeMap<String, BTreeSet<u32>>,
    old_flags: &BTreeMap<String, BTreeSet<u32>>,
    outside: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for file in split_patch(patch) {
        let Some(path) = file.path().map(str::to_string) else { continue };
        let new = new_flags.get(&path);
        let old = old_flags.get(&path);
        let selected: Vec<&Hunk> = file
            .hunks
            .iter()
            .filter(|hunk| {
                outside.contains(&path)
                    || new.is_some_and(|numbers| holds(hunk.new.0, hunk.new.1, numbers))
                    || old.is_some_and(|numbers| holds(hunk.old.0, hunk.old.1, numbers))
            })
            .collect();
        if selected.is_empty() {
            continue;
        }
        let mut lines = file.header;
        for hunk in selected {
            lines.extend(hunk.lines.iter().cloned());
        }
        out.push((path, lines.join("\n")));
    }
    out
}

fn fenced(text: &str) -> String {
    let mut body = text.to_string();
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    format!("```diff\n{body}```")
}

fn flagged_body(flagged: &[(String, String)]) -> String {
    if flagged.is_empty() {
        return "None flagged.".into();
    }
    flagged.iter().map(|(_, text)| fenced(text)).collect::<Vec<_>>().join("\n\n")
}

/// At most `limit` bytes of `text`, cut on a character.
fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    text[..end].to_string()
}

/// Cut the tail blocks to fit, then end with the TRUNCATED line.
fn truncate(full_text: &str, fixed: &str, blocks: &[String], max_bytes: usize) -> String {
    let reserve = TRUNCATED.replace("{bytes}", "999999999999").len() + 2;
    let budget = max_bytes.saturating_sub(reserve);
    let mut out = fixed.to_string();
    if out.len() > budget {
        out = clip(&out, budget);
    } else {
        for block in blocks {
            let room = budget.saturating_sub(out.len());
            if room == 0 {
                break;
            }
            if block.len() <= room {
                out.push_str(block);
            } else {
                out.push_str(&clip(block, room));
                break;
            }
        }
    }
    let out = out.trim_end_matches('\n');
    let left = full_text.len().saturating_sub(out.len());
    let line = TRUNCATED.replace("{bytes}", &left.to_string());
    if out.is_empty() { line } else { format!("{out}\n{line}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::checks::{Hit, Source, WarnHit};
    use crate::task::types::{TestRun, Turn};

    fn report(ok: bool, failed: &str) -> CheckReport {
        CheckReport {
            turn: 1,
            ok,
            seconds: 1.0,
            test: Some(TestRun { ok, failed: vec![failed.into()], ..Default::default() }),
            ..Default::default()
        }
    }

    fn task() -> Task {
        Task {
            name: "fix-1".into(),
            state: crate::task::types::TaskState::Done,
            started_ms: Some(1_000),
            ended_ms: Some(4_500),
            scope: vec!["src/**".into()],
            handoffs: vec!["Command Code ran out of credits, so opencode Go went on.".into()],
            turns: vec![Turn {
                n: 1,
                worker: "go".into(),
                text: "Did the work.\n\n## Gaps\nThe docs are old.\n\n## After\nAfter the Gaps.\n".into(),
                usage: Usage { steps: 3, cost_usd: 0.25, ..Default::default() },
                ..Default::default()
            }],
            changed: vec![
                ChangedFile { path: "src/a.rs".into(), added: 2, removed: 0, in_scope: true },
                ChangedFile { path: "outside.txt".into(), added: 1, removed: 1, in_scope: false },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn render_puts_its_sections_in_order() {
        let mut task = task();
        task.turns.push(Turn {
            n: 2,
            worker: "go".into(),
            role: "qa".into(),
            text: task.turns[0].text.clone(),
            ..Default::default()
        });
        task.qa_used = 1;
        task.checks = vec![report(false, "a_test")];
        task.checks[0].turn = 1;
        task.checks[0].warn_hits = vec![WarnHit {
            rule: "no-unwrap".into(),
            side: Side::Added,
            file: "src/a.rs".into(),
            line: 2,
            text: "unwrap()".into(),
            source: Source::Builtin,
        }];
        task.checks[0].forbid_hits =
            vec![Hit { pattern: "secret".into(), file: "src/a.rs".into(), line: 2, text: "secret".into() }];
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,2 @@\n one\n+x\n";
        let input = ReviewInput { task, patch: patch.into(), max_bytes: 20_000, ..Default::default() };
        let text = render(&input).text;
        let order = ["## Checks", "## QA rounds", "## Scope", "## Warns", "## Gaps", "## Report", "## Flagged hunks"];
        let mut at = 0;
        for title in order {
            let found = text[at..].find(title).unwrap_or_else(|| panic!("{title} missing from {text}"));
            at += found + title.len();
        }
        assert!(text.starts_with("# Review: fix-1\ndone · opencode Go · 4 s · $0.250 · steps 3 · by —\n"), "{text}");
        assert!(text.contains("Command Code ran out of credits, so opencode Go went on."), "{text}");
        assert!(text.contains("## Checks\nFAILED · 1.0 s\n- failed: a_test"), "a failed report's words show: {text}");
        assert!(text.contains("- round 1: failed: 1 test failed"), "{text}");
        assert!(text.contains("| src/a.rs | 2 | 0 | in scope |"), "{text}");
        assert!(text.contains("| outside.txt | 1 | 1 | **OUTSIDE scope** |"), "{text}");
        assert!(text.contains("- no-unwrap src/a.rs:2 `unwrap()` (builtin)"), "{text}");
        assert!(text.contains("## Gaps\nThe docs are old."), "{text}");
        assert!(text.contains("## Report\nDid the work."), "{text}");
        assert!(text.contains("After the Gaps."), "{text}");
        assert!(text.contains("+x"), "only the flagged hunk is shown: {text}");
    }

    #[test]
    fn a_report_without_gaps_says_so() {
        let mut task = task();
        task.turns[0].text = "All done.\n".into();
        let input = ReviewInput { task, max_bytes: 20_000, ..Default::default() };
        let text = render(&input).text;
        assert!(text.contains("## Gaps\nThe report has no Gaps section.\n\n"), "{text}");
        assert!(text.contains("## Report\nAll done."), "{text}");
    }

    #[test]
    fn a_big_review_is_cut_on_a_character_and_ends_with_its_marker() {
        let mut task = task();
        task.turns[0].text = "é".repeat(2_000);
        let input = ReviewInput { task, max_bytes: 700, ..Default::default() };
        let rendered = render(&input);
        assert!(rendered.truncated, "{}", rendered.text);
        assert!(rendered.text.ends_with("bytes left out; raise max_bytes for more"), "{}", rendered.text);
        assert!(!rendered.text.contains('\u{fffd}'), "a character was split: {}", rendered.text);
        assert!(rendered.text.len() <= 700, "{} > 700", rendered.text.len());
    }

    #[test]
    fn a_cut_report_ends_with_the_pythons_marker() {
        let mut task = task();
        task.turns[0].text = "r".repeat(REPORT_CHARS + 100);
        let input = ReviewInput { task, max_bytes: 20_000, ..Default::default() };
        let text = render(&input).text;
        let expected = format!("## Report\n{}\n\n[… cut]\n\n", "r".repeat(REPORT_CHARS));
        assert!(text.contains(&expected), "{text}");
    }

    #[test]
    fn a_quoted_path_keeps_its_escapes() {
        let patch = r#"diff --git "a/tab\tname.rs" "b/tab\tname.rs"
--- "a/tab\tname.rs"
+++ "b/tab\tname.rs"
@@ -1 +1 @@
-a
+b
"#;
        let files = split_patch(patch);
        assert_eq!(files[0].path(), Some(r"tab\tname.rs"));
    }

    #[test]
    fn flagged_hunks_show_only_what_was_flagged() {
        let mut task = task();
        task.changed.truncate(1);
        task.checks = vec![CheckReport {
            ok: false,
            forbid_hits: vec![Hit { pattern: "x".into(), file: "src/a.rs".into(), line: 12, text: "x".into() }],
            ..Default::default()
        }];
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,2 @@\n one\n+two\n@@ -10,1 +11,2 @@\n ten\n+x\n\
                     diff --git a/other.rs b/other.rs\n--- a/other.rs\n+++ b/other.rs\n@@ -1,1 +1,2 @@\n a\n+b\n";
        let input = ReviewInput { task, patch: patch.into(), max_bytes: 20_000, ..Default::default() };
        let text = render(&input).text;
        assert!(text.contains("@@ -10,1 +11,2 @@\n ten\n+x"), "{text}");
        assert!(!text.contains("+two"), "the other hunk of the file is left out: {text}");
        assert!(!text.contains("+b"), "an unflagged file is left out: {text}");
    }

    #[test]
    fn a_patch_that_fits_alone_goes_to_a_file_when_the_review_does_not() {
        let mut task = task();
        task.turns[0].text = "r".repeat(900);
        task.checks.clear();
        let patch = format!(
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,2 @@\n one\n+{}\n",
            "p".repeat(400)
        );
        assert!(patch.len() < 1_500);
        let path = PathBuf::from("C:/state/patches/fix-1.diff");
        let input =
            ReviewInput { task, patch, full: true, max_bytes: 1_500, patch_path: path.clone(), ..Default::default() };
        let rendered = render(&input);
        assert_eq!(rendered.patch_file.as_deref(), path.to_str());
        assert!(!rendered.truncated, "the rest fits: {}", rendered.text);
        assert!(rendered.text.contains(path.to_str().unwrap()), "{}", rendered.text);
        let gaps = rendered.text.find("## Gaps").unwrap();
        let patch_at = rendered.text.find("## Patch").unwrap();
        let report = rendered.text.find("## Report").unwrap();
        assert!(gaps < patch_at && patch_at < report, "the pointer is fixed, after the Gaps: {}", rendered.text);
        assert!(!rendered.text.contains(&"p".repeat(400)), "the patch stayed out of the text");
    }

    #[test]
    fn a_truncated_review_still_names_the_patch_file() {
        let mut task = task();
        task.turns[0].text = "r".repeat(6_000);
        task.checks.clear();
        let patch = format!(
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,2 @@\n one\n+{}\n",
            "p".repeat(200)
        );
        let path = PathBuf::from("C:/state/patches/fix-2.diff");
        let input =
            ReviewInput { task, patch, full: true, max_bytes: 1_400, patch_path: path.clone(), ..Default::default() };
        let rendered = render(&input);
        assert!(rendered.truncated, "{}", rendered.text);
        assert_eq!(rendered.patch_file.as_deref(), path.to_str());
        assert!(rendered.text.contains(path.to_str().unwrap()), "the pointer is not cut: {}", rendered.text);
        assert!(rendered.text.ends_with("bytes left out; raise max_bytes for more"), "{}", rendered.text);
    }

    #[test]
    fn qa_rounds_follow_the_turns_that_ran_qa() {
        let turn = |n: u32, role: &str| Turn { n, worker: "go".into(), role: role.into(), ..Default::default() };
        let failed = |turn: u32, names: &[&str]| CheckReport {
            turn,
            seconds: 1.0,
            test: Some(TestRun { failed: names.iter().map(|name| name.to_string()).collect(), ..Default::default() }),
            ..Default::default()
        };
        let render_text = |turns: Vec<Turn>, checks: Vec<CheckReport>| {
            let mut task = task();
            task.qa_used = 0; // a redo sets it back
            task.turns = turns;
            task.checks = checks;
            render(&ReviewInput { task, max_bytes: 20_000, ..Default::default() }).text
        };
        let first = failed(1, &["a_test"]);
        let second = failed(2, &["b_test", "c_test"]);
        let text = render_text(vec![turn(1, "worker"), turn(2, "qa"), turn(3, "redo")], vec![first.clone(), second]);
        let qa = qa_section(&text);
        assert_eq!(qa.matches("- round ").count(), 1, "{qa}");
        assert!(qa.contains("- round 1: failed: 1 test failed"), "{qa}");
        assert!(!qa.contains("2 tests failed"), "the report no qa turn followed is left out: {qa}");

        let third = failed(3, &["d_test", "e_test", "f_test"]);
        let text = render_text(
            vec![turn(1, "worker"), turn(2, "qa"), turn(3, "redo"), turn(4, "qa")],
            vec![first, failed(2, &["b_test", "c_test"]), third],
        );
        let qa = qa_section(&text);
        assert_eq!(qa.matches("- round ").count(), 2, "{qa}");
        assert!(qa.contains("- round 1: failed: 1 test failed"), "{qa}");
        assert!(qa.contains("- round 2: failed: 3 tests failed"), "{qa}");
        assert!(!qa.contains("2 tests failed"), "the redo turn is not a qa round: {qa}");

        let text = render_text(vec![turn(1, "worker"), turn(2, "redo")], vec![failed(1, &["a_test"])]);
        assert!(!text.contains("## QA rounds"), "a redo alone shows no rounds: {text}");
    }

    fn qa_section(text: &str) -> &str {
        let Some(start) = text.find("## QA rounds") else { return "" };
        let rest = &text[start..];
        match rest.find("\n## ") {
            Some(end) => &rest[..end],
            None => rest,
        }
    }
}
