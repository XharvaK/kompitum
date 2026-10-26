//! Cumulative release notes and this launch's update range. Reading notes never
//! changes the last successful version; only expanded spotlights are recorded.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Note {
    pub version: String,
    pub date: String,
    pub items: Vec<Item>,
    pub spotlight: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Item {
    pub headline: String,
    pub detail: String,
    pub fixed: bool,
    pub media: Vec<String>,
}

pub fn parse(text: &str) -> Vec<Note> {
    let mut notes: Vec<Note> = Vec::new();
    let mut fixed = false;
    for line in text.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("## [") {
            if let Some((version, date)) = heading.split_once("] - ") {
                notes.push(Note { version: version.into(), date: date.into(), ..Note::default() });
                fixed = false;
            }
        } else if let Some(note) = notes.last_mut() {
            if line == "<!-- spotlight -->" {
                note.spotlight = true;
            } else if line.starts_with("### ") {
                fixed = line == "### Fixed";
            } else if let Some(url) = line.strip_prefix("![").and_then(|s| s.split_once("](")).and_then(|(_, s)| s.strip_suffix(')')) {
                if url.starts_with("https://") && let Some(item) = note.items.last_mut() {
                    item.media.push(url.into());
                }
            } else if let Some(text) = line.strip_prefix("- ") {
                let (headline, detail) = text.strip_prefix("**").and_then(|s| s.split_once("**"))
                    .map_or(("", text), |(headline, detail)| (headline, detail.trim()));
                note.items.push(Item { headline: headline.into(), detail: detail.into(), fixed, ..Item::default() });
            }
        }
    }
    notes
}

#[derive(Default, Deserialize, Serialize)]
struct Record {
    last_run: Option<String>,
    #[serde(default)]
    presented: Vec<String>,
}

struct Launch {
    record: Record,
    previous: Option<String>,
    running: String,
    changed: bool,
}

impl Launch {
    fn new(mut record: Record, running: &str, first_install: bool) -> Self {
        let previous = record.last_run.clone();
        let changed = !first_install && previous.as_deref().is_none_or(|v| crate::update::newer(running, v));
        record.last_run = Some(running.into());
        Self { record, previous, running: running.into(), changed }
    }

    fn range(&self, notes: &[Note], target: Option<&str>) -> Vec<Note> {
        let (lower, upper) = match target {
            Some(target) => (Some(self.running.as_str()), target),
            None if self.previous.as_deref().is_some_and(|v| crate::update::newer(v, &self.running)) => return Vec::new(),
            // Since the last version that ran; with nothing new, this version's own notes.
            None => (self.previous.as_deref().filter(|v| crate::update::newer(&self.running, v)), self.running.as_str()),
        };
        notes.iter().filter(|n| {
            !crate::update::newer(&n.version, upper)
                && lower.map_or(n.version == upper, |lower| crate::update::newer(&n.version, lower))
        }).cloned().collect()
    }
}

static LAUNCH: Mutex<Option<Launch>> = Mutex::new(None);
static FETCHED: Mutex<Option<Vec<Note>>> = Mutex::new(None);

fn path(name: &str) -> std::path::PathBuf {
    crate::mods::app_dir().join(name)
}

fn save(record: &Record) {
    let result = serde_json::to_vec_pretty(record).map_err(std::io::Error::other).and_then(|bytes| {
        std::fs::create_dir_all(crate::mods::app_dir())?;
        let file = path("update-notes.json");
        let temp = file.with_extension("json.new");
        std::fs::write(&temp, bytes)?;
        std::fs::rename(temp, file)
    });
    if let Err(e) = result { tracing::warn!("update notes state: {e}"); }
}

/// Called once the live window has connected and synchronized successfully.
pub fn started(first_install: bool) {
    let mut launch = LAUNCH.lock().unwrap();
    if launch.is_some() { return; }
    let record = std::fs::read(path("update-notes.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let state = Launch::new(record, env!("CARGO_PKG_VERSION"), first_install);
    save(&state.record);
    *launch = Some(state);
    std::thread::spawn(|| fetch_media(&all()));
}

pub fn cache(text: &str) {
    let notes = parse(text);
    if notes.is_empty() { return; }
    *FETCHED.lock().unwrap() = Some(notes.clone());
    std::thread::spawn(move || fetch_media(&notes));
    if let Err(e) = std::fs::write(path("update-notes-cache.md"), text) {
        tracing::warn!("update notes cache: {e}");
    }
}

fn all() -> Vec<Note> {
    let mut notes = parse(include_str!("../CHANGELOG.md"));
    let fetched = FETCHED.lock().unwrap().clone().unwrap_or_else(|| {
        std::fs::read_to_string(path("update-notes-cache.md")).map(|s| parse(&s)).unwrap_or_default()
    });
    // The running binary's curated notes win over cached copies of its releases.
    for note in fetched {
        if !notes.iter().any(|n| n.version == note.version) { notes.push(note); }
    }
    notes.sort_by(|a, b| if crate::update::newer(&a.version, &b.version) { std::cmp::Ordering::Less } else if crate::update::newer(&b.version, &a.version) { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
    notes
}

fn media_path(url: &str) -> std::path::PathBuf {
    path("update-notes-media").join(crate::update::sha256(url.as_bytes()))
}

fn fetch_media(notes: &[Note]) {
    let dir = path("update-notes-media");
    if std::fs::create_dir_all(&dir).is_err() { return; }
    for url in notes.iter().flat_map(|n| &n.items).flat_map(|item| &item.media) {
        let file = media_path(url);
        if file.is_file() { continue; }
        // The same URL may be in both the startup and newly fetched feed.
        let temp = file.with_extension(format!("{:?}.new", std::thread::current().id()));
        let result = crate::spawn::command(crate::update::system("curl"))
            .args(["-fsSL", "--max-time", "30", "--max-filesize", "16777216", "-o"])
            .arg(&temp).arg(url).stdin(std::process::Stdio::null()).output();
        if result.is_ok_and(|out| out.status.success()) {
            let _ = std::fs::rename(&temp, &file);
        }
        let _ = std::fs::remove_file(temp);
    }
}

pub fn notes(target: Option<&str>) -> Vec<Note> {
    let mut notes = all();
    for note in &mut notes {
        for item in &mut note.items {
            item.media = item.media.iter().map(|url| media_path(url)).filter(|p| p.is_file()).filter_map(|p| p.strip_prefix(crate::mods::app_dir()).ok().map(|p| p.to_string_lossy().into_owned())).collect();
        }
    }
    LAUNCH.lock().unwrap().as_ref().map(|l| l.range(&notes, target)).unwrap_or_default()
}

pub fn spotlights() -> Vec<Note> {
    let notes = notes(None);
    let launch = LAUNCH.lock().unwrap();
    let Some(l) = launch.as_ref().filter(|l| l.changed) else { return Vec::new() };
    notes.into_iter().filter(|n| n.spotlight && !l.record.presented.contains(&n.version)).collect()
}

pub fn changed() -> bool {
    LAUNCH.lock().unwrap().as_ref().is_some_and(|l| l.changed)
}

pub fn presented(versions: &[String]) {
    let known = all();
    if let Some(l) = LAUNCH.lock().unwrap().as_mut() {
        for version in versions {
            if known.iter().any(|n| n.version == *version && n.spotlight) && !l.record.presented.contains(version) {
                l.record.presented.push(version.clone());
            }
        }
        save(&l.record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<Note> {
        parse("## [1.8.0] - 2026-10-23\n<!-- spotlight -->\n- New thing\n![demo](https://releases.herdr.dev/demo.png)\n## [1.7.0] - 2026-10-22\n- Another thing\n## [1.5.0] - 2026-10-20\n- Old thing\n")
    }
    fn from(version: Option<&str>, running: &str, fresh: bool) -> Launch {
        Launch::new(Record { last_run: version.map(str::to_owned), ..Record::default() }, running, fresh)
    }
    #[test]
    fn first_install_only_welcomes() {
        let l = from(None, "1.8.0", true);
        assert!(!l.changed);
        assert_eq!(l.record.last_run.as_deref(), Some("1.8.0"));
    }
    #[test]
    fn skipped_versions_and_pending_range() {
        let l = from(Some("1.5.0"), "1.8.0", false);
        assert_eq!(l.range(&fixture(), None).iter().map(|n| n.version.as_str()).collect::<Vec<_>>(), ["1.8.0", "1.7.0"]);
        let before = from(Some("1.5.0"), "1.5.0", false);
        assert_eq!(before.range(&fixture(), Some("1.8.0")).len(), 2);
        assert_eq!(before.record.last_run.as_deref(), Some("1.5.0"));
    }
    #[test]
    fn downgrade_records_without_replaying_future_notes() {
        let l = from(Some("1.8.0"), "1.5.0", false);
        assert!(!l.changed);
        assert!(l.range(&fixture(), None).is_empty());
        assert_eq!(l.record.last_run.as_deref(), Some("1.5.0"));
    }
    #[test]
    fn missing_baseline_is_current_release_only() {
        let l = from(None, "1.8.0", false);
        assert!(l.changed);
        assert_eq!(l.range(&fixture(), None).len(), 1);
        // Nothing new since the last run: Settings still shows this version's notes.
        let same = from(Some("1.8.0"), "1.8.0", false);
        assert!(!same.changed);
        assert_eq!(same.range(&fixture(), None).iter().map(|n| n.version.as_str()).collect::<Vec<_>>(), ["1.8.0"]);
    }
    #[test]
    fn spotlight_presented_before_restart_does_not_replay() {
        let mut before = from(Some("1.5.0"), "1.5.0", false);
        before.record.presented.push("1.8.0".into());
        let after = Launch::new(before.record, "1.8.0", false);
        assert!(after.changed);
        assert!(!after.range(&fixture(), None).iter().any(|n| n.spotlight && !after.record.presented.contains(&n.version)));
    }
    #[test]
    fn sections_and_item_media() {
        let notes = parse("## [1.8.0] - 2026-10-23\n### New\n- **A headline.** Its detail.\n![demo](https://example.com/demo.png)\n- **Another headline.** More detail.\n![bad](http://example.com/no.png)\n### Fixed\n- A short fix.\n## [1.7.0] - 2026-10-22\n- Plain item.\n");
        let items = &notes[0].items;
        assert_eq!(items[0].headline, "A headline.");
        assert_eq!(items[0].detail, "Its detail.");
        assert_eq!(items[0].media, ["https://example.com/demo.png"]);
        assert!(items[1].media.is_empty());
        assert!(items[2].fixed);
        assert_eq!(items[2].detail, "A short fix.");
        assert!(!notes[1].items[0].fixed);
        assert_eq!(notes[1].items[0].detail, "Plain item.");
    }
    #[test]
    fn curated_markdown_and_remote_media() {
        let notes = fixture();
        assert!(notes[0].spotlight);
        assert_eq!(notes[0].items[0].media, ["https://releases.herdr.dev/demo.png"]);
        assert_eq!(notes[1].items[0].detail, "Another thing");
        assert!(notes[1].items[0].headline.is_empty());
        assert!(!notes[1].items[0].fixed);
        assert_eq!(parse(include_str!("../CHANGELOG.md"))[0].version, env!("CARGO_PKG_VERSION"));
    }
}
