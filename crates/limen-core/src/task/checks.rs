//! What is checked when a worker ends: patterns on added lines, and what a test
//! run printed. Pure: text in, values out, so every rule here is a unit test.
//! The Python's `checks.py` is the reference.

use crate::task::decide::in_scope;
use crate::task::types::cut;
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

/// The committed checks file, and the label its problems are said with.
pub const CHECKS_PATH: &str = ".limen/checks.json";
pub const SOURCE_FILE: &str = ".limen/checks.json@HEAD";
pub const DEFAULT_QA_ROUNDS: u8 = 1;
pub const MAX_QA_ROUNDS: i64 = 3;
pub const DEFAULT_REUSE_TEST_FOR: &str = "*.md";
/// A review reads at most this many hits of one kind.
pub const MAX_HITS: usize = 50;
pub const MAX_FAILED: usize = 50;
/// A QA round's look at a subprocess call reaches this far.
pub const UNLESS_SCAN_LINES: usize = 30;
pub const TAIL_CHARS: usize = 3000;
pub const SUMMARY_LIMIT: usize = 5;

/// `.limen/checks.json`, normalized: every key the loop reads, with its default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksFile {
    pub test_cmd: Option<String>,
    pub test_timeout_seconds: Option<u64>,
    pub forbid: Vec<String>,
    pub require: Vec<String>,
    pub warn: Vec<String>,
    pub reuse_test_for: Vec<String>,
    pub qa_rounds: u8,
    pub focused_cmd: Option<String>,
    pub second_reader: bool,
}

impl Default for ChecksFile {
    fn default() -> ChecksFile {
        ChecksFile {
            test_cmd: None,
            test_timeout_seconds: None,
            forbid: Vec::new(),
            require: Vec::new(),
            warn: Vec::new(),
            reuse_test_for: vec![DEFAULT_REUSE_TEST_FOR.to_string()],
            qa_rounds: DEFAULT_QA_ROUNDS,
            focused_cmd: None,
            second_reader: false,
        }
    }
}

fn string_key(spec: &Map<String, Value>, key: &str, out: &mut Option<String>, problems: &mut Vec<String>) {
    match spec.get(key) {
        None | Some(Value::Null) => {}
        Some(Value::String(value)) => *out = Some(value.clone()),
        Some(_) => problems.push(format!("{SOURCE_FILE}: {key} must be a string")),
    }
}

fn array_key(spec: &Map<String, Value>, key: &str, out: &mut Vec<String>, problems: &mut Vec<String>) {
    match spec.get(key) {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            *out = items.iter().filter_map(|item| item.as_str()).map(str::to_string).collect();
        }
        Some(_) => {
            // `_normalize` and `_reuse_test_for` leave an unreadable list empty, not at its default.
            out.clear();
            problems.push(format!("{SOURCE_FILE}: {key} must be an array of strings"));
        }
    }
}

/// `.limen/checks.json` as the text says it is, and a problem for each key it could not read.
pub fn parse_checks(text: &str) -> (ChecksFile, Vec<String>) {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => return (ChecksFile::default(), vec![format!("{CHECKS_PATH} is not valid JSON: {e}")]),
    };
    let Value::Object(spec) = value else {
        return (ChecksFile::default(), vec![format!("{CHECKS_PATH} must contain a JSON object")]);
    };
    let mut file = ChecksFile::default();
    let mut problems = Vec::new();
    string_key(&spec, "test_cmd", &mut file.test_cmd, &mut problems);
    match spec.get("test_timeout_seconds") {
        None => {}
        Some(value) => match value.as_u64().filter(|seconds| *seconds > 0) {
            Some(seconds) => file.test_timeout_seconds = Some(seconds),
            None => problems.push(format!("{SOURCE_FILE}: test_timeout_seconds must be a positive integer")),
        },
    }
    array_key(&spec, "forbid", &mut file.forbid, &mut problems);
    array_key(&spec, "require", &mut file.require, &mut problems);
    array_key(&spec, "warn", &mut file.warn, &mut problems);
    array_key(&spec, "reuse_test_for", &mut file.reuse_test_for, &mut problems);
    match spec.get("qa_rounds") {
        None => {}
        Some(value) => match value.as_i64() {
            Some(rounds) if (0..=MAX_QA_ROUNDS).contains(&rounds) => file.qa_rounds = rounds as u8,
            _ => problems.push(format!("{SOURCE_FILE}: qa_rounds must be an integer from 0 to {MAX_QA_ROUNDS}")),
        },
    }
    string_key(&spec, "focused_cmd", &mut file.focused_cmd, &mut problems);
    match spec.get("second_reader") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(flag)) => file.second_reader = *flag,
        Some(_) => problems.push(format!("{SOURCE_FILE}: second_reader must be a boolean")),
    }
    (file, problems)
}

/// A call's checks laid over the committed file's: its command replaces, its patterns add.
pub fn merged(file: &ChecksFile, call: &ChecksFile) -> ChecksFile {
    let mut both = file.clone();
    // `run_checks` takes the call's command only when it is a non-empty string.
    if let Some(cmd) = call.test_cmd.as_deref().filter(|cmd| !cmd.is_empty()) {
        both.test_cmd = Some(cmd.to_string());
    }
    both.forbid = added(&file.forbid, &call.forbid);
    both.require = added(&file.require, &call.require);
    // `run_checks` drops a repeated forbid or require entry; `_warn_rules` keeps a repeated warn.
    both.warn = file.warn.iter().chain(&call.warn).cloned().collect();
    both
}

fn added(file: &[String], call: &[String]) -> Vec<String> {
    let mut both: Vec<String> = Vec::new();
    for entry in file.iter().chain(call) {
        if !both.contains(entry) {
            both.push(entry.clone());
        }
    }
    both
}

/// Which side of a diff a line is on. `+` is what the worker added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Added,
    Removed,
}

impl Side {
    pub fn word(self) -> &'static str {
        match self {
            Side::Added => "+",
            Side::Removed => "-",
        }
    }
}

/// One line of `git diff -U0`, with the number it had on its own side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub path: String,
    pub number: u32,
    pub side: Side,
    pub text: String,
}

fn diff_path(value: &str, prefix: &str) -> Option<String> {
    let value = value.trim();
    if value == "/dev/null" || value.is_empty() {
        return None;
    }
    // git quotes a path with a byte it cannot print; the escapes stay as written.
    match value.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        Some(quoted) => Some(quoted.strip_prefix(prefix).unwrap_or(quoted).to_string()),
        None => Some(value.strip_prefix(prefix).unwrap_or(value).replace('\\', "/")),
    }
}

fn number(value: &str) -> Option<u32> {
    let (start, count) = match value.split_once(',') {
        Some((start, count)) => (start, Some(count)),
        None => (value, None),
    };
    if let Some(count) = count {
        count.parse::<u32>().ok()?;
    }
    start.parse().ok()
}

/// `@@ -old,count +new,count @@`, as the two numbers it starts at.
fn hunk_header(raw: &str) -> Option<(u32, u32)> {
    let rest = raw.strip_prefix("@@ -")?;
    let (from, rest) = rest.split_once(' ')?;
    let (to, rest) = rest.strip_prefix('+')?.split_once(' ')?;
    if !rest.starts_with("@@") {
        return None;
    }
    Some((number(from)?, number(to)?))
}

/// The added and the removed lines of `git diff -U0`: a hunk's header gives both
/// the old and the new number to count from, so each line keeps its own side's.
pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let (mut old_path, mut new_path): (Option<String>, Option<String>) = (None, None);
    let mut in_hunk = false;
    let (mut old, mut new) = (0u32, 0u32);
    for raw in text.lines() {
        if raw.starts_with("diff --git ") {
            old_path = None;
            new_path = None;
            in_hunk = false;
            continue;
        }
        if !in_hunk {
            if let Some(value) = raw.strip_prefix("--- ") {
                old_path = diff_path(value, "a/");
                continue;
            }
            if let Some(value) = raw.strip_prefix("+++ ") {
                new_path = diff_path(value, "b/");
                continue;
            }
        }
        if let Some((from, to)) = hunk_header(raw) {
            in_hunk = true;
            old = from;
            new = to;
            continue;
        }
        if !in_hunk {
            continue;
        }
        if let Some(text) = raw.strip_prefix('+') {
            if let Some(path) = &new_path {
                lines.push(DiffLine { path: path.clone(), number: new, side: Side::Added, text: text.to_string() });
            }
            new = new.saturating_add(1);
        } else if let Some(text) = raw.strip_prefix('-') {
            if let Some(path) = &old_path {
                lines.push(DiffLine { path: path.clone(), number: old, side: Side::Removed, text: text.to_string() });
            }
            old = old.saturating_add(1);
        } else if raw.starts_with(' ') {
            old = old.saturating_add(1);
            new = new.saturating_add(1);
        } else if !raw.starts_with('\\') {
            // Anything else ends the hunk: a garbled body is skipped, not fatal.
            in_hunk = false;
        }
    }
    lines
}

/// `glob::pattern`, or just a pattern that narrows nothing.
pub fn split_entry(entry: &str) -> (Option<&str>, &str) {
    match entry.split_once("::") {
        Some((glob, pattern)) => (Some(glob), pattern),
        None => (None, entry),
    }
}

/// Every form of the pattern in which `**/` may match no folder, as `_glob_variants`.
fn glob_variants(pattern: &str) -> Vec<String> {
    let mut variants = vec![String::new()];
    for (index, part) in pattern.split("**/").enumerate() {
        if index == 0 {
            variants = vec![part.to_string()];
            continue;
        }
        let mut next = Vec::with_capacity(variants.len() * 2);
        for prefix in &variants {
            next.push(format!("{prefix}**/{part}"));
            next.push(format!("{prefix}{part}"));
        }
        variants = next;
    }
    variants
}

/// Python's `fnmatchcase`, backslashes read as `/`. `decide::in_scope` is the matcher.
fn glob_match(path: &str, glob: &str) -> bool {
    in_scope(path, &glob_variants(&glob.replace('\\', "/")))
}

/// A line a forbid pattern matched, as the review shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub pattern: String,
    pub file: String,
    pub line: u32,
    pub text: String,
}

/// The added lines a forbid entry's pattern matches, and each entry that won't compile.
pub fn forbid_hits(lines: &[DiffLine], entries: &[String]) -> (Vec<Hit>, Vec<String>) {
    let mut problems = Vec::new();
    let mut compiled = Vec::new();
    for entry in entries {
        let (glob, pattern) = split_entry(entry);
        match Regex::new(pattern) {
            Ok(regex) => compiled.push((entry, glob.filter(|glob| !glob.is_empty()), regex)),
            Err(e) => problems.push(format!("forbid entry {entry:?} is not a valid regex: {e}")),
        }
    }
    let added: Vec<&DiffLine> = lines.iter().filter(|line| line.side == Side::Added).collect();
    let mut hits = Vec::new();
    for (entry, glob, regex) in compiled {
        for line in &added {
            if glob.is_some_and(|glob| !glob_match(&line.path, glob)) {
                continue;
            }
            if regex.is_match(&line.text) {
                hits.push(Hit {
                    pattern: entry.clone(),
                    file: line.path.clone(),
                    line: line.number,
                    text: cut(line.text.trim(), 200),
                });
                if hits.len() >= MAX_HITS {
                    return (hits, problems);
                }
            }
        }
    }
    (hits, problems)
}

/// The require entries no file of `files` answers, and each entry that won't work.
pub fn require_missing(files: &[(String, String)], entries: &[String]) -> (Vec<String>, Vec<String>) {
    let mut missing = Vec::new();
    let mut problems = Vec::new();
    let mut compiled = Vec::new();
    for entry in entries {
        let (glob, pattern) = split_entry(entry);
        let Some(glob) = glob else {
            problems.push(format!("require entry {entry:?} needs glob::regex"));
            continue;
        };
        match Regex::new(pattern) {
            Ok(regex) => compiled.push((entry, glob, regex)),
            Err(e) => problems.push(format!("require entry {entry:?} is not a valid regex: {e}")),
        }
    }
    for (entry, glob, regex) in compiled {
        let found =
            files.iter().any(|(path, text)| glob_match(path, glob) && text.lines().any(|line| regex.is_match(line)));
        if !found {
            missing.push(entry.clone());
        }
    }
    (missing, problems)
}

/// Where a warn rule came from, so a review can say whose lesson it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Builtin,
    File,
    Call,
}

/// One warn rule, before its pattern is compiled: the built-ins are written this way.
#[derive(Clone, Copy, Debug)]
pub struct WarnSpec {
    pub rule: &'static str,
    pub side: Side,
    pub glob: Option<&'static str>,
    pub regex: &'static str,
    /// A pattern the line before must match: a removed test's `fn` follows `#[test]`.
    pub prior: Option<&'static str>,
    /// Text in the matched call that exempts the hit.
    pub unless: Option<&'static str>,
}

/// The warn rule that catches either Windows flag that opens a console, written from
/// parts: this repository's own forbid check refuses a plain mention of them here.
const CONSOLE_FLAGS: &str = concat!("DETACHED", "_PROCESS|CREATE", "_NEW_CONSOLE");

/// The warn rules every repository gets, in `checks.py`'s words and these for Rust.
pub const BUILTIN_WARNS: &[WarnSpec] = &[
    WarnSpec {
        rule: "removed-test",
        side: Side::Removed,
        glob: Some("**/*.py"),
        regex: r"^\s*(?:async\s+)?def (test_\w+)",
        prior: None,
        unless: None,
    },
    WarnSpec {
        rule: "removed-test",
        side: Side::Removed,
        glob: Some("**/*.*js"),
        regex: "^\\s*(?:it|test)\\(\\s*(['\"`].+?['\"`])",
        prior: None,
        unless: None,
    },
    WarnSpec {
        rule: "removed-test",
        side: Side::Removed,
        glob: Some("**/*.ts"),
        regex: "^\\s*(?:it|test)\\(\\s*(['\"`].+?['\"`])",
        prior: None,
        unless: None,
    },
    WarnSpec {
        rule: "added-skip",
        side: Side::Added,
        glob: None,
        regex: r"@(?:unittest\.)?skip|pytest\.mark\.skip|\.skipTest\(|\b(?:it|test|describe)\.skip\(|#\[ignore",
        prior: None,
        unless: None,
    },
    WarnSpec {
        rule: "removed-test",
        side: Side::Removed,
        glob: Some("**/*.rs"),
        regex: r"^\s*(?:async\s+)?fn (\w+)",
        prior: Some(r"^\s*#\[(?:tokio::)?test\]"),
        unless: None,
    },
    WarnSpec {
        rule: "subprocess-window",
        side: Side::Added,
        glob: Some("**/*.py"),
        regex: r"\bsubprocess\.(?:Popen|run|call|check_call|check_output)\(",
        prior: None,
        unless: Some("creationflags"),
    },
    WarnSpec { rule: "new-console", side: Side::Added, glob: None, regex: CONSOLE_FLAGS, prior: None, unless: None },
];

/// A warn rule with its pattern compiled.
#[derive(Clone, Debug)]
pub struct WarnRule {
    pub rule: String,
    pub side: Side,
    pub glob: Option<String>,
    pub regex: Regex,
    /// A pattern the line before must match: a removed test's `fn` follows `#[test]`.
    pub prior: Option<Regex>,
    /// Text in the matched call that exempts the hit.
    pub unless: Option<String>,
    /// A removed test's name comes back in an added line when it was moved, not removed.
    pub removed_test: bool,
    pub source: Source,
}

fn compile(pattern: &'static str) -> Regex {
    Regex::new(pattern).expect("a built-in warn pattern compiles")
}

impl WarnRule {
    /// A rule from a checks file's `warn` entry, or the words saying why not.
    pub fn new(entry: &str, source: Source) -> Result<WarnRule, String> {
        let (glob, pattern) = split_entry(entry);
        let regex = Regex::new(pattern).map_err(|e| format!("warn entry {entry:?} is not a valid regex: {e}"))?;
        Ok(WarnRule {
            rule: entry.to_string(),
            side: Side::Added,
            glob: glob.filter(|glob| !glob.is_empty()).map(str::to_string),
            regex,
            prior: None,
            unless: None,
            removed_test: false,
            source,
        })
    }
}

/// The built-in rules, every repository's, in order.
pub fn builtin_warns() -> Vec<WarnRule> {
    BUILTIN_WARNS
        .iter()
        .map(|spec| WarnRule {
            rule: spec.rule.to_string(),
            side: spec.side,
            glob: spec.glob.map(str::to_string),
            regex: compile(spec.regex),
            prior: spec.prior.map(compile),
            unless: spec.unless.map(str::to_string),
            removed_test: spec.rule == "removed-test",
            source: Source::Builtin,
        })
        .collect()
}

/// A line a warn rule flagged, for the review and never for a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarnHit {
    pub rule: String,
    pub side: Side,
    pub file: String,
    pub line: u32,
    pub text: String,
    pub source: Source,
}

/// The text of a matched call, from the match to the parenthesis that closes it.
fn call_text(text: &str, number: u32, start: usize, line: &str) -> String {
    let rows: Vec<&str> = text.lines().collect();
    let index = number.checked_sub(1).map(|index| index as usize).unwrap_or(usize::MAX);
    if index >= rows.len() {
        return line.to_string();
    }
    let mut chars = String::new();
    let mut depth = 0i64;
    let mut opened = false;
    for (offset, row) in rows[index..].iter().take(UNLESS_SCAN_LINES).enumerate() {
        let part = if offset == 0 { row.get(start..).unwrap_or_default() } else { row };
        for c in part.chars() {
            chars.push(c);
            match c {
                '(' => {
                    depth += 1;
                    opened = true;
                }
                ')' => {
                    depth -= 1;
                    if opened && depth <= 0 {
                        return chars;
                    }
                }
                _ => {}
            }
        }
        chars.push('\n');
    }
    chars
}

/// The warn hits of a diff, by `run_warns`. `read_lines` gives a path's text for `unless`.
pub fn warn_hits<F>(lines: &[DiffLine], rules: &[WarnRule], read_lines: F) -> Vec<WarnHit>
where
    F: Fn(&str) -> Option<String>,
{
    let added: Vec<&DiffLine> = lines.iter().filter(|line| line.side == Side::Added).collect();
    let removed: Vec<&DiffLine> = lines.iter().filter(|line| line.side == Side::Removed).collect();
    let mut moved: Vec<String> = Vec::new();
    for rule in rules.iter().filter(|rule| rule.removed_test) {
        for line in &added {
            if let Some(name) = rule.regex.captures(&line.text).and_then(|found| found.get(1)) {
                moved.push(name.as_str().to_string());
            }
        }
    }
    let mut hits = Vec::new();
    for rule in rules {
        if hits.len() >= MAX_HITS {
            break;
        }
        let side = if rule.side == Side::Added { &added } else { &removed };
        for line in side {
            if hits.len() >= MAX_HITS {
                break;
            }
            if rule.glob.as_ref().is_some_and(|glob| !glob_match(&line.path, glob)) {
                continue;
            }
            let Some(found) = rule.regex.captures(&line.text) else { continue };
            if rule.removed_test
                && let Some(name) = found.get(1)
                && moved.iter().any(|item| item == name.as_str())
            {
                continue;
            }
            if let Some(prior) = &rule.prior
                && !lines.iter().any(|other| {
                    other.side == line.side
                        && other.path == line.path
                        && other.number.saturating_add(1) == line.number
                        && prior.is_match(&other.text)
                })
            {
                continue;
            }
            if let Some(unless) = &rule.unless
                && rule.side == Side::Added
                && let Some(text) = read_lines(&line.path)
                && call_text(&text, line.number, found.get(0).map_or(0, |whole| whole.start()), &line.text)
                    .contains(unless)
            {
                continue;
            }
            hits.push(WarnHit {
                rule: rule.rule.clone(),
                side: rule.side,
                file: line.path.clone(),
                line: line.number,
                text: cut(line.text.trim(), 200),
                source: rule.source,
            });
        }
    }
    hits
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a test-output pattern compiles"))
}

static UNITTEST_SUMMARY: OnceLock<Regex> = OnceLock::new();
static UNITTEST_RESULT: OnceLock<Regex> = OnceLock::new();
static PYTEST_SUMMARY: OnceLock<Regex> = OnceLock::new();
static UNITTEST_FAILED: OnceLock<Regex> = OnceLock::new();
static CARGO_FAILED: OnceLock<Regex> = OnceLock::new();

/// What a test run printed, read a line at a time: its summary, the names that failed,
/// and the end of it, which is where the reason usually is.
#[derive(Clone, Debug, Default)]
pub struct TestRead {
    /// At most `SUMMARY_LIMIT` lines.
    pub summary: Vec<String>,
    /// At most `MAX_FAILED` names, without repeats.
    pub failed: Vec<String>,
    /// The last `TAIL_CHARS` characters of the run.
    pub tail: String,
    names: Vec<String>,
    chars: usize,
}

impl TestRead {
    /// One line, with or without the `\r\n` that ended it.
    pub fn feed(&mut self, line: &str) {
        let line = line.trim_end_matches(['\r', '\n']);
        if self.summary.len() < SUMMARY_LIMIT && is_summary(line) {
            self.summary.push(line.to_string());
        }
        if let Some(name) = failed_name(line)
            && !self.names.contains(&name)
            && self.failed.len() < MAX_FAILED
        {
            self.names.push(name.clone());
            self.failed.push(name);
        }
        self.tail.push_str(line);
        self.tail.push('\n');
        self.chars += line.chars().count() + 1;
        if self.chars > TAIL_CHARS {
            let drop = self.chars - TAIL_CHARS;
            let end = self.tail.char_indices().nth(drop).map_or(self.tail.len(), |(at, _)| at);
            self.tail.replace_range(..end, "");
            self.chars = TAIL_CHARS;
        }
    }
}

fn is_summary(line: &str) -> bool {
    re(&UNITTEST_SUMMARY, r"^Ran \d+ tests? in ").is_match(line)
        || re(&UNITTEST_RESULT, r"^(?:OK|FAILED)\b.*").is_match(line)
        || re(&PYTEST_SUMMARY, r"^=+ .*\b(?:passed|failed|error|errors)\b.* in [\d.]+s.* =+$").is_match(line)
        || line.starts_with("test result: ")
}

fn failed_name(line: &str) -> Option<String> {
    if let Some(found) = re(&UNITTEST_FAILED, r"^(?:FAIL|ERROR): (.+)$").captures(line) {
        return found.get(1).map(|name| name.as_str().to_string());
    }
    // `regex` has no look-ahead for `(?!\()`, so a pytest count is refused by hand.
    if let Some(rest) = line.strip_prefix("FAILED ").or_else(|| line.strip_prefix("ERROR ")) {
        let name = rest.find(char::is_whitespace).map_or(rest, |at| &rest[..at]);
        if !name.is_empty() && !name.starts_with('(') {
            return Some(name.to_string());
        }
    }
    if let Some(found) = re(&CARGO_FAILED, r"^test (\S+) \.\.\. FAILED").captures(line) {
        return found.get(1).map(|name| name.as_str().to_string());
    }
    None
}

#[cfg(test)]
mod tests {
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
        ];
        let hits = warn_hits(&lines, &rules, |_| None);
        assert_eq!(hits.len(), 1, "only the test the attribute names is flagged: {hits:?}");
        assert_eq!(hits[0].rule, "removed-test");
        assert_eq!(hits[0].file, "src/a.rs");
        assert_eq!(hits[0].line, 11);
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
                "Ran 3 tests in 0.002s",
                "Ran 4 tests in 0.003s",
                "FAILED (failures=1, errors=1)",
                "OK",
                "Ran 5 tests in 0.004s",
            ],
            "at most five, and a `\\r` at the end is ignored"
        );
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
}
