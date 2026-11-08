//! A note is Markdown with a small front matter block: `key: value` lines,
//! and `[a, b]` for lists. Only that subset of YAML is read, on purpose: every
//! agent can write it by hand, and nothing in it can surprise a parser.
//! It is the same format the Python Limen writes.

use regex::Regex;
use std::sync::OnceLock;

/// What a message may say it is.
pub const KINDS: [&str; 6] = ["request", "report", "decision", "tried", "question", "answer"];
/// The statuses a sender may mark, and the only words open/answered mean.
pub const STATUSES: [&str; 3] = ["open", "answered", "superseded"];

/// One front matter value: a line, or a list of lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
}

/// Front matter in the order the file has it: writing a note back keeps that order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta(pub Vec<(String, Value)>);

impl Meta {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Set a key, keeping its place when it is already there, as a dict does.
    pub fn set(&mut self, key: &str, value: Value) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, slot)) => *slot = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// A key as one line: a list joins with `, `.
    pub fn text(&self, key: &str) -> Option<String> {
        self.get(key).map(|value| match value {
            Value::Text(text) => text.clone(),
            Value::List(items) => items.join(", "),
        })
    }

    /// A key as a list: a single line is a one-item list, an empty one is no items.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            None => Vec::new(),
            Some(Value::List(items)) => items.clone(),
            Some(Value::Text(text)) if text.is_empty() => Vec::new(),
            Some(Value::Text(text)) => vec![text.clone()],
        }
    }

    /// Every value, in order, as the search reads them.
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.0.iter().map(|(_, value)| value)
    }
}

/// A note's front matter and body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    pub meta: Meta,
    pub body: String,
}

fn line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_-]*):\s*(.*)$").expect("a front matter line"))
}

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^#\s+(.+)$").expect("a heading"))
}

/// The note's front matter and body. A file with no front matter has empty meta
/// and all text as body; so does a block that never closes.
pub fn parse_note(text: &str) -> Note {
    let normal = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    if !normal.starts_with("---\n") {
        return Note { meta: Meta::default(), body: normal };
    }
    let Some(end) = normal[3..].find("\n---").map(|at| at + 3) else {
        return Note { meta: Meta::default(), body: normal };
    };
    let after = &normal[end + 4..];
    if !after.is_empty() && !after.starts_with('\n') {
        return Note { meta: Meta::default(), body: normal };
    }
    let mut meta = Meta::default();
    let block = if end >= 4 { &normal[4..end] } else { "" };
    for line in block.split('\n') {
        let Some(caps) = line_re().captures(line) else { continue };
        let key = caps[1].to_string();
        let raw = strip_comment(&caps[2]).trim().to_string();
        if raw.starts_with('[') && raw.ends_with(']') && raw.len() >= 2 {
            let items = raw[1..raw.len() - 1]
                .split(',')
                .map(|item| unquote(item.trim()))
                .filter(|item| !item.is_empty())
                .collect();
            meta.set(&key, Value::List(items));
        } else {
            meta.set(&key, Value::Text(unquote(&raw)));
        }
    }
    // The line break that ends the block, and one blank line after it, belong
    // to the format, not the body.
    let body = after.strip_prefix("\n\n").or_else(|| after.strip_prefix('\n')).unwrap_or(after).to_string();
    Note { meta, body }
}

/// A note's text: the front matter in the order given, then the body.
pub fn format_note(meta: &Meta, body: &str) -> String {
    let mut lines = Vec::new();
    for (key, value) in &meta.0 {
        match value {
            Value::List(items) => lines.push(format!("{key}: [{}]", items.join(", "))),
            Value::Text(text) => {
                let text = if needs_quotes(text) { quote(text) } else { text.clone() };
                lines.push(format!("{key}: {text}"));
            }
        }
    }
    format!("---\n{}\n---\n\n{}\n", lines.join("\n"), body.trim_end())
}

/// A note's status. A message with none is open; an unknown word is kept, so it
/// can be shown, not hidden.
pub fn status_of(meta: &Meta) -> String {
    meta.text("status").unwrap_or_else(|| "open".to_string())
}

/// The title: front matter `title`, else the first `# ` heading, else the file name.
pub fn title_of(note: &Note, file_name: &str) -> String {
    if let Some(title) = note.meta.text("title").filter(|title| !title.is_empty()) {
        return title;
    }
    if let Some(heading) = heading_re().captures(&note.body) {
        return heading[1].trim().to_string();
    }
    file_name.strip_suffix(".md").unwrap_or(file_name).to_string()
}

/// The file name a title earns: lower-case words, one dash between them, at most 48.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for ch in title.to_lowercase().chars() {
        if ('\u{300}'..='\u{36f}').contains(&ch) {
            continue; // a combining mark, as NFKD would set apart
        }
        let ch = base_letter(ch);
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    let out: String = out.chars().take(48).collect();
    let out = out.trim_end_matches('-');
    if out.is_empty() { "note".to_string() } else { out.to_string() }
}

/// A few composed Latin letters, so `Café` names a file `cafe` as the Python does.
fn base_letter(ch: char) -> char {
    match ch {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' => 'c',
        'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

fn strip_comment(value: &str) -> &str {
    if value.starts_with('"') || value.starts_with('\'') {
        return value;
    }
    let mut space = None;
    for (at, ch) in value.char_indices() {
        if ch == '#'
            && let Some(space) = space
        {
            return &value[..space];
        }
        space = ch.is_whitespace().then_some(at);
    }
    value
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return serde_json::from_str::<String>(value).unwrap_or_else(|_| value[1..value.len() - 1].to_string());
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].to_string();
    }
    value.to_string()
}

/// What the writer must quote: a value YAML would read as something else.
fn needs_quotes(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let first = value.chars().next().unwrap_or(' ');
    if first.is_whitespace() || "[]{}'\"#&*!|>%@`".contains(first) {
        return true;
    }
    if value.contains('\n') || value.chars().last().is_some_and(char::is_whitespace) {
        return true;
    }
    let chars: Vec<char> = value.chars().collect();
    for (at, ch) in chars.iter().enumerate() {
        if *ch == ':' && chars.get(at + 1).is_some_and(|next| next.is_whitespace()) {
            return true;
        }
        if ch.is_whitespace() && chars.get(at + 1) == Some(&'#') {
            return true;
        }
    }
    false
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_key_value_lines_lists_quotes_and_comments() {
        let note = parse_note(
            "---\r\nfrom: manager\r\nto: [opus, thinker]\r\nkind: request   # what it is\r\n\
             title: \"Key: rotate\"\r\nempty:\r\n---\r\n\r\n# Heading\r\nBody.\r\n",
        );
        assert_eq!(note.meta.text("from").as_deref(), Some("manager"));
        assert_eq!(note.meta.list("to"), ["opus", "thinker"]);
        assert_eq!(note.meta.text("kind").as_deref(), Some("request"));
        assert_eq!(note.meta.text("title").as_deref(), Some("Key: rotate"));
        assert_eq!(note.meta.text("empty").as_deref(), Some(""));
        assert_eq!(note.body, "# Heading\nBody.\n");
        assert_eq!(note.meta.list("empty"), Vec::<String>::new());
        assert_eq!(note.meta.text("to").as_deref(), Some("opus, thinker"));
    }

    #[test]
    fn treats_text_with_no_front_matter_or_an_unclosed_block_as_body() {
        assert_eq!(parse_note("# Just text\n"), Note { meta: Meta::default(), body: "# Just text\n".into() });
        assert_eq!(parse_note("---\nfrom: a\nno end\n").meta, Meta::default());
        assert_eq!(parse_note("---\n---"), Note::default(), "an empty block is still a block");
        assert_eq!(parse_note("---\n---\n"), Note::default());
    }

    #[test]
    fn writes_what_it_reads_back_the_same() {
        let mut meta = Meta::default();
        for (key, value) in [
            ("from", Value::Text("opus".into())),
            ("to", Value::List(vec!["manager".into()])),
            ("kind", Value::Text("answer".into())),
            ("title", Value::Text("Re: the key: done".into())),
            ("re", Value::Text("manager/to/opus/x.md".into())),
            ("status", Value::Text("answered".into())),
        ] {
            meta.set(key, value);
        }
        let again = parse_note(&format_note(&meta, "Done.\n\nTwo lines."));
        assert_eq!(again.meta, meta);
        assert_eq!(again.body, "Done.\n\nTwo lines.\n");
    }

    #[test]
    fn finds_the_status_and_the_title() {
        assert_eq!(status_of(&Meta::default()), "open");
        let mut meta = Meta::default();
        meta.set("status", Value::Text("answered".into()));
        assert_eq!(status_of(&meta), "answered");
        let titled = Note { meta: meta.clone(), body: "# Heading".into() };
        assert_eq!(title_of(&titled, "f.md"), "Heading");
        let mut given = Meta::default();
        given.set("title", Value::Text("Given".into()));
        assert_eq!(title_of(&Note { meta: given, body: "text".into() }, "f.md"), "Given");
        assert_eq!(title_of(&Note::default(), "2026_note.md"), "2026_note");
    }

    #[test]
    fn slugs_follow_the_python_rules() {
        assert_eq!(slug("Key: moved to the vault"), "key-moved-to-the-vault");
        assert_eq!(slug("  "), "note");
        assert_eq!(slug("Café déjà vu"), "cafe-deja-vu");
        let long = "x".repeat(60);
        assert_eq!(slug(&long).len(), 48);
    }
}
