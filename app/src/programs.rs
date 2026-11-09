//! What runs in a terminal, as people see it: a name, an icon and a tile.
//! One table for every mod: `defaults/programs.json` (built in), with the
//! person's `programs.json` in the home folder on top (entry by entry). The
//! shipped icons are written to `icons/` in the home folder (the app's;
//! rewritten when they change). Mods read the result as `program` on
//! panes, agents and terminals.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;

const BUILT_IN: &str = include_str!("../defaults/programs.json");
include!(concat!(env!("OUT_DIR"), "/default_icons.rs"));

#[derive(Deserialize, Default)]
struct Table {
    #[serde(default)]
    shells: Vec<String>,
    #[serde(default)]
    interpreters: Vec<String>,
    #[serde(default)]
    programs: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Serialize, Default, Clone)]
struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// A path in the home folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    /// A one-color glyph, drawn in `color` (or the text color); else a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    /// Other process names (or agents' 7501 app names) for this entry.
    #[serde(default, rename = "match", skip_serializing_if = "Vec::is_empty")]
    also: Vec<String>,
    /// How to start it, for an agent people can start from a menu (`knownAgents()`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    run: Vec<String>,
    /// How its interactive command takes a model and a reasoning level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    flags: Option<Flags>,
    /// How its interactive command starts, resumes or continues a conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<Session>,
    /// The reasoning levels, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    efforts: Vec<String>,
    /// The models the CLI is known to take.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    models: Vec<Model>,
    /// The argv that prints the CLI's own model list, one per line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    list_models: Vec<String>,
    /// How to install it on each OS (`std::env::consts::OS` values), or where.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    install: Option<Install>,
    /// The argv that signs in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    login: Vec<String>,
    /// An argv whose exit code says whether the CLI is signed in (0 yes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    signed_in: Vec<String>,
    /// The core reader that runs it as a worker: codex, command_code or opencode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worker: Option<String>,
    /// The core's route template for this worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<String>,
    /// Its subscription plans and their limits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    plans: Vec<Plan>,
    /// status_line: the numbers come from the CLI's own status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reports: Option<String>,
    /// The argv with which the CLI reports its own plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    usage: Vec<String>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq)]
struct Flags {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    model: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    effort: Vec<String>,
}

/// The argv templates that start, resume or continue a CLI's conversation.
#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq)]
struct Session {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    new: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resume: Option<Vec<String>>,
    #[serde(default, rename = "continue", skip_serializing_if = "Option::is_none")]
    continuation: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq)]
struct Model {
    id: String,
    label: String,
}

/// How a CLI installs on each OS, keyed by `std::env::consts::OS`; the ones
/// with no command are left out, and `page` is its install page.
#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq)]
struct Install {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    windows: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    macos: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linux: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    page: Option<String>,
}

impl Install {
    /// The install argv for an OS; None when it has no command there.
    #[cfg(test)]
    fn argv(&self, os: &str) -> Option<&[String]> {
        match os {
            "windows" => self.windows.as_deref(),
            "macos" => self.macos.as_deref(),
            "linux" => self.linux.as_deref(),
            _ => None,
        }
    }
}

/// A plan's limits by window: `windows`, or per model where they differ.
#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq)]
struct Plan {
    id: String,
    label: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    windows: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    models: BTreeMap<String, BTreeMap<String, f64>>,
}

impl Entry {
    fn over(&mut self, mine: Entry) {
        self.name = mine.name.or(self.name.take());
        self.icon = mine.icon.or(self.icon.take());
        self.tint = mine.tint.or(self.tint);
        self.tile = mine.tile.or(self.tile.take());
        self.color = mine.color.or(self.color.take());
        if !mine.also.is_empty() {
            self.also = mine.also;
        }
        if !mine.run.is_empty() {
            self.run = mine.run;
        }
        self.flags = mine.flags.or(self.flags.take());
        self.session = mine.session.or(self.session.take());
        if !mine.efforts.is_empty() {
            self.efforts = mine.efforts;
        }
        if !mine.models.is_empty() {
            self.models = mine.models;
        }
        if !mine.list_models.is_empty() {
            self.list_models = mine.list_models;
        }
        self.install = mine.install.or(self.install.take());
        if !mine.login.is_empty() {
            self.login = mine.login;
        }
        if !mine.signed_in.is_empty() {
            self.signed_in = mine.signed_in;
        }
        self.worker = mine.worker.or(self.worker.take());
        self.route = mine.route.or(self.route.take());
        if !mine.plans.is_empty() {
            self.plans = mine.plans;
        }
        self.reports = mine.reports.or(self.reports.take());
        if !mine.usage.is_empty() {
            self.usage = mine.usage;
        }
    }
}

/// The agents people pick most, first in menus; the rest follow by name.
const AGENTS_FIRST: &[&str] = &[
    "claude", "codex", "pi", "opencode", "cursor", "copilot", "amp", "gemini", "grok",
];

/// Every agent the table can start (entries with `run`), most picked first:
/// `{ key, name, icon, tint, tile, color, run }`.
pub fn startable() -> Vec<Value> {
    TABLE.with(|t| {
        let t = t.borrow();
        let mut keys: Vec<&String> = t
            .programs
            .iter()
            .filter(|(_, e)| !e.run.is_empty())
            .map(|(k, _)| k)
            .collect();
        keys.sort_by_key(|k| {
            (
                AGENTS_FIRST
                    .iter()
                    .position(|f| f == k)
                    .unwrap_or(AGENTS_FIRST.len()),
                k.to_string(),
            )
        });
        keys.into_iter()
            .map(|k| {
                let e = &t.programs[k];
                let mut v = shown(k, e, None, false, true);
                v["run"] = json!(e.run);
                if let Some(flags) = &e.flags {
                    v["flags"] = json!(flags);
                }
                if let Some(session) = &e.session {
                    v["session"] = json!(session);
                }
                if !e.efforts.is_empty() {
                    v["efforts"] = json!(e.efforts);
                }
                if !e.models.is_empty() {
                    v["models"] = json!(e.models);
                }
                if !e.list_models.is_empty() {
                    v["list_models"] = json!(e.list_models);
                }
                if let Some(install) = &e.install {
                    v["install"] = json!(install);
                }
                if !e.login.is_empty() {
                    v["login"] = json!(e.login);
                }
                if !e.signed_in.is_empty() {
                    v["signed_in"] = json!(e.signed_in);
                }
                if let Some(worker) = &e.worker {
                    v["worker"] = json!(worker);
                }
                if let Some(route) = &e.route {
                    v["route"] = json!(route);
                }
                if !e.plans.is_empty() {
                    v["plans"] = json!(e.plans);
                }
                if let Some(reports) = &e.reports {
                    v["reports"] = json!(reports);
                }
                if !e.usage.is_empty() {
                    v["usage"] = json!(e.usage);
                }
                v
            })
            .collect()
    })
}

/// One entry as the lead needs it: how to start it, its flags, and its session argv.
/// None when the table has no such program.
pub fn lead_spec(key: &str) -> Option<Value> {
    TABLE.with(|t| {
        let t = t.borrow();
        let e = t.programs.get(key)?;
        Some(json!({
            "run": e.run,
            "flags": e.flags,
            "session": e.session,
        }))
    })
}

/// Which of `commands` this machine can start, asked of the shell agents start
/// in (so an app opened from the Dock finds what the person's terminal finds).
/// On another machine (`on`): its login/interactive shell, as remote agents start.
pub fn installed(commands: &[String], on: Option<&str>) -> Vec<String> {
    if commands.is_empty() {
        return Vec::new();
    }
    // Each found one as a marked line: an interactive shell may print its own
    // things (a prompt's escapes, a greeting) around the answer.
    let out = match on {
        Some(m) => {
            let argv = crate::remote_shell::check(commands);
            let result = match crate::mods::run_remote(m, &argv[0], &argv[1..], None) {
                Err(e) if e.contains("cannot start sh: program not found") => {
                    crate::mods::run_remote(
                        m,
                        "powershell.exe",
                        &installed_powershell(commands),
                        None,
                    )
                }
                r => r,
            };
            result
                .map(|(_, out, _)| String::from_utf8_lossy(&out).into_owned())
                .unwrap_or_default()
        }
        None if cfg!(windows) => {
            // The core refreshes user/system PATH; the app may have inherited only the system PATH.
            let mut args = vec!["exec".into(), "--".into(), crate::window::shell()];
            args.extend(installed_powershell(commands));
            crate::mods::run_here(&crate::core_bin::path().to_string_lossy(), &args)
                .unwrap_or_default()
        }
        None => {
            let shell = crate::window::shell();
            let line = installed_line(commands, base(&shell) == "fish");
            crate::mods::run_here(&shell, &["-ic".into(), line]).unwrap_or_default()
        }
    };
    let found: std::collections::HashSet<&str> = out
        .split("kompitum-has:")
        .skip(1)
        .filter_map(|s| s.split_whitespace().next())
        .collect();
    commands
        .iter()
        .filter(|c| found.contains(c.as_str()))
        .cloned()
        .collect()
}

fn installed_powershell(commands: &[String]) -> Vec<String> {
    let list = commands
        .iter()
        .map(|c| powershell_quote(c))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); foreach($c in @({list})){{ if(Get-Command $c -ErrorAction SilentlyContinue){{ \"kompitum-has:$c\" }} }}"
    );
    vec!["-NoLogo".into(), "-Command".into(), script]
}

/// Check in the shell itself for functions, but not aliases (`"$@"` cannot run them).
fn installed_line(commands: &[String], fish: bool) -> String {
    commands
        .iter()
        .map(|c| {
            let word = format!("'{}'", c.replace('\'', "'\\''"));
            let unalias = if fish {
                String::new()
            } else {
                format!("unalias -- {word} 2>/dev/null; ")
            };
            format!(
                "{unalias}command -v {word} >/dev/null 2>&1 && printf 'kompitum-has:%s\\n' {word};"
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(serde_json::from_str(BUILT_IN).expect("defaults/programs.json"));
}

pub fn path() -> std::path::PathBuf {
    crate::mods::app_dir().join("programs.json")
}

/// Writes the shipped icons into the home folder.
pub fn install_icons() {
    let dir = crate::mods::app_dir().join("icons");
    let _ = std::fs::create_dir_all(&dir);
    for (name, bytes) in DEFAULT_ICONS {
        let file = dir.join(name);
        if std::fs::read(&file).ok().as_deref() != Some(*bytes) {
            let _ = crate::persist::write_atomic(&file, bytes);
        }
    }
}

/// Reads the person's table on top of the built-in one. Err: what's wrong with their file.
pub fn load() -> Result<(), String> {
    let mut table: Table = serde_json::from_str(BUILT_IN).expect("defaults/programs.json");
    let result = match std::fs::read_to_string(path()) {
        Err(_) => Ok(()),
        Ok(text) => match serde_json::from_str::<Table>(&text) {
            Err(e) => Err(format!("programs.json: {e}")),
            Ok(mine) => {
                table.shells.extend(mine.shells);
                table.interpreters.extend(mine.interpreters);
                for (key, entry) in mine.programs {
                    table.programs.entry(key).or_default().over(entry);
                }
                Ok(())
            }
        },
    };
    TABLE.with(|t| *t.borrow_mut() = table);
    result
}

/// How a terminal shows: from the agent's own report (`app`), else the
/// program it runs (`argv`). `{ key, name, icon, tint, tile, color, shell, agent }`.
pub fn of(app: Option<&str>, argv: &[String]) -> Value {
    TABLE.with(|t| resolve(&t.borrow(), app, argv))
}

/// How the app calls an agent ("Kiro finished"), from its 7501 `app`: the
/// name `of` shows for it (the person's `programs.json` included), else the `app`.
pub fn name(app: &str) -> String {
    TABLE
        .with(|t| {
            find(&t.borrow(), app).map(|(key, e)| e.name.clone().unwrap_or_else(|| key.to_owned()))
        })
        .unwrap_or_else(|| app.to_owned())
}

/// An agent's 7501 `app` as a name Kompitum ships (`claude` for `claude-code`),
/// else `other` (`unknown` when it gave none): never the person's own words.
pub fn agent_name(app: Option<&str>) -> String {
    static NAMES: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    let names = NAMES.get_or_init(|| {
        let table: Table = serde_json::from_str(BUILT_IN).expect("defaults/programs.json");
        let mut names = BTreeMap::new();
        for (key, e) in table.programs {
            for alias in &e.also {
                names.insert(alias.to_lowercase(), key.clone());
            }
            names.insert(key.to_lowercase(), key);
        }
        names
    });
    match app.map(str::trim).filter(|a| !a.is_empty()) {
        None => "unknown".into(),
        Some(a) => names
            .get(&a.to_lowercase())
            .cloned()
            .unwrap_or_else(|| "other".into()),
    }
}

fn base(s: &str) -> &str {
    let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let s = s.trim_start_matches('-');
    // Windows: `pwsh.exe` is `pwsh`.
    match s.len().checked_sub(4) {
        Some(i) if s.is_char_boundary(i) && s[i..].eq_ignore_ascii_case(".exe") => &s[..i],
        _ => s,
    }
}

/// PowerShell's single quotes, which include the curly ones.
const PS_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

/// `& 'prog' 'arg 1'`: a command for PowerShell's `-Command`, each word quoted.
pub fn powershell_line(cmd: &[String]) -> String {
    format!(
        "& {}",
        cmd.iter()
            .map(|w| powershell_quote(w))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn powershell_quote(word: &str) -> String {
    let mut s = String::from("'");
    for ch in word.chars() {
        if PS_QUOTES.contains(&ch) {
            s.push(ch);
        }
        s.push(ch);
    }
    s + "'"
}

/// The words back from `powershell_line`.
fn powershell_words(line: &str) -> Option<Vec<String>> {
    let mut chars = line.strip_prefix("& ")?.chars().peekable();
    let mut words = Vec::new();
    while let Some(c) = chars.next() {
        if c == ' ' {
            continue;
        }
        if !PS_QUOTES.contains(&c) {
            return None;
        }
        let mut w = String::new();
        while let Some(ch) = chars.next() {
            if PS_QUOTES.contains(&ch) {
                if chars.peek().is_some_and(|n| PS_QUOTES.contains(n)) {
                    w.push(chars.next().unwrap_or(ch));
                    continue;
                }
                break;
            }
            w.push(ch);
        }
        words.push(w);
    }
    Some(words)
}

fn find<'a>(t: &'a Table, name: &str) -> Option<(&'a str, &'a Entry)> {
    let lower = name.to_lowercase();
    t.programs
        .iter()
        .find(|(k, _)| k.as_str() == name || k.to_lowercase() == lower)
        .or_else(|| {
            t.programs.iter().find(|(_, e)| {
                e.also
                    .iter()
                    .any(|a| a == name || a.to_lowercase() == lower)
            })
        })
        .map(|(k, e)| (k.as_str(), e))
}

fn resolve(t: &Table, app: Option<&str>, argv: &[String]) -> Value {
    if let Some(app) = app.filter(|a| !a.is_empty()) {
        return match find(t, app) {
            Some((key, e)) => shown(key, e, None, false, true),
            None => shown(
                "agent",
                t.programs.get("agent").unwrap_or(&Entry::default()),
                Some(app),
                false,
                true,
            ),
        };
    }
    // The app starts commands in the person's shell: `zsh -ic '"$@"; exec "$0" -i' zsh <cmd>...`.
    if argv.len() > 4
        && t.shells.iter().any(|s| s == base(&argv[0]))
        && argv[1].starts_with('-')
        && argv[1].ends_with('c')
        && (argv[2].contains("\"$@\"") || argv[2] == "command $argv[2..]; exec $argv[1] -i")
    {
        return resolve(t, None, &argv[4..]);
    }
    // On Windows: `pwsh -NoExit -Command "& 'codex' ..."`.
    if argv.len() == 4
        && t.shells.iter().any(|s| s == base(&argv[0]))
        && argv[1] == "-NoExit"
        && argv[2] == "-Command"
        && let Some(words) = powershell_words(&argv[3])
    {
        return resolve(t, None, &words);
    }
    let Some(first) = argv.first().map(|a| base(a)) else {
        return shown(
            "shell",
            t.programs.get("shell").unwrap_or(&Entry::default()),
            None,
            true,
            false,
        );
    };
    // `node .../vite.js dev`, `npx vite`, `uv run pytest`: the program they run, when it's known.
    if t.interpreters.iter().any(|i| i == first) {
        for arg in argv[1..].iter().filter(|a| !a.starts_with('-')) {
            // A script's name without its extension (`vite.js`), never a module path (`http.server`).
            let name = base(arg);
            let stem = [".js", ".mjs", ".cjs", ".ts", ".py", ".rb"]
                .iter()
                .find_map(|x| name.strip_suffix(x))
                .unwrap_or(name);
            if let Some((key, e)) =
                find(t, stem).filter(|(k, _)| !t.interpreters.iter().any(|i| i == k))
            {
                return shown(key, e, None, false, false);
            }
        }
    }
    if let Some((key, e)) = find(t, first) {
        return shown(key, e, None, false, false);
    }
    if t.shells.iter().any(|s| s == first) {
        return shown(
            "shell",
            t.programs.get("shell").unwrap_or(&Entry::default()),
            Some(first),
            true,
            false,
        );
    }
    let mut e = t.programs.get("program").cloned().unwrap_or_default();
    e.tile = Some(hash_color(first).into());
    e.color = Some("#ffffff".into());
    shown("program", &e, Some(first), false, false)
}

fn shown(key: &str, e: &Entry, name: Option<&str>, shell: bool, agent: bool) -> Value {
    json!({
        "key": key,
        "name": name.map(str::to_owned).or_else(|| e.name.clone()).unwrap_or_else(|| key.to_owned()),
        "icon": e.icon,
        "tint": e.tint.unwrap_or(true),
        "tile": e.tile,
        "color": e.color,
        "shell": shell,
        "agent": agent,
    })
}

/// A color for a program nobody listed, the same every time.
fn hash_color(s: &str) -> &'static str {
    const PALETTE: [&str; 8] = [
        "#6d5bd0", "#2f80ed", "#0f9d8a", "#d9822b", "#c2417d", "#3a7d44", "#8a5a44", "#4e5d94",
    ];
    let h = s
        .bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    PALETTE[(h % PALETTE.len() as u32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Table {
        serde_json::from_str(BUILT_IN).unwrap()
    }
    fn key(app: Option<&str>, argv: &[&str]) -> String {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        resolve(&t(), app, &argv)["key"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn powershell_words_survive_quotes() {
        let words: Vec<String> = ["pi", "it's \u{2019}fine\u{2019}", "", "a  b"]
            .map(String::from)
            .to_vec();
        let line = powershell_line(&words);
        assert_eq!(
            line,
            "& 'pi' 'it''s \u{2019}\u{2019}fine\u{2019}\u{2019}' '' 'a  b'"
        );
        assert_eq!(powershell_words(&line), Some(words));
    }

    #[test]
    fn powershell_checks_keep_command_names_literal() {
        let argv = installed_powershell(&[
            "claude".into(),
            "missing'command; $x".into(),
            "O’Brien".into(),
        ]);
        assert_eq!(argv[..2], ["-NoLogo", "-Command"]);
        assert!(argv[2].contains("@('claude','missing''command; $x','O’’Brien')"));
        assert!(argv[2].contains("[Console]::OutputEncoding"));
    }

    #[cfg(windows)]
    #[test]
    fn powershell_finds_installed_commands() {
        let out = crate::mods::run_here(
            "powershell.exe",
            &installed_powershell(&["cmd.exe".into(), "kompitum_missing_agent_440".into()]),
        )
        .unwrap();
        assert!(out.contains("kompitum-has:cmd.exe"));
        assert!(!out.contains("kompitum-has:kompitum_missing_agent_440"));
    }

    #[test]
    fn agents_shells_tools_and_what_interpreters_run() {
        assert_eq!(key(Some("claude-code"), &["/bin/zsh"]), "claude");
        assert_eq!(key(Some("grok-build"), &["/bin/zsh"]), "grok");
        assert_eq!(key(Some("dsh-tui"), &["/bin/zsh"]), "dsh-tui");
        assert_eq!(key(None, &["dsh-tui"]), "dsh-tui");
        assert_eq!(key(Some("newagent"), &[]), "agent");
        assert_eq!(key(None, &["/bin/zsh"]), "shell");
        assert_eq!(key(None, &["-zsh"]), "shell");
        assert_eq!(key(None, &["nvim", "x.rs"]), "nvim");
        assert_eq!(key(None, &["vi"]), "vim");
        assert_eq!(
            key(None, &["node", "/x/node_modules/.bin/vite", "dev"]),
            "vite"
        );
        assert_eq!(key(None, &["npx", "vitest"]), "vitest");
        assert_eq!(key(None, &["node", "server.js"]), "node");
        assert_eq!(
            key(None, &["python3", "-m", "http.server", "8799"]),
            "python"
        );
        assert_eq!(key(None, &["uv", "run", "pytest"]), "pytest");
        assert_eq!(key(None, &["mytool"]), "program");
        assert_eq!(
            key(
                None,
                &[
                    "/bin/zsh",
                    "-ic",
                    "\"$@\"; exec \"$0\" -i",
                    "/bin/zsh",
                    "vi",
                    "x"
                ]
            ),
            "vim"
        );
        assert_eq!(
            key(
                None,
                &[
                    "/usr/bin/fish",
                    "-ic",
                    "command $argv[2..]; exec $argv[1] -i",
                    "/usr/bin/fish",
                    "vi",
                    "x"
                ]
            ),
            "vim"
        );
        let line = powershell_line(&["codex".into(), "fix it".into()]);
        assert_eq!(
            key(None, &["pwsh.exe", "-NoExit", "-Command", &line]),
            "codex"
        );
        assert_eq!(key(None, &[r"C:\Windows\powershell.exe"]), "shell");
    }

    #[cfg(unix)]
    #[test]
    fn installed_checks_functions_and_quoted_names_but_not_aliases() {
        let commands = [
            "issue57_function",
            "issue57_alias",
            "printf",
            "missing'command; false",
        ]
        .map(String::from);
        let line = installed_line(&commands, false);
        let out = std::process::Command::new("sh")
            .args([
                "-c",
                &format!("issue57_function() {{ :; }}; alias issue57_alias='printf alias'; {line}"),
            ])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "kompitum-has:issue57_function\nkompitum-has:printf\n"
        );
    }

    #[test]
    fn every_entry_has_its_icon() {
        for (k, e) in t().programs {
            let icon = e.icon.unwrap_or_else(|| panic!("{k} has no icon"));
            let file = icon.strip_prefix("icons/").unwrap();
            assert!(
                DEFAULT_ICONS.iter().any(|(n, _)| *n == file),
                "{k}: {icon} not shipped"
            );
        }
    }

    #[test]
    fn shipped_agents_carry_their_flags_models_and_workers() {
        let table = t();
        for key in ["claude", "command-code", "opencode", "grok"] {
            let e = &table.programs[key];
            assert!(e.flags.is_some(), "{key} has no flags");
            assert!(!e.efforts.is_empty(), "{key} has no efforts");
            assert!(!e.models.is_empty(), "{key} has no models");
        }
        for (key, e) in &table.programs {
            if e.worker.is_some() {
                assert!(!e.models.is_empty(), "{key} has a worker but no models");
                assert!(e.route.is_some(), "{key} has a worker but no route");
            }
        }
    }

    #[test]
    fn shipped_leads_carry_their_install_login_and_signed_in() {
        let table = t();
        for key in ["claude", "codex", "command-code", "opencode", "grok"] {
            let e = &table.programs[key];
            let install = e.install.as_ref().unwrap_or_else(|| panic!("{key} has no install"));
            assert!(
                install.page.as_deref().is_some_and(|p| p.starts_with("https://")),
                "{key} has no install page"
            );
            assert!(!e.login.is_empty(), "{key} has no login");
        }
        assert_eq!(table.programs["claude"].signed_in, ["claude", "auth", "status"]);
        assert!(table.programs["codex"].signed_in.is_empty(), "codex's help promises no exit code");
        assert_eq!(
            table.programs["opencode"].install.as_ref().unwrap().argv("windows"),
            None,
            "OpenCode has no Windows install command"
        );
        let codex = table.programs["codex"].install.as_ref().unwrap().argv("windows").unwrap();
        assert_eq!(
            &codex[..5],
            ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"]
        );
        assert_eq!(
            codex[5],
            "$env:Path = \"$env:SystemRoot\\System32;$env:Path\"; irm https://chatgpt.com/codex/install.ps1 | iex"
        );
    }

    #[test]
    fn install_argv_is_picked_for_the_os_and_absent_elsewhere() {
        let install: Install = serde_json::from_value(json!({
            "windows": ["install.exe"], "macos": ["sh", "-c", "install"], "page": "https://example.invalid"
        }))
        .unwrap();
        assert_eq!(install.argv("windows").map(|a| a.join(" ")).as_deref(), Some("install.exe"));
        assert_eq!(install.argv("macos").map(|a| a.join(" ")).as_deref(), Some("sh -c install"));
        assert!(install.argv("linux").is_none());
        assert!(install.argv("freebsd").is_none());
    }

    #[test]
    fn known_agents_carry_the_catalog_fields() {
        let list = startable();
        let claude = list.iter().find(|a| a["key"] == "claude").unwrap();
        assert_eq!(claude["flags"]["model"], json!(["--model", "{model}"]));
        assert_eq!(claude["reports"], json!("status_line"));
        assert_eq!(claude["login"], json!(["claude", "auth", "login"]));
        assert_eq!(claude["signed_in"], json!(["claude", "auth", "status"]));
        assert_eq!(
            claude["install"]["linux"],
            json!(["sh", "-c", "curl -fsSL https://claude.ai/install.sh | bash"])
        );
        let opencode = list.iter().find(|a| a["key"] == "opencode").unwrap();
        assert_eq!(opencode["worker"], json!("opencode"));
        assert_eq!(opencode["route"], json!("{model}#{effort}"));
        assert!(
            opencode["plans"][0]["models"]["opencode-go/deepseek-v4.1-flash"]["5H"].is_number()
        );
    }

    #[test]
    fn lead_programs_carry_their_session_argv() {
        let table = t();
        assert_eq!(
            table.programs["claude"].session.as_ref().unwrap().new,
            Some(vec!["--session-id".into(), "{id}".into()])
        );
        assert_eq!(
            table.programs["claude"].session.as_ref().unwrap().resume,
            Some(vec!["--resume".into(), "{id}".into()])
        );
        assert_eq!(
            table.programs["opencode"].session.as_ref().unwrap().new,
            Some(vec!["--session".into(), "{id}".into()])
        );
        assert_eq!(
            table.programs["opencode"].session.as_ref().unwrap().resume,
            Some(vec!["--session".into(), "{id}".into()])
        );
        assert_eq!(
            table.programs["grok"].session.as_ref().unwrap().resume,
            Some(vec!["--resume".into(), "{id}".into()])
        );
        assert_eq!(
            table.programs["codex"]
                .session
                .as_ref()
                .unwrap()
                .continuation,
            Some(vec!["resume".into(), "--last".into()])
        );
        assert_eq!(
            table.programs["command-code"]
                .session
                .as_ref()
                .unwrap()
                .continuation,
            Some(vec!["--continue".into()])
        );
        for key in ["claude", "opencode", "grok", "codex", "command-code"] {
            assert!(!table.programs[key].run.is_empty(), "{key} has no run");
        }
        let spec = lead_spec("claude").unwrap();
        assert_eq!(spec["session"]["new"], json!(["--session-id", "{id}"]));
        assert_eq!(spec["run"], json!(["claude"]));
        assert!(lead_spec("no-such-program").is_none());
    }
}
