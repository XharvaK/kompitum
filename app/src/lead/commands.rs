//! The lead's slash commands: Claude Code's project and user skills and
//! commands, the installed plugins', and the built-ins table. A name comes
//! from the first source that has it; the list is sorted by name.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// The built-ins table shipped with the app (`app/defaults/claude-commands.json`).
pub const BUILTINS: &str = include_str!("../../defaults/claude-commands.json");

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub name: String,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub description: String,
    pub source: String,
}

struct Front {
    name: Option<String>,
    description: Option<String>,
    hint: Option<String>,
    user_invocable: bool,
}

impl Default for Front {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            hint: None,
            user_invocable: true,
        }
    }
}

#[derive(Default, Deserialize)]
struct Builtin {
    name: String,
    #[serde(default)]
    hint: String,
    #[serde(default)]
    description: String,
}

/// One cached `/` list and the modification times that produced it.
struct CachedCommands {
    home: PathBuf,
    folder: PathBuf,
    builtins: String,
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    commands: Vec<Command>,
}

static COMMANDS_CACHE: Mutex<Vec<CachedCommands>> = Mutex::new(Vec::new());
const CACHE_ENTRIES: usize = 8;

/// Every command the lead's `/` list can offer, in name order: the lead
/// folder's skills and commands, the user's, the installed plugins', then the
/// built-ins table. A name already taken by an earlier source is skipped.
/// The result is cached per lead folder until a mtime under one of its roots changes.
pub fn lead_commands(home: &Path, folder: &Path, builtins: &str) -> Vec<Command> {
    let plugins = installed_plugins(home);
    let mut roots = vec![
        folder.join(".claude/skills"),
        folder.join(".claude/commands"),
        home.join(".claude/skills"),
        home.join(".claude/commands"),
        home.join(".claude/plugins/installed_plugins.json"),
    ];
    for (_, install) in &plugins {
        roots.push(install.join("skills"));
        roots.push(install.join("commands"));
    }
    let stamps = root_stamps(&roots);
    if let Ok(cache) = COMMANDS_CACHE.lock()
        && let Some(hit) = cache.iter().find(|entry| {
            entry.home == home
                && entry.folder == folder
                && entry.builtins == builtins
                && entry.stamps == stamps
        })
    {
        return hit.commands.clone();
    }
    let commands = scan_commands(home, folder, builtins, &plugins);
    if let Ok(mut cache) = COMMANDS_CACHE.lock() {
        cache.retain(|entry| entry.home != home || entry.folder != folder);
        cache.push(CachedCommands {
            home: home.to_path_buf(),
            folder: folder.to_path_buf(),
            builtins: builtins.to_owned(),
            stamps,
            commands: commands.clone(),
        });
        if cache.len() > CACHE_ENTRIES {
            cache.remove(0);
        }
    }
    commands
}

fn scan_commands(
    home: &Path,
    folder: &Path,
    builtins: &str,
    plugins: &[(String, PathBuf)],
) -> Vec<Command> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    read_claude_dir(
        &folder.join(".claude"),
        "project",
        None,
        &mut seen,
        &mut out,
    );
    read_claude_dir(&home.join(".claude"), "user", None, &mut seen, &mut out);
    for (plugin, install) in plugins {
        let prefix = plugin.as_str();
        read_skills(
            &install.join("skills"),
            "plugin",
            Some(prefix),
            &mut seen,
            &mut out,
        );
        read_command_files(
            &install.join("commands"),
            "plugin",
            Some(prefix),
            &mut seen,
            &mut out,
        );
    }

    let builtins: Vec<Builtin> = serde_json::from_str(builtins).unwrap_or_default();
    for builtin in builtins {
        let name = clean_name(&builtin.name);
        if name.is_empty() {
            continue;
        }
        push(
            &mut seen,
            &mut out,
            Command {
                name,
                hint: builtin.hint.trim().to_owned(),
                description: describe(&builtin.description),
                source: "built-in".into(),
            },
        );
    }

    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

fn root_stamps(roots: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut out = Vec::new();
    for root in roots {
        stamp_tree(root, 0, &mut out);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn stamp_tree(path: &Path, depth: usize, out: &mut Vec<(PathBuf, Option<SystemTime>)>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        out.push((path.to_path_buf(), None));
        return;
    };
    out.push((path.to_path_buf(), meta.modified().ok()));
    if depth >= 8 || !meta.is_dir() || is_link(path, &meta.file_type()) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let child = entry.path();
        if is_link(&child, &kind) {
            continue;
        }
        stamp_tree(&child, depth + 1, out);
    }
}

/// Whether `path` is a symbolic link or a Windows junction/reparse point.
fn is_link(path: &Path, kind: &std::fs::FileType) -> bool {
    if kind.is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            return meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        }
    }
    false
}

fn linked(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| is_link(path, &meta.file_type()))
}

fn read_claude_dir(
    root: &Path,
    source: &str,
    prefix: Option<&str>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Command>,
) {
    read_skills(&root.join("skills"), source, prefix, seen, out);
    read_command_files(&root.join("commands"), source, prefix, seen, out);
}

fn read_skills(
    dir: &Path,
    source: &str,
    prefix: Option<&str>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Command>,
) {
    if linked(dir) {
        return;
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let kind = entry.file_type().ok()?;
            (!is_link(&path, &kind) && kind.is_dir()).then(|| path.join("SKILL.md"))
        })
        .filter(|path| {
            std::fs::symlink_metadata(path).is_ok_and(|meta| {
                !is_link(path, &meta.file_type()) && meta.file_type().is_file()
            })
        })
        .collect();
    entries.sort();
    for path in entries {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (front, _) = split_front(&text);
        let front = front.unwrap_or_default();
        if !front.user_invocable {
            continue;
        }
        let folder_name = path
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = front
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(folder_name);
        let Some(name) = prefixed(prefix, &name) else {
            continue;
        };
        push(
            seen,
            out,
            Command {
                name,
                hint: front.hint.unwrap_or_default(),
                description: front
                    .description
                    .map(|text| describe(&text))
                    .unwrap_or_default(),
                source: source.into(),
            },
        );
    }
}

fn read_command_files(
    dir: &Path,
    source: &str,
    prefix: Option<&str>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Command>,
) {
    if linked(dir) {
        return;
    }
    let mut files = Vec::new();
    command_files(dir, 0, &mut files);
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (front, body) = split_front(&text);
        let has_front = front.is_some();
        let front = front.unwrap_or_default();
        if !front.user_invocable {
            continue;
        }
        let Some(name) = command_name(dir, &path) else {
            continue;
        };
        let Some(name) = prefixed(prefix, &name) else {
            continue;
        };
        let description = match (front.description, has_front) {
            (Some(description), _) => describe(&description),
            (None, true) => String::new(),
            (None, false) => describe(&first_line(&body)),
        };
        push(
            seen,
            out,
            Command {
                name,
                hint: front.hint.unwrap_or_default(),
                description,
                source: source.into(),
            },
        );
    }
}

fn command_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(PathBuf, std::fs::FileType)> = entries
        .flatten()
        .filter_map(|entry| Some((entry.path(), entry.file_type().ok()?)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, kind) in entries {
        if is_link(&path, &kind) {
            continue;
        }
        if kind.is_dir() {
            if depth < 8 {
                command_files(&path, depth + 1, out);
            }
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            out.push(path);
        }
    }
}

fn command_name(commands: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(commands).ok()?;
    let mut parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let last = parts.last_mut()?;
    if let Some(stem) = last.strip_suffix(".md") {
        *last = stem.to_owned();
    }
    Some(parts.join(":"))
}

fn installed_plugins(home: &Path) -> Vec<(String, PathBuf)> {
    let path = home.join(".claude/plugins/installed_plugins.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(plugins) = value.get("plugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut list: Vec<(String, PathBuf)> = plugins
        .iter()
        .filter_map(|(key, installs)| {
            let plugin = key.split('@').next().unwrap_or(key).to_owned();
            let install = installs.as_array()?.first()?;
            let path = install.get("installPath")?.as_str()?;
            Some((plugin, PathBuf::from(path)))
        })
        .collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list
}

fn split_front(text: &str) -> (Option<Front>, String) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return (None, text.to_owned());
    }
    let Some(close) = lines
        .iter()
        .skip(1)
        .position(|line| line.trim() == "---")
        .map(|index| index + 1)
    else {
        return (None, text.to_owned());
    };
    let mut front = Front {
        user_invocable: true,
        ..Default::default()
    };
    let block = &lines[1..close];
    let mut at = 0;
    while at < block.len() {
        let line = block[at];
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            at += 1;
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            at += 1;
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let (value, next) = if matches!(value, ">" | ">-" | "|" | "|-") {
            let folded = value.starts_with('>');
            let mut parts = Vec::new();
            let mut end = at + 1;
            while end < block.len() && (block[end].starts_with(' ') || block[end].starts_with('\t'))
            {
                parts.push(block[end].trim().to_owned());
                end += 1;
            }
            (parts.join(if folded { " " } else { "\n" }), end)
        } else {
            (unquote(value), at + 1)
        };
        match key {
            "name" => front.name = Some(value),
            "description" => front.description = Some(value),
            "argument-hint" => front.hint = Some(value),
            "user-invocable" if value.eq_ignore_ascii_case("false") => front.user_invocable = false,
            _ => {}
        }
        at = next;
    }
    (Some(front), lines[close + 1..].join("\n"))
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return value[1..value.len() - 1].to_owned();
    }
    value.to_owned()
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn describe(text: &str) -> String {
    let first = first_line(text);
    if first.chars().count() <= 120 {
        first
    } else {
        let mut cut: String = first.chars().take(119).collect();
        cut.push('…');
        cut
    }
}

fn clean_name(name: &str) -> String {
    name.trim().trim_start_matches('/').trim().to_owned()
}

fn prefixed(prefix: Option<&str>, name: &str) -> Option<String> {
    let name = clean_name(name);
    if name.is_empty() {
        return None;
    }
    Some(match prefix {
        Some(prefix) => format!("{prefix}:{name}"),
        None => name,
    })
}

fn push(seen: &mut BTreeSet<String>, out: &mut Vec<Command>, command: Command) {
    if !seen.insert(command.name.clone()) {
        return;
    }
    out.push(command);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "kompitum-commands-{name}-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn find<'a>(list: &'a [Command], name: &str) -> &'a Command {
        list.iter()
            .find(|command| command.name == name)
            .unwrap_or_else(|| panic!("missing {name}: {list:?}"))
    }

    #[test]
    fn every_source_is_read_and_the_earlier_source_wins() {
        let home = Temp::new("home");
        let folder = Temp::new("folder");
        folder.write(
            ".claude/skills/deploy/SKILL.md",
            "---\nname: deploy\ndescription: \"Deploy the app\"\nargument-hint: \"[env]\"\n---\n",
        );
        folder.write(
            ".claude/skills/shared/SKILL.md",
            "---\nname: shared\ndescription: The project's\n---\n",
        );
        home.write(
            ".claude/skills/shared/SKILL.md",
            "---\nname: shared\ndescription: The user's\n---\n",
        );
        home.write(
            ".claude/skills/check/SKILL.md",
            "---\ndescription: >-\n  Check the tests\n  and the lint\n---\n",
        );
        home.write(
            ".claude/commands/hello.md",
            "---\ndescription: 'Say hello.'\n---\n",
        );
        let plugin = Temp::new("plugin");
        plugin.write(
            "skills/status/SKILL.md",
            "---\nname: status\ndescription: Show the plugin's status\n---\n",
        );
        plugin.write("commands/run/release.md", "Cut a release.\n");
        home.write(
            ".claude/plugins/installed_plugins.json",
            &serde_json::json!({
                "version": 2,
                "plugins": { "acme@market": [{ "installPath": plugin.0.to_string_lossy() }] }
            })
            .to_string(),
        );
        let builtins = r#"[
        {"name": "compact", "hint": "[instructions]", "description": "Summarize the conversation."},
        {"name": "deploy", "description": "A built-in deploy that loses to the project"}
    ]"#;

        let list = lead_commands(&home.0, &folder.0, builtins);

        let deploy = find(&list, "deploy");
        assert_eq!(deploy.source, "project");
        assert_eq!(deploy.hint, "[env]");
        assert_eq!(deploy.description, "Deploy the app");
        assert_eq!(find(&list, "shared").source, "project");
        assert_eq!(find(&list, "shared").description, "The project's");
        assert_eq!(find(&list, "check").source, "user");
        assert_eq!(
            find(&list, "check").description,
            "Check the tests and the lint"
        );
        assert_eq!(find(&list, "hello").source, "user");
        assert_eq!(find(&list, "hello").description, "Say hello.");
        assert_eq!(find(&list, "acme:status").source, "plugin");
        assert_eq!(find(&list, "acme:run:release").source, "plugin");
        assert_eq!(
            find(&list, "acme:run:release").description,
            "Cut a release."
        );
        assert_eq!(find(&list, "compact").source, "built-in");
        assert_eq!(
            list.iter()
                .filter(|command| command.name == "deploy")
                .count(),
            1,
            "a name is taken once: {list:?}"
        );
        let names: Vec<&str> = list.iter().map(|command| command.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(names, sorted, "the list is sorted by name");
    }

    #[test]
    fn a_nested_command_is_named_by_its_path() {
        let home = Temp::new("home");
        let folder = Temp::new("folder");
        folder.write(".claude/commands/git/push.md", "Push the branch.\n");
        let list = lead_commands(&home.0, &folder.0, "[]");
        assert_eq!(find(&list, "git:push").description, "Push the branch.");
    }

    #[test]
    fn a_plugin_command_is_named_with_its_plugin() {
        let home = Temp::new("home");
        let folder = Temp::new("folder");
        let plugin = Temp::new("plugin");
        plugin.write("commands/git/push.md", "---\ndescription: Push it\n---\n");
        home.write(
            ".claude/plugins/installed_plugins.json",
            &serde_json::json!({
                "version": 2,
                "plugins": { "acme@market": [{ "installPath": plugin.0.to_string_lossy() }] }
            })
            .to_string(),
        );
        let list = lead_commands(&home.0, &folder.0, "[]");
        assert_eq!(find(&list, "acme:git:push").source, "plugin");
        assert_eq!(find(&list, "acme:git:push").description, "Push it");
    }

    #[test]
    fn quoted_and_folded_descriptions_lose_their_markers() {
        let home = Temp::new("home");
        let folder = Temp::new("folder");
        folder.write(
            ".claude/skills/folded/SKILL.md",
            "---\nname: folded\ndescription: >\n  One line\n  joined\n---\n",
        );
        folder.write(
            ".claude/skills/quoted/SKILL.md",
            "---\nname: quoted\ndescription: \"Quoted, kept\"\n---\n",
        );
        folder.write(
            ".claude/skills/literal/SKILL.md",
            "---\nname: literal\ndescription: |\n  First line\n  Second line\n---\n",
        );
        let list = lead_commands(&home.0, &folder.0, "[]");
        assert_eq!(find(&list, "folded").description, "One line joined");
        assert_eq!(find(&list, "quoted").description, "Quoted, kept");
        assert_eq!(find(&list, "literal").description, "First line");
    }

    #[test]
    fn user_invocable_false_is_left_out() {
        let home = Temp::new("home");
        let folder = Temp::new("folder");
        folder.write(
            ".claude/skills/hidden/SKILL.md",
            "---\nname: hidden\ndescription: Only Claude\nuser-invocable: false\n---\n",
        );
        folder.write(
            ".claude/commands/hidden-command.md",
            "---\ndescription: Only Claude\nuser-invocable: false\n---\n",
        );
        let list = lead_commands(&home.0, &folder.0, "[]");
        assert!(
            list.iter().all(|command| command.name != "hidden"),
            "{list:?}"
        );
        assert!(
            list.iter().all(|command| command.name != "hidden-command"),
            "{list:?}"
        );
    }

    #[test]
    fn a_missing_plugins_file_is_no_plugins_and_never_an_error() {
        let home = Temp::new("home");
        let folder = Temp::new("folder");
        let list = lead_commands(&home.0, &folder.0, r#"[{"name":"clear"}]"#);
        assert_eq!(find(&list, "clear").source, "built-in");
    }

    #[test]
    fn the_shipped_builtins_parse_with_unique_names_and_the_essential_commands() {
        let list: Vec<Builtin> =
            serde_json::from_str(BUILTINS).expect("claude-commands.json parses");
        let mut names = BTreeSet::new();
        for builtin in &list {
            assert!(!builtin.name.is_empty());
            assert!(
                names.insert(builtin.name.clone()),
                "duplicate built-in {}",
                builtin.name
            );
        }
        for essential in ["compact", "model", "clear"] {
            assert!(
                names.contains(essential),
                "the built-ins include {essential}: {names:?}"
            );
        }
    }

    #[test]
    fn a_description_keeps_its_first_line_and_stops_at_120_characters() {
        let home = Temp::new("home");
        let folder = Temp::new("folder");
        let long = "x".repeat(150);
        folder.write(
            ".claude/skills/long/SKILL.md",
            &format!("---\nname: long\ndescription: |\n  {long}\n  second\n---\n"),
        );
        let list = lead_commands(&home.0, &folder.0, "[]");
        let description = &find(&list, "long").description;
        assert_eq!(description.chars().count(), 120);
        assert!(description.ends_with('…'), "{description}");
    }

    #[test]
    fn an_unchanged_tree_is_answered_from_the_cache_and_a_new_root_mtime_rescans() {
        let home = Temp::new("cache-home");
        let folder = Temp::new("cache-folder");
        folder.write(".claude/commands/hello.md", "First.\n");
        let list = lead_commands(&home.0, &folder.0, "[]");
        assert_eq!(find(&list, "hello").description, "First.");

        let path = folder.0.join(".claude/commands/hello.md");
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, "Second.\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let cached = lead_commands(&home.0, &folder.0, "[]");
        assert_eq!(
            find(&cached, "hello").description,
            "First.",
            "with every mtime unchanged the cached list answers"
        );

        std::thread::sleep(std::time::Duration::from_millis(20));
        folder.write(".claude/commands/another.md", "Another.\n");
        let fresh = lead_commands(&home.0, &folder.0, "[]");
        assert_eq!(find(&fresh, "hello").description, "Second.");
        assert_eq!(find(&fresh, "another").description, "Another.");
    }

    #[test]
    fn command_files_stop_at_depth_eight() {
        let home = Temp::new("depth-home");
        let folder = Temp::new("depth-folder");
        for level in 0..=9 {
            let mut dir = folder.0.join(".claude/commands");
            for depth in 0..=level {
                dir.push(format!("d{depth}"));
            }
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("at{level}.md")), format!("Depth {level}.\n")).unwrap();
        }
        let list = lead_commands(&home.0, &folder.0, "[]");
        assert!(
            list.iter().any(|command| command.name.ends_with(":at7")),
            "a file in the eighth directory is read: {list:?}"
        );
        assert!(
            list.iter().all(|command| !command.name.ends_with(":at8")),
            "nothing below depth 8 is read: {list:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_followed() {
        use std::os::unix::fs::symlink;
        let home = Temp::new("link-home");
        let folder = Temp::new("link-folder");
        let outside = Temp::new("link-outside");
        outside.write("escape.md", "Outside.\n");
        let commands = folder.0.join(".claude/commands");
        std::fs::create_dir_all(&commands).unwrap();
        symlink(outside.0.join("escape.md"), commands.join("escape.md")).unwrap();
        symlink(&outside.0, commands.join("outside")).unwrap();
        let list = lead_commands(&home.0, &folder.0, "[]");
        assert!(
            list.iter().all(|command| command.name != "escape" && command.name != "outside:escape"),
            "a link is not walked: {list:?}"
        );
    }
}
