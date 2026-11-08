//! What a turn is told, and what a worker carries.

use super::handoff::{go_good, refused_after_work};
use super::*;
use kompitum_core::task::brief::Plan;
use kompitum_core::task::checks::ChecksFile;
use kompitum_core::task::prompt::test_plan;

/// A repository for the direct tests: one commit, and its folder.
fn scratch_repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kom-prompt-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
    git(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "first"]);
    dir
}

#[test]
fn a_real_turn_carries_the_brief_the_limits_and_the_notes() {
    let s = Session::new();
    let repo = s.repo();
    std::fs::write(repo.join("AGENTS.md"), "# House rules\n\nUse tabs.\n").unwrap();
    std::fs::create_dir_all(repo.join(".kompitum")).unwrap();
    std::fs::write(repo.join(".kompitum").join("brief.md"), "# Repo notes\n\nCommit small.\n").unwrap();
    std::fs::write(
        repo.join(".kompitum").join("rules.md"),
        "## Keep the windows shut\ncovers: src/**\nwhy: a console over a game is the worst\n\nNo console window from a child.\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "notes"]);
    let kept = s.root.join("real-kept.md");
    let script = s.script("real", json!([go_good(Some(&kept))]));
    s.config(&script, "");
    let brief = "write made.txt\n";
    s.start("real-1", brief, &repo, &["--worker", "go", "--scope", "src/x.rs"]);
    let (exit, rows, summary) = s.wait(&["real-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");

    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.starts_with(brief.trim_end()), "{prompt}");
    let limits = prompt.find("# Limits on this task (added by Kompitum)").expect("the limits");
    assert!(prompt[limits..].contains("`src/x.rs`"), "the limits name the scope: {prompt}");
    let rule = prompt.find("No console window from a child.").expect("the rule");
    let agents = prompt.find("Use tabs.").expect("AGENTS.md's words");
    let notes = prompt.find("Commit small.").expect("the repo's notes");
    assert!(limits < rule && rule < agents && agents < notes, "{prompt}");
}

#[test]
fn the_notes_come_from_the_base_and_not_from_the_worker() {
    let s = Session::new();
    let repo = s.repo();
    std::fs::write(repo.join("AGENTS.md"), "# House rules\n\nThe committed words.\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "notes"]);
    let kept = s.root.join("base-kept.md");
    let mut first = vec![json!({"write": ["AGENTS.md", "# House rules\n\nThe rewrite.\n"]})];
    first.extend(refused_after_work());
    let script = s.script("base-notes", json!([first, go_good(Some(&kept))]));
    s.config(&script, "");
    s.start("base-1", "write made.txt", &repo, &[]);
    let (exit, rows, summary) = s.wait(&["base-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");

    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.contains("The committed words."), "{prompt}");
    assert!(!prompt.contains("The rewrite."), "AGENTS.md comes from the base, not the worktree: {prompt}");
}

#[test]
fn a_followup_on_a_new_worker_comes_after_the_limits() {
    let s = Session::new();
    let repo = s.repo();
    let kept = s.root.join("followup-kept.md");
    let script = s.script("followup", json!([refused_after_work(), go_good(Some(&kept))]));
    s.config(&script, "");
    s.json(&["workers", "--block", "go", "--until", "2099-01-01", "--reason", "wait"]);
    let brief = "write made.txt\n";
    s.start("follow-1", brief, &repo, &[]);
    wait_until("the task is stranded", || s.task("follow-1")["task"]["state"] == "stranded");
    let view = s.task("follow-1");
    let folder = PathBuf::from(view["folder"].as_str().unwrap());
    let followup = "check the edges first\n";
    std::fs::write(folder.join("turn-2.followup.md"), followup).unwrap();
    s.json(&["workers", "--clear", "go"]);
    let (exit, rows, summary) = s.wait(&["follow-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");

    let note = std::fs::read_to_string(folder.join("turn-2.note.md")).unwrap();
    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.starts_with(&note), "the note comes first: {prompt}");
    let brief_at = prompt.find(brief.trim_end()).expect("the brief");
    let limits = prompt.find("# Limits on this task (added by Kompitum)").expect("the limits");
    let asked = prompt.find("# What is asked now").expect("the follow-up heading");
    assert!(brief_at < limits && limits < asked, "{prompt}");
    assert!(prompt.ends_with(followup), "{prompt}");
}

#[test]
fn a_bad_checks_file_warns_and_the_turn_starts() {
    let s = Session::new();
    let repo = s.repo();
    std::fs::create_dir_all(repo.join(".kompitum")).unwrap();
    std::fs::write(repo.join(".kompitum").join("checks.json"), "this is not JSON\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "checks"]);
    let script = s.script("bad-checks", json!([go_good(None)]));
    s.config(&script, "");
    let started = s.start("bad-checks-1", "start anyway", &repo, &["--worker", "go"]);
    let warnings = started["warnings"].to_string();
    assert!(warnings.contains(".kompitum/checks.json is not valid JSON"), "{started}");
    let (exit, rows, summary) = s.wait(&["bad-checks-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "an unreadable checks file is an error in the report: {rows:?} {summary}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    assert_eq!(rows[0]["checks"], "failed: 1 error", "{rows:?}");
    let warnings = s.task("bad-checks-1")["task"]["warnings"].as_array().unwrap().clone();
    let word = ".kompitum/checks.json is not valid JSON: expected ident at line 1 column 2";
    assert_eq!(warnings.iter().filter(|warning| warning.as_str() == Some(word)).count(), 1, "{warnings:?}");
}

#[test]
fn the_start_warns_of_a_long_brief_a_phrase_and_no_check() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("warnings", json!([go_good(None)]));
    s.config(&script, "");
    let long = "a".repeat(19 * 1024 + 1);
    let phrased =
        s.start("long-1", &long, &repo, &["--worker", "go", "--require", "src/*.rs::a phrase", "--qa-rounds", "0"]);
    let warnings = phrased["warnings"].to_string();
    assert!(warnings.contains("past about 19 KB"), "{phrased}");
    assert!(warnings.contains("isn't one token"), "{phrased}");
    assert!(!warnings.contains("no check at all"), "a require is a check: {phrased}");

    let bare = s.start("bare-1", "short", &repo, &["--worker", "go"]);
    assert!(bare["warnings"].to_string().contains("this task has no check at all"), "{bare}");

    let tested = s.start("test-1", "short", &repo, &["--worker", "go", "--test", "cargo --version"]);
    assert!(!tested["warnings"].to_string().contains("no check at all"), "{tested}");

    let (exit, rows, summary) = s.wait(&["long-1", "bare-1", "test-1"], &["--timeout", "120"]);
    assert_eq!(exit, 1, "a require that finds nothing fails the checks: {rows:?} {summary}");
    let long = rows.iter().find(|row| row["task"] == "long-1").unwrap();
    assert_eq!(long["state"], "done", "{long}");
    assert_eq!(long["ok"], false, "{long}");
    assert_eq!(long["checks"], "failed: 1 required pattern missing", "{long}");
    let bare = rows.iter().find(|row| row["task"] == "bare-1").unwrap();
    assert_eq!((bare["state"].as_str(), bare["ok"].as_bool()), (Some("done"), Some(true)), "{bare}");
    let tested = rows.iter().find(|row| row["task"] == "test-1").unwrap();
    assert_eq!((tested["state"].as_str(), tested["ok"].as_bool()), (Some("done"), Some(true)), "{tested}");
}

#[test]
fn the_start_warns_of_a_stale_rule() {
    let s = Session::new();
    let repo = s.repo();
    std::fs::create_dir_all(repo.join(".kompitum")).unwrap();
    std::fs::write(
        repo.join(".kompitum").join("rules.md"),
        "## Old rule\ncovers: docs/*.md\n\nA rule for a folder that is gone.\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "rules"]);
    let script = s.script("stale", json!([go_good(None)]));
    s.config(&script, "");
    let started = s.start("stale-1", "short", &repo, &["--worker", "go", "--test", "cargo --version"]);
    let warnings: Vec<&str> = started["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(warnings.contains(&r#"rule "Old rule" covers nothing any more: docs/*.md"#), "{started}");
    assert!(!warnings.iter().any(|warning| warning.contains("no check at all")), "a test is a check: {started}");
    let (exit, rows, summary) = s.wait(&["stale-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
}

#[tokio::test]
async fn resuming_gives_the_followup_alone() {
    let dir = scratch_repo("resume");
    let task = kompitum_core::task::types::Task {
        repo: dir.display().to_string(),
        base: git(&dir, &["rev-parse", "HEAD"]).trim().to_string(),
        ..Default::default()
    };
    let followup = b"only this, byte for byte\n";
    let built = kompitum_core::task::prompt::turn_prompt(&task, "the brief", None, Some(followup), true).await.unwrap();
    assert_eq!(built.text, followup);
    assert!(built.warnings.is_empty(), "{:?}", built.warnings);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_plan_follows_the_scope_and_the_focused_command() {
    let spec = ChecksFile {
        test_cmd: Some("cargo test".into()),
        focused_cmd: Some("cargo test --test {files}".into()),
        focused_glob: vec!["tests/*.rs".into()],
        ..Default::default()
    };
    let scope = vec![
        "tests/a.rs".to_string(),
        "tests/a b.rs".to_string(),
        "tests/*.rs".to_string(),
        "src/lib.rs".to_string(),
        "tests/a.rs".to_string(),
    ];
    let plan = test_plan(&spec, &scope);
    assert_eq!(plan.worker, Plan::Focused);
    assert_eq!(plan.full.as_deref(), Some("cargo test"));
    assert_eq!(plan.focused.as_deref(), Some(r#"cargo test --test tests/a.rs "tests/a b.rs""#));

    let no_files = test_plan(&spec, &["tests/*.rs".to_string(), "src/lib.rs".to_string()]);
    assert_eq!(no_files.worker, Plan::NoFiles);
    assert_eq!(no_files.focused, None);
    assert_eq!(no_files.full.as_deref(), Some("cargo test"));

    let whole = ChecksFile { test_cmd: Some("cargo test".into()), ..Default::default() };
    let plan = test_plan(&whole, &scope);
    assert_eq!(plan.worker, Plan::Full);
    assert_eq!(plan.focused, None);
}
