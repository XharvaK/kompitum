//! The interface's words, by language: `strings/<language>.json` in the app's
//! home folder or the shipped English table. Every missing key falls back to
//! English; a key nobody has is returned as written and warned about once.

use std::collections::BTreeMap;
use std::path::Path;

/// The shipped table, compiled in like the shipped themes.
const SHIPPED_EN: &str = include_str!("../defaults/strings/en.json");

struct Table {
    language: String,
    primary: BTreeMap<String, String>,
    fallback: BTreeMap<String, String>,
}

impl Table {
    fn load(language: &str, home: &Path, shipped_en: &str) -> Self {
        let fallback = parse_table(shipped_en).unwrap_or_default();
        let path = home.join("strings").join(format!("{language}.json"));
        let primary = read_table(&path)
            .or_else(|| (language == "en").then(|| fallback.clone()))
            .unwrap_or_default();
        Self {
            language: language.to_owned(),
            primary,
            fallback,
        }
    }

    fn set_language(&mut self, language: &str, home: &Path, shipped_en: &str) {
        if self.language != language {
            *self = Self::load(language, home, shipped_en);
        }
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        self.primary
            .get(key)
            .or_else(|| self.fallback.get(key))
            .map(String::as_str)
    }

    #[cfg(test)]
    fn text(&self, key: &str, vars: &[(&str, &str)]) -> String {
        self.lookup(key)
            .map(|text| fill(text, vars))
            .unwrap_or_else(|| key.to_owned())
    }

    #[cfg(test)]
    fn from_parts(language: &str, primary: &str, fallback: &str) -> Self {
        Self {
            language: language.to_owned(),
            primary: parse_table(primary).unwrap(),
            fallback: parse_table(fallback).unwrap(),
        }
    }
}

fn parse_table(text: &str) -> Result<BTreeMap<String, String>, serde_json::Error> {
    serde_json::from_str(text)
}

fn read_table(path: &Path) -> Option<BTreeMap<String, String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => match parse_table(&text) {
            Ok(table) => Some(table),
            Err(error) => {
                tracing::warn!("{}: {error}", path.display());
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!("{}: {error}", path.display());
            None
        }
    }
}

fn fill(text: &str, vars: &[(&str, &str)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

static TABLE: std::sync::Mutex<Option<Table>> = std::sync::Mutex::new(None);
static WARNED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn warn_once(key: &str) {
    let mut warned = WARNED.lock().unwrap();
    if warned.iter().any(|warned| warned == key) {
        return;
    }
    warned.push(key.to_owned());
    tracing::warn!("no text for string key {key:?}");
}

pub fn text(key: &str) -> String {
    text_with(key, &[])
}

pub fn text_with(key: &str, vars: &[(&str, &str)]) -> String {
    let value = {
        let mut table = TABLE.lock().unwrap();
        table
            .get_or_insert_with(|| Table::load("en", &crate::mods::app_dir(), SHIPPED_EN))
            .lookup(key)
            .map(str::to_owned)
    };
    match value {
        Some(value) => fill(&value, vars),
        None => {
            warn_once(key);
            key.to_owned()
        }
    }
}

/// Reloads the table when the person's language changes.
pub fn set_language(language: &str) {
    let home = crate::mods::app_dir();
    let mut table = TABLE.lock().unwrap();
    match table.as_mut() {
        Some(table) => table.set_language(language, &home, SHIPPED_EN),
        None => *table = Some(Table::load(language, &home, SHIPPED_EN)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fancy_regex::Regex;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_home() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let home = std::env::temp_dir().join(format!(
            "kompitum-strings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(home.join("strings")).unwrap();
        home
    }

    fn fixture(js: &str, table: &str) -> PathBuf {
        let root = temp_home();
        std::fs::create_dir_all(root.join("defaults/strings")).unwrap();
        std::fs::write(root.join("defaults/mod.js"), js).unwrap();
        std::fs::write(root.join("defaults/strings/en.json"), table).unwrap();
        root
    }

    #[test]
    fn a_key_the_language_lacks_comes_from_english() {
        let table = Table::from_parts(
            "tr",
            r#"{"bell.clear":"Temizle"}"#,
            r#"{"bell.clear":"Clear","bell.count":"{n} waiting"}"#,
        );
        assert_eq!(table.text("bell.clear", &[]), "Temizle");
        assert_eq!(table.text("bell.count", &[("n", "2")]), "2 waiting");
    }

    #[test]
    fn variables_fill_their_places() {
        let table = Table::from_parts("en", "{}", r#"{"welcome.hello":"Hello, {name}!"}"#);
        assert_eq!(
            table.text("welcome.hello", &[("name", "Ada")]),
            "Hello, Ada!"
        );
    }

    #[test]
    fn a_variable_without_a_value_stays_as_written() {
        let table = Table::from_parts(
            "en",
            "{}",
            r#"{"welcome.hello":"Hello, {name}; {other} waits."}"#,
        );
        assert_eq!(
            table.text("welcome.hello", &[("name", "Ada")]),
            "Hello, Ada; {other} waits."
        );
    }

    #[test]
    fn a_missing_key_is_the_key_itself() {
        let table = Table::from_parts("en", "{}", "{}");
        assert_eq!(table.text("bell.missing", &[]), "bell.missing");
        assert_eq!(table.text("bell.missing", &[("n", "2")]), "bell.missing");
    }

    #[test]
    fn a_language_switch_reloads_the_table() {
        let home = temp_home();
        std::fs::write(
            home.join("strings/tr.json"),
            r#"{"bell.clear":"Temizle"}"#,
        )
        .unwrap();
        let fallback = r#"{"bell.clear":"Clear","bell.count":"{n} waiting"}"#;
        let mut table = Table::load("en", &home, fallback);
        assert_eq!(table.text("bell.clear", &[]), "Clear");
        table.set_language("tr", &home, fallback);
        assert_eq!(table.text("bell.clear", &[]), "Temizle");
        assert_eq!(table.text("bell.count", &[("n", "3")]), "3 waiting");
        table.set_language("en", &home, fallback);
        assert_eq!(table.text("bell.clear", &[]), "Clear");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn every_key_used_in_code_is_in_the_shipped_table() {
        let fixture = fixture(
            r#"import { text } from "kompitum";
const clear = text("bell.clear");
const many = text("bell.count", { n: 2 });
"#,
            "{}",
        );
        assert_eq!(
            missing_keys(&fixture),
            vec!["bell.clear".to_owned(), "bell.count".to_owned()]
        );
        std::fs::remove_dir_all(fixture).unwrap();
        let app = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(
            missing_keys(app).is_empty(),
            "keys used in code are missing from strings/en.json: {:?}",
            missing_keys(app)
        );
    }

    #[test]
    fn every_key_in_the_shipped_table_is_used() {
        let fixture = fixture(
            r#"import { text } from "kompitum";
const clear = text("bell.clear");
"#,
            r#"{"bell.clear":"Clear","bell.unused":"Nobody says this"}"#,
        );
        assert_eq!(unused_keys(&fixture), vec!["bell.unused".to_owned()]);
        std::fs::remove_dir_all(fixture).unwrap();
        let app = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(
            unused_keys(app).is_empty(),
            "keys in strings/en.json that nothing uses: {:?}",
            unused_keys(app)
        );
    }

    #[test]
    fn the_shipped_table_is_flat_and_sorted() {
        let table = parse_table(SHIPPED_EN).expect("strings/en.json is a flat object of strings");
        let keys: Vec<&String> = table.keys().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "strings/en.json is sorted by key");
    }

    fn missing_keys(root: &Path) -> Vec<String> {
        let table = table_keys(root);
        let used = used_keys(root);
        used.iter()
            .filter(|key| !table.contains_key(*key))
            .cloned()
            .collect()
    }

    fn unused_keys(root: &Path) -> Vec<String> {
        let table = table_keys(root);
        let used = used_keys(root);
        table
            .keys()
            .filter(|key| !used.contains(key.as_str()))
            .cloned()
            .collect()
    }

    fn table_keys(root: &Path) -> BTreeMap<String, String> {
        let path = root.join("defaults/strings/en.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        parse_table(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    fn used_keys(root: &Path) -> BTreeSet<String> {
        let mut used = BTreeSet::new();
        let import = Regex::new(
            r#"(?s)import\s*\{[^}]*\btext\b[^}]*\}\s*from\s*["']kompitum["']"#,
        )
        .expect("import pattern");
        let call = Regex::new(r#"(?<![\w.$])text\(\s*"([^"]+)""#).expect("JS text pattern");
        for file in sources(root, "defaults", "js") {
            let text = std::fs::read_to_string(&file).unwrap();
            if import.is_match(&text).unwrap_or(false) {
                capture(&call, &text, &mut used);
            }
        }
        let qualified = Regex::new(r#"\bstrings::text(?:_with)?\(\s*"([^"]+)""#)
            .expect("Rust strings::text pattern");
        let bare = Regex::new(r#"(?<![\w.:])text(?:_with)?\(\s*"([^"]+)""#)
            .expect("Rust text pattern");
        for file in sources(root, "src", "rs") {
            if file.file_name().and_then(|name| name.to_str()) == Some("strings.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            capture(&qualified, &text, &mut used);
            if text.contains("use crate::strings") {
                capture(&bare, &text, &mut used);
            }
        }
        used
    }

    fn capture(pattern: &Regex, text: &str, out: &mut BTreeSet<String>) {
        for caps in pattern.captures_iter(text).flatten() {
            if let Some(key) = caps.get(1) {
                out.insert(key.as_str().to_owned());
            }
        }
    }

    fn sources(root: &Path, folder: &str, extension: &str) -> Vec<PathBuf> {
        fn walk(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
            paths.sort();
            for path in paths {
                if path.is_dir() {
                    walk(&path, extension, out);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(&root.join(folder), extension, &mut files);
        files
    }
}
