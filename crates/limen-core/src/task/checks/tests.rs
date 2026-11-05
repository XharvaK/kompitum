use super::*;

fn diff(path: &str, number: u32, side: Side, text: &str) -> DiffLine {
    DiffLine { path: path.into(), number, side, text: text.into() }
}

fn file(path: &str, text: &str) -> (String, String) {
    (path.to_string(), text.to_string())
}

#[test]
fn a_diff_reads_with_quoted_paths_deletions_and_binaries() {
    let text = r#"diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -3,0 +4,2 @@
+one
+two
@@ -8 +10,0 @@
-three
diff --git a/bin.dat b/bin.dat
index 3333333..4444444 100644
Binary files a/bin.dat and b/bin.dat differ
diff --git "a/tab\tname.rs" "b/tab\tname.rs"
index 5555555..6666666 100644
--- "a/tab\tname.rs"
+++ "b/tab\tname.rs"
@@ -1 +1 @@
-old
+new
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-bye
"#;
    assert_eq!(
        parse_diff(text),
        [
            diff("src/a.rs", 4, Side::Added, "one"),
            diff("src/a.rs", 5, Side::Added, "two"),
            diff("src/a.rs", 8, Side::Removed, "three"),
            diff("tab\\tname.rs", 1, Side::Removed, "old"),
            diff("tab\\tname.rs", 1, Side::Added, "new"),
            diff("gone.rs", 1, Side::Removed, "bye"),
        ],
        "a binary file and /dev/null say nothing; a quoted path keeps its escapes"
    );
}

#[test]
fn a_garbled_diff_is_survived() {
    let text = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -a,b +c,d @@\n+not placed\n\
@@ -99999999999 +1 @@\n+also not placed\ndiff --git a/y b/y\n--- a/y\n+++ b/y\n\
@@ -2 +2 @@ fn main()\n-old\n+héllo\n";
    assert_eq!(
        parse_diff(text),
        [diff("y", 2, Side::Removed, "old"), diff("y", 2, Side::Added, "héllo")],
        "a garbled or overflowing hunk header places nothing, and the rest still reads"
    );
    assert!(parse_diff("").is_empty());
    assert!(parse_diff("diff --git a/x b/x\n+++ b/x\n+garbled\n").is_empty());
    assert!(parse_diff("@@ -1 +1 @@\n+x\n").is_empty(), "a hunk with no file places nothing");
}

#[test]
fn forbid_looks_only_at_added_lines_in_its_glob() {
    let lines = [
        diff("src/a.rs", 1, Side::Added, "panic!(\"x\")"),
        diff("src/a.rs", 2, Side::Removed, "panic!(\"old\")"),
        diff("docs/a.md", 1, Side::Added, "panic!(\"doc\")"),
        diff("src/b.rs", 7, Side::Added, "    panic!(\"y\")  "),
    ];
    let (hits, problems) = forbid_hits(&lines, &["src/**/*.rs::panic!".to_string()]);
    assert!(problems.is_empty());
    assert_eq!(
        hits,
        [
            Hit {
                pattern: "src/**/*.rs::panic!".into(),
                file: "src/a.rs".into(),
                line: 1,
                text: "panic!(\"x\")".into()
            },
            Hit {
                pattern: "src/**/*.rs::panic!".into(),
                file: "src/b.rs".into(),
                line: 7,
                text: "panic!(\"y\")".into()
            },
        ],
        "`src/**/*.rs` reaches a file straight under src, and the review shows the trimmed line"
    );
    let mut many: Vec<DiffLine> = Vec::new();
    for number in 1..=60 {
        many.push(diff("src/a.rs", number, Side::Added, "panic!"));
    }
    let (hits, _) = forbid_hits(&many, &["::panic".to_string()]);
    assert_eq!(hits.len(), MAX_HITS, "a review reads at most 50 hits");
}

#[test]
fn a_bad_pattern_is_a_problem_in_words() {
    let lines = [diff("src/a.rs", 1, Side::Added, "x")];
    let files = [file("src/a.rs", "fn main() {}\n")];
    let (hits, problems) = forbid_hits(&lines, &["src/*.rs::(".to_string()]);
    assert!(hits.is_empty());
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("forbid entry \"src/*.rs::(\" is not a valid regex: "), "{}", problems[0]);
    let (missing, problems) = require_missing(&files, &["src/*.rs".to_string(), "src/*.rs::[".to_string()]);
    assert!(missing.is_empty());
    assert_eq!(problems.len(), 2);
    assert_eq!(problems[0], "require entry \"src/*.rs\" needs glob::regex");
    assert!(problems[1].starts_with("require entry \"src/*.rs::[\" is not a valid regex: "), "{}", problems[1]);
    let words = WarnRule::new("src/*.rs::(", Source::File).err().unwrap_or_default();
    assert!(words.starts_with("warn entry \"src/*.rs::(\" is not a valid regex: "), "{words}");
    assert!(WarnRule::new("src/*.rs::panic", Source::File).is_ok());
}

#[test]
fn require_says_which_entry_found_nothing() {
    let files = [file("src/a.rs", "fn main() {}\nfn gone() {}\n"), file("src/b.rs", "// nothing\n")];
    let entries =
        ["src/*.rs::fn main".to_string(), "src/*.rs::fn absent".to_string(), "docs/*.md::anything".to_string()];
    let (missing, problems) = require_missing(&files, &entries);
    assert!(problems.is_empty());
    assert_eq!(missing, ["src/*.rs::fn absent", "docs/*.md::anything"]);
}

#[test]
fn a_removed_test_is_flagged_unless_it_moved() {
    let rules = builtin_warns();
    let lines = [
        diff("t/a.py", 5, Side::Removed, "def test_gone(self):"),
        diff("t/b.py", 9, Side::Removed, "    def test_moved(self):"),
        diff("t/b.py", 3, Side::Added, "    def test_moved(self):"),
    ];
    let hits = warn_hits(&lines, &rules, |_| None);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].rule, "removed-test");
    assert_eq!(hits[0].side, Side::Removed);
    assert_eq!(hits[0].file, "t/a.py");
    assert_eq!(hits[0].line, 5);
    assert_eq!(hits[0].source, Source::Builtin);

    let lines = [
        diff("src/a.rs", 10, Side::Removed, "#[test]"),
        diff("src/a.rs", 11, Side::Removed, "fn test_gone() {"),
        diff("src/a.rs", 20, Side::Removed, "fn helper() {"),
        diff("src/b.rs", 4, Side::Removed, "#[tokio::test]"),
        diff("src/b.rs", 5, Side::Removed, "async fn test_moved() {"),
        diff("src/b.rs", 1, Side::Added, "async fn test_moved() {"),
        diff("src/c.rs", 4, Side::Removed, "#[tokio::test(flavor = \"multi_thread\")]"),
        diff("src/c.rs", 5, Side::Removed, "async fn test_threaded() {"),
    ];
    let hits = warn_hits(&lines, &rules, |_| None);
    assert_eq!(hits.len(), 2, "only the tests an attribute names are flagged: {hits:?}");
    assert_eq!(hits[0].rule, "removed-test");
    assert_eq!(hits[0].file, "src/a.rs");
    assert_eq!(hits[0].line, 11);
    assert_eq!(hits[1].file, "src/c.rs");
}

#[test]
fn an_added_skip_is_flagged() {
    let rules = builtin_warns();
    let lines = [
        diff("t/a.py", 1, Side::Added, "@unittest.skip(\"why\")"),
        diff("t/b.js", 1, Side::Added, "it.skip(\"does nothing\", () => {"),
        diff("src/a.rs", 1, Side::Added, "#[ignore]"),
        diff("src/b.rs", 1, Side::Removed, "#[ignore]"),
    ];
    let hits = warn_hits(&lines, &rules, |_| None);
    assert_eq!(hits.len(), 3, "{hits:?}");
    assert!(hits.iter().all(|hit| hit.rule == "added-skip"), "{hits:?}");
    assert!(hits.iter().all(|hit| hit.side == Side::Added));
    assert_eq!(hits[2].file, "src/a.rs");
}

#[test]
fn a_subprocess_without_creationflags_is_flagged_and_with_it_is_not() {
    let rules = builtin_warns();
    let lines = [
        diff("s/a.py", 3, Side::Added, "    out = subprocess.run([\"git\", \"status\"], capture_output=True)"),
        diff("s/b.py", 3, Side::Added, "    out = subprocess.run([\"git\"], creationflags=NO_WINDOW)"),
    ];
    let files = [
        file("s/a.py", "x = 1\ny = 2\n    out = subprocess.run([\"git\", \"status\"], capture_output=True)\n"),
        file("s/b.py", "x = 1\ny = 2\n    out = subprocess.run([\"git\"], creationflags=NO_WINDOW)\n"),
    ];
    let read = |path: &str| files.iter().find(|(name, _)| name == path).map(|(_, text)| text.clone());
    let hits = warn_hits(&lines, &rules, read);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].rule, "subprocess-window");
    assert_eq!(hits[0].file, "s/a.py");
    assert_eq!(hits[0].line, 3);
}

#[test]
fn a_new_console_is_flagged() {
    let rules = builtin_warns();
    // A flag is written from the rule's own parts, as the rule is: neither is spelled here.
    let flags: Vec<&str> = CONSOLE_FLAGS.split('|').collect();
    let lines = [
        diff("s/a.py", 1, Side::Added, &format!("    flags = {}", flags[0])),
        diff("s/b.rs", 1, Side::Added, &format!("    {}", flags[1])),
    ];
    let hits = warn_hits(&lines, &rules, |_| None);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|hit| hit.rule == "new-console"), "{hits:?}");
}

#[test]
fn the_checks_file_parses_with_every_key() {
    let text = r#"{
        "test_cmd": "cargo test",
        "test_timeout_seconds": 60,
        "forbid": ["src/*.rs::panic"],
        "require": ["src/*.rs::fn main"],
        "warn": ["src/*.rs::dbg!"],
        "reuse_test_for": ["*.md", "docs/*"],
        "qa_rounds": 2,
        "focused_cmd": "cargo test --lib",
        "second_reader": true,
        "a key it does not know": 1
    }"#;
    let (file, problems) = parse_checks(text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(file.test_cmd.as_deref(), Some("cargo test"));
    assert_eq!(file.test_timeout_seconds, Some(60));
    assert_eq!(file.forbid, ["src/*.rs::panic"]);
    assert_eq!(file.require, ["src/*.rs::fn main"]);
    assert_eq!(file.warn, ["src/*.rs::dbg!"]);
    assert_eq!(file.reuse_test_for, ["*.md", "docs/*"]);
    assert_eq!(file.qa_rounds, 2);
    assert_eq!(file.focused_cmd.as_deref(), Some("cargo test --lib"));
    assert!(file.second_reader);
    let (empty, problems) = parse_checks("{}");
    assert!(problems.is_empty());
    assert_eq!(empty.reuse_test_for, [DEFAULT_REUSE_TEST_FOR], "a default the file never names");
    assert_eq!(empty.qa_rounds, DEFAULT_QA_ROUNDS);
    assert!(!empty.second_reader);
}

#[test]
fn a_wrong_type_is_a_problem_in_words_and_the_rest_still_counts() {
    let text = r#"{"test_cmd": 5, "forbid": ["a", 7], "require": ["x::y"], "qa_rounds": 9,
                   "test_timeout_seconds": 0, "second_reader": "yes"}"#;
    let (file, problems) = parse_checks(text);
    assert_eq!(file.test_cmd, None);
    assert!(file.forbid.is_empty());
    assert_eq!(file.require, ["x::y"], "a good key keeps counting");
    assert_eq!(file.qa_rounds, DEFAULT_QA_ROUNDS, "a wrong qa_rounds falls back to 1");
    assert_eq!(file.test_timeout_seconds, None);
    assert_eq!(
        problems,
        [
            ".limen/checks.json@HEAD: test_cmd must be a string",
            ".limen/checks.json@HEAD: test_timeout_seconds must be a positive integer",
            ".limen/checks.json@HEAD: forbid must be an array of strings",
            ".limen/checks.json@HEAD: qa_rounds must be an integer from 0 to 3",
            ".limen/checks.json@HEAD: second_reader must be a boolean",
        ]
    );
    let (_, problems) = parse_checks("not json");
    assert!(problems[0].starts_with(".limen/checks.json is not valid JSON: "), "{}", problems[0]);
    assert_eq!(parse_checks("[]").1, [".limen/checks.json must contain a JSON object"]);
    assert_eq!(
        parse_checks(r#"{"reuse_test_for": "*.md"}"#).1,
        [".limen/checks.json@HEAD: reuse_test_for must be an array of strings"]
    );
    assert!(
        parse_checks(r#"{"reuse_test_for": "*.md"}"#).0.reuse_test_for.is_empty(),
        "an unreadable list is empty, never the default"
    );
}

#[test]
fn a_call_replaces_the_test_and_adds_the_patterns() {
    let file_spec = ChecksFile {
        test_cmd: Some("full".into()),
        forbid: vec!["a".into()],
        require: vec!["b".into()],
        warn: vec!["c".into()],
        ..Default::default()
    };
    let call = ChecksFile {
        test_cmd: Some("call".into()),
        forbid: vec!["d".into(), "a".into()],
        require: vec!["e".into()],
        warn: vec!["c".into()],
        ..Default::default()
    };
    let both = merged(&file_spec, &call);
    assert_eq!(both.test_cmd.as_deref(), Some("call"), "a call's command replaces the file's");
    assert_eq!(both.forbid, ["a", "d"], "the file's first, and an entry is said once");
    assert_eq!(both.require, ["b", "e"]);
    assert_eq!(both.warn, ["c", "c"], "a warn entry is a rule: said twice, it is two rules");
    let kept = merged(&file_spec, &ChecksFile::default());
    assert_eq!(kept.test_cmd.as_deref(), Some("full"), "no call command keeps the file's");
    assert_eq!(kept.forbid, ["a"]);
    let blank = merged(&file_spec, &ChecksFile { test_cmd: Some(String::new()), ..Default::default() });
    assert_eq!(blank.test_cmd.as_deref(), Some("full"), "an empty call command is no command");
}

#[test]
fn a_unittest_run_reads_its_summary_and_failures() {
    let mut read = TestRead::default();
    for line in [
        "Ran 3 tests in 0.002s\r",
        "",
        "FAIL: test_one (test_x.T)",
        "ERROR: test_two (test_x.T)",
        "Ran 4 tests in 0.003s",
        "FAILED (failures=1, errors=1)",
        "OK",
        "Ran 5 tests in 0.004s",
        "Ran 6 tests in 0.005s",
        "FAILED (failures=2)",
    ] {
        read.feed(line);
    }
    assert_eq!(
        read.summary,
        [
            "FAILED (failures=1, errors=1)",
            "OK",
            "Ran 5 tests in 0.004s",
            "Ran 6 tests in 0.005s",
            "FAILED (failures=2)",
        ],
        "the last five: a run's verdict is at its end"
    );
    let mut crlf = TestRead::default();
    crlf.feed("Ran 3 tests in 0.002s\r");
    assert_eq!(crlf.summary, ["Ran 3 tests in 0.002s"], "a `\\r` at the end is ignored");
    assert_eq!(read.failed, ["test_one (test_x.T)", "test_two (test_x.T)"]);
    assert!(read.tail.ends_with("FAILED (failures=2)\n"), "the tail is the last 3000 characters");
    read.feed("FAIL: test_one (test_x.T)");
    assert_eq!(read.failed.len(), 2, "a name is said once");
}

#[test]
fn a_pytest_run_reads_its_failures_but_not_failed_counts() {
    let mut read = TestRead::default();
    for line in [
        "============================= test session starts ==============================",
        "FAILED tests/test_a.py::test_one - assert 1 == 2",
        "ERROR tests/test_b.py::test_two",
        "FAILED (failures=1)",
        "========================= 1 failed, 1 passed in 0.12s =========================",
    ] {
        read.feed(line);
    }
    assert_eq!(read.failed, ["tests/test_a.py::test_one", "tests/test_b.py::test_two"]);
    assert!(!read.failed.iter().any(|name| name.starts_with('(')), "a count is not a name");
    assert_eq!(
        read.summary,
        [
            "FAILED tests/test_a.py::test_one - assert 1 == 2",
            "FAILED (failures=1)",
            "========================= 1 failed, 1 passed in 0.12s =========================",
        ],
        "`brief.py`'s summary takes any line beginning FAILED, as `_collect` does"
    );
}

#[test]
fn a_cargo_run_reads_its_results_and_failures() {
    let mut read = TestRead::default();
    for line in [
        "running 3 tests",
        "test task::brief::tests::a_new_console_is_flagged ... ok",
        "test task::brief::tests::a_removed_test ... FAILED",
        "test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s",
    ] {
        read.feed(line);
    }
    assert_eq!(read.failed, ["task::brief::tests::a_removed_test"]);
    assert_eq!(
        read.summary,
        ["test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s"]
    );
}

#[test]
fn what_check_windows_prints_reads_as_a_run() {
    let text = "\
--- format: cargo fmt --all -- --check
--- the worker loop's unit tests: cargo test --locked -p limen-core --lib --no-fail-fast -- task::
test task::decide::tests::a_wait_ends ... FAILED
test result: FAILED. 10 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
--- the end of the worker loop's unit tests
some failing output
--- the docs and the schema: cargo test --locked -p limen-core --test cli_docs --test schema_file
ERROR: the docs and the schema
Ran 11 tests in 42s
FAILED (steps=2)
";
    let mut read = TestRead::default();
    for line in text.lines() {
        read.feed(line);
    }
    assert_eq!(read.failed, ["task::decide::tests::a_wait_ends", "the docs and the schema"]);
    assert_eq!(
        read.summary,
        [
            "test result: FAILED. 10 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s",
            "Ran 11 tests in 42s",
            "FAILED (steps=2)",
        ]
    );
    let mut many = TestRead::default();
    for _ in 0..7 {
        many.feed("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s");
    }
    many.feed("Ran 11 tests in 42s");
    many.feed("OK");
    assert_eq!(many.summary.len(), SUMMARY_LIMIT);
    assert_eq!(many.summary.last().map(String::as_str), Some("OK"), "many test programs do not hide the verdict");
}

#[test]
fn the_tail_is_cut_on_a_character() {
    let mut read = TestRead::default();
    for _ in 0..2000 {
        read.feed("é");
    }
    assert_eq!(read.tail.chars().count(), TAIL_CHARS, "counted in characters, never cut in a byte");
    assert!(read.tail.chars().all(|c| c == 'é' || c == '\n'), "a two-byte character stays whole");
    assert!(read.tail.ends_with("é\n"));
    let mut one = TestRead::default();
    one.feed(&"x".repeat(TAIL_CHARS + 10));
    assert_eq!(one.tail.chars().count(), TAIL_CHARS);
    assert_eq!(one.tail, format!("{}\n", "x".repeat(TAIL_CHARS - 1)));
    let mut none = TestRead::default();
    none.feed("short");
    assert_eq!(none.tail, "short\n");
}
