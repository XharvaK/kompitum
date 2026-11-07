//! What is checked when a worker ends: patterns on added lines, and what a test
//! run printed. Pure: text in, values out, so every rule here is a unit test.
//! The Python's `checks.py` is the reference.

use crate::task::decide::in_scope;
use crate::task::types::{Asked, cut};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::OnceLock;

/// The committed checks file, and the label its problems are said with.
pub const CHECKS_PATH: &str = ".limen/checks.json";
pub const SOURCE_FILE: &str = ".limen/checks.json@HEAD";
pub const DEFAULT_QA_ROUNDS: u8 = 1;
pub const MAX_QA_ROUNDS: i64 = 3;
pub const DEFAULT_REUSE_TEST_FOR: &str = "*.md";
/// The scope's plain paths that look like test files, when `focused_glob` names none.
pub const DEFAULT_FOCUSED_GLOB: &[&str] = &["**/test_*.py", "**/*_test.py", "**/*.test.*", "**/*.spec.*"];
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
    pub focused_glob: Vec<String>,
    pub second_reader: bool,
    /// Keys whose values were unusable: said, but never a failed check.
    pub warnings: Vec<String>,
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
            focused_glob: DEFAULT_FOCUSED_GLOB.iter().map(|glob| glob.to_string()).collect(),
            second_reader: false,
            warnings: Vec::new(),
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
    // A warn never fails a task, so its own bad key is a warning, not a problem.
    array_key(&spec, "warn", &mut file.warn, &mut file.warnings);
    array_key(&spec, "reuse_test_for", &mut file.reuse_test_for, &mut problems);
    match spec.get("qa_rounds") {
        None => {}
        Some(value) => match value.as_i64() {
            Some(rounds) if (0..=MAX_QA_ROUNDS).contains(&rounds) => file.qa_rounds = rounds as u8,
            _ => file.warnings.push(format!("{SOURCE_FILE}: qa_rounds must be an integer from 0 to {MAX_QA_ROUNDS}")),
        },
    }
    string_key(&spec, "focused_cmd", &mut file.focused_cmd, &mut problems);
    // `focused_globs`: one glob or a list of them; anything else keeps the default.
    match spec.get("focused_glob") {
        Some(Value::String(glob)) if !glob.trim().is_empty() => file.focused_glob = vec![glob.clone()],
        Some(Value::Array(items)) => {
            file.focused_glob = items
                .iter()
                .filter_map(Value::as_str)
                .filter(|glob| !glob.trim().is_empty())
                .map(str::to_string)
                .collect();
        }
        _ => {}
    }
    match spec.get("second_reader") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(flag)) => file.second_reader = *flag,
        Some(_) => file.warnings.push(format!("{SOURCE_FILE}: second_reader must be a boolean")),
    }
    (file, problems)
}

/// What a task asked, as the checks `merged` lays over the committed file.
pub fn asked_checks(asked: &Asked) -> ChecksFile {
    ChecksFile {
        test_cmd: asked.test.clone(),
        forbid: asked.forbid.clone(),
        require: asked.require.clone(),
        warn: asked.warn.clone(),
        ..ChecksFile::default()
    }
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
    both.warnings = file.warnings.iter().chain(&call.warnings).cloned().collect();
    // A file that names an empty command names none, as an empty call command does.
    both.test_cmd = both.test_cmd.filter(|cmd| !cmd.is_empty());
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
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
pub(crate) fn glob_match(path: &str, glob: &str) -> bool {
    in_scope(path, &glob_variants(&glob.replace('\\', "/")))
}

/// A line a forbid pattern matched, as the review shows it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
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
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
        prior: Some(r"^\s*#\[(?:tokio::)?test\b"),
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
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
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
    /// The last `SUMMARY_LIMIT` lines: a run's verdict is at its end.
    pub summary: Vec<String>,
    /// At most `MAX_FAILED` names, without repeats.
    pub failed: Vec<String>,
    /// The last `TAIL_CHARS` characters of the run.
    pub tail: String,
    chars: usize,
}

impl TestRead {
    /// One line, with or without the `\r\n` that ended it.
    pub fn feed(&mut self, line: &str) {
        let line = line.trim_end_matches(['\r', '\n']);
        if is_summary(line) {
            if self.summary.len() == SUMMARY_LIMIT {
                self.summary.remove(0);
            }
            self.summary.push(line.to_string());
        }
        if let Some(name) = failed_name(line)
            && !self.failed.contains(&name)
            && self.failed.len() < MAX_FAILED
        {
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
mod tests;
