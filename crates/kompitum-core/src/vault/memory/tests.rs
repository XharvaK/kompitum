use super::super::clock;
use super::*;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kom-memory-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn vault(root: &Path) -> Vault {
    Vault { root: root.to_path_buf(), agent: "opus".into() }
}

fn draft(title: &str, body: &str) -> MemoryDraft {
    MemoryDraft { title: title.into(), body: body.into(), ..MemoryDraft::default() }
}

fn write(root: &Path, path: &str, text: &str) {
    let full = root.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn memory(
    path: &str,
    title: &str,
    body: &str,
    project: Option<&str>,
    standing: bool,
    updated: &str,
    review_after: Option<&str>,
    status: &str,
) -> Memory {
    Memory {
        path: path.into(),
        title: title.into(),
        body: body.into(),
        project: project.map(str::to_string),
        standing,
        status: status.into(),
        review_after: review_after.map(str::to_string),
        updated: updated.into(),
    }
}

#[test]
fn close_titles_need_two_shared_words_and_half_the_new_title() {
    assert!(close_titles("Vault memory lives in git", "Memory lives in the vault"));
    assert!(!close_titles("Vault memory lives in git", "Cargo builds the daemon"));
    assert!(!close_titles("Git in vault", "Git vault"));
    assert!(close_titles("MEMORY lives in GIT", "memory lives in git"));
    assert!(!close_titles("A memory", "A memory"), "no word meets the four-letter floor");
}

#[test]
fn a_close_title_is_refused_a_distinct_one_written_and_new_keeps_both() {
    let root = scratch("close");
    let v = vault(&root);
    let now = clock::Moment::at(0, 0);
    let first = write_memory(&v, &draft("Vault memory lives in git", "One fact."), &now).unwrap();
    assert!(first.starts_with("opus/memory/") && first.ends_with("vault-memory-lives-in-git.md"), "{first}");

    let refused = write_memory(&v, &draft("Memory lives in the vault", "A near copy."), &now).unwrap_err();
    assert_eq!(refused.kind, "refused");
    assert!(refused.message.contains(&first), "{}", refused.message);
    assert!(refused.message.contains("Vault memory lives in git"), "{}", refused.message);
    assert!(
        refused.message.contains("Rewrite one with --replace <path>, or pass --new to keep both."),
        "{}",
        refused.message
    );
    assert_eq!(std::fs::read_dir(root.join("opus/memory")).unwrap().count(), 1);

    let distinct = write_memory(&v, &draft("Cargo builds in the worktree", "Other."), &now).unwrap();
    assert!(distinct.ends_with("cargo-builds-in-the-worktree.md"), "{distinct}");

    let mut keep = draft("Memory lives in the vault", "A near copy, kept.");
    keep.new = true;
    let both = write_memory(&v, &keep, &now).unwrap();
    assert!(both.ends_with("memory-lives-in-the-vault.md"), "{both}");
    assert_eq!(std::fs::read_dir(root.join("opus/memory")).unwrap().count(), 3);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn remember_writes_the_front_matter_in_order_and_replace_keeps_date_and_path() {
    let root = scratch("remember");
    let v = vault(&root);
    let first_moment = clock::Moment::at(0, 0);
    let mut full = draft("Vault memory lives in git", "One fact.\n");
    full.project = Some("myapp".into());
    full.standing = true;
    full.review_after = Some("2026-12-01".into());
    let path = write_memory(&v, &full, &first_moment).unwrap();
    let text = std::fs::read_to_string(root.join(&path)).unwrap();
    let note = notes::parse_note(&text);
    let keys: Vec<&str> = note.meta.0.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["from", "kind", "project", "title", "date", "updated", "standing", "review_after", "status"]);
    assert_eq!(note.meta.text("from").as_deref(), Some("opus"));
    assert_eq!(note.meta.text("kind").as_deref(), Some("memory"));
    assert_eq!(note.meta.text("project").as_deref(), Some("myapp"));
    assert_eq!(note.meta.text("date").as_deref(), Some("1970-01-01T00:00+00:00"));
    assert_eq!(note.meta.text("standing").as_deref(), Some("true"));
    assert_eq!(note.meta.text("review_after").as_deref(), Some("2026-12-01"));
    assert_eq!(note.meta.text("status").as_deref(), Some("open"));
    assert_eq!(note.body, "One fact.\n");

    let later = clock::Moment::at(86_400, 0);
    let mut replaced = draft("Vault memory lives in the repository", "Rewritten.");
    replaced.replace = Some(path.clone());
    let again = write_memory(&v, &replaced, &later).unwrap();
    assert_eq!(again, path, "the path stays");
    let note = notes::parse_note(&std::fs::read_to_string(root.join(&again)).unwrap());
    let keys: Vec<&str> = note.meta.0.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["from", "kind", "title", "date", "updated", "status"]);
    assert_eq!(note.meta.text("date").as_deref(), Some("1970-01-01T00:00+00:00"), "the date stays");
    assert_eq!(note.meta.text("updated").as_deref(), Some("1970-01-02T00:00+00:00"));
    assert_eq!(note.meta.text("title").as_deref(), Some("Vault memory lives in the repository"));
    assert_eq!(note.meta.text("status").as_deref(), Some("open"));
    assert_eq!(note.body, "Rewritten.\n");

    for outside in ["manager/memory/x.md", "opus/to/x.md", "opus/memory/../x.md"] {
        let mut bad = draft("A title", "A body.");
        bad.replace = Some(outside.into());
        let refused = write_memory(&v, &bad, &later).unwrap_err();
        assert!(refused.message.contains("memory"), "{outside}: {}", refused.message);
    }
    let refused = write_memory(&v, &draft(&"x".repeat(TITLE_MAX + 1), "A body."), &later).unwrap_err();
    assert!(refused.message.contains(&TITLE_MAX.to_string()), "{}", refused.message);
    let refused = write_memory(&v, &draft("A title", &"x".repeat(BODY_MAX + 1)), &later).unwrap_err();
    assert!(refused.message.contains(&BODY_MAX.to_string()), "{}", refused.message);
    let refused = write_memory(&v, &draft("One\ntwo", "A body."), &later).unwrap_err();
    assert!(refused.message.contains("one line"), "{}", refused.message);
    let mut bad_date = draft("A dated title", "A body.");
    bad_date.review_after = Some("2026-02-30".into());
    let refused = write_memory(&v, &bad_date, &later).unwrap_err();
    assert!(refused.message.contains("YYYY-MM-DD"), "{}", refused.message);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn forget_supersedes_it_and_the_packet_drops_it() {
    let root = scratch("forget");
    let v = vault(&root);
    let path = write_memory(&v, &draft("Vault memory lives in git", "One fact."), &clock::Moment::at(0, 0)).unwrap();
    forget_note(&root, &path, "The vault moved.", &clock::Moment::at(86_400, 0)).unwrap();
    let note = notes::parse_note(&std::fs::read_to_string(root.join(&path)).unwrap());
    assert_eq!(note.meta.text("status").as_deref(), Some("superseded"));
    assert_eq!(note.meta.text("why").as_deref(), Some("The vault moved."));
    assert_eq!(note.meta.text("updated").as_deref(), Some("1970-01-02T00:00+00:00"));
    assert_eq!(note.meta.text("date").as_deref(), Some("1970-01-01T00:00+00:00"));
    assert_eq!(note.body, "One fact.\n");
    let (memories, problems) = load_memories(&root, "opus").unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let packet = packet("opus", &memories, None, "2026-11-09", None, &[]);
    assert!(!packet.contains("Vault memory lives in git"), "{packet}");
    assert!(packet.contains("(opus, 0 notes)"), "{packet}");

    assert!(own_memory("opus", "manager/memory/x.md").unwrap_err().message.contains("not yours"));
    assert!(own_memory("opus", "opus/to/x.md").unwrap_err().message.contains("memory path"));
    write(&root, "opus/memory/not-memory.md", "---\nkind: request\ntitle: Not memory\n---\n\nBody.\n");
    let refused = forget_note(&root, "opus/memory/not-memory.md", "No.", &clock::Moment::at(0, 0)).unwrap_err();
    assert!(refused.message.contains("not a memory note"), "{}", refused.message);
    assert!(own_memory("opus", "opus/memory/../x.md").is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_packet_puts_standing_in_full_then_general_then_projects() {
    let memories = vec![
        memory(
            "opus/memory/rule.md",
            "Always answer in English",
            "Use English.",
            Some("myapp"),
            true,
            "2026-11-01",
            None,
            "open",
        ),
        memory(
            "opus/memory/general-new.md",
            "General newer",
            "Newest general.",
            None,
            false,
            "2026-11-05",
            Some("2026-11-08"),
            "open",
        ),
        memory(
            "opus/memory/general-old.md",
            "General older",
            "Older general.",
            None,
            false,
            "2026-11-03",
            None,
            "open",
        ),
        memory(
            "opus/memory/alpha-new.md",
            "Alpha newer",
            "Newest alpha.",
            Some("alpha"),
            false,
            "2026-11-04",
            None,
            "open",
        ),
        memory(
            "opus/memory/alpha-old.md",
            "Alpha older",
            "Older alpha.",
            Some("alpha"),
            false,
            "2026-11-02",
            None,
            "open",
        ),
        memory("opus/memory/beta.md", "Beta newest", "Newest beta.", Some("beta"), false, "2026-11-06", None, "open"),
        memory("opus/memory/gone.md", "Gone memory", "Superseded.", None, false, "2026-11-07", None, "superseded"),
    ];
    let text = packet("opus", &memories, None, "2026-11-09", None, &[]);
    assert!(text.starts_with("# Your memory (opus, 6 notes)"), "{text}");
    let standing = text.find("## Standing").unwrap();
    let index = text.find("## Index").unwrap();
    assert!(standing < index);
    assert!(text.contains("### Always answer in English\n\nUse English."), "{text}");
    assert!(!text.contains("Gone memory"), "{text}");
    let general = text.find("### General").unwrap();
    let beta = text.find("### beta").unwrap();
    let alpha = text.find("### alpha").unwrap();
    assert!(general < beta && beta < alpha, "{text}");
    assert!(text.find("General newer").unwrap() < text.find("General older").unwrap());
    assert!(text.find("Alpha newer").unwrap() < text.find("Alpha older").unwrap());
    assert!(text.contains("- General newer (opus/memory/general-new.md)"), "{text}");
    assert!(text.contains("1 due for review: kom vault memory --due"), "{text}");
    assert!(!text.contains("Over budget:"), "{text}");

    let one = packet("opus", &memories, Some("alpha"), "2026-11-09", None, &[]);
    assert!(one.contains("### Always answer in English"), "{one}");
    assert!(one.contains("### General"), "{one}");
    assert!(one.contains("### alpha"), "{one}");
    assert!(!one.contains("### beta"), "{one}");
    assert!(one.contains("(opus, 5 notes)"), "{one}");
}

#[test]
fn the_packet_keeps_whole_lines_and_points_at_the_rest() {
    let mut memories = Vec::new();
    for at in 0..300 {
        let title = format!("Memory number {at:03} with a title long enough to fill the budget");
        memories.push(memory(
            &format!("opus/memory/m-{at:03}.md"),
            &title,
            "A fact.",
            None,
            false,
            &format!("2026-11-{:02}", 1 + at % 28),
            None,
            "open",
        ));
    }
    let text = packet("opus", &memories, None, "2026-11-09", None, &[]);
    assert!(text.chars().count() <= BUDGET, "{}", text.chars().count());
    assert!(text.contains(" more: kom vault find --kind memory"), "{text}");
    assert!(
        text.contains("Over budget: merge or forget memories (kom vault remember --replace, kom vault forget)."),
        "{text}"
    );
    for line in text.lines().filter(|line| line.starts_with("- ")) {
        assert!(line.ends_with(')'), "a whole index line: {line}");
    }

    let standing = vec![
        memory(
            "opus/memory/rule.md",
            "Always answer in English",
            &"x".repeat(BUDGET + 1),
            None,
            true,
            "2026-11-01",
            None,
            "open",
        ),
        memory("opus/memory/other.md", "Other", "Other.", None, false, "2026-11-02", None, "open"),
    ];
    let text = packet("opus", &standing, None, "2026-11-09", None, &[]);
    assert!(text.contains("1 more: kom vault find --kind memory"), "{text}");
    assert!(text.contains("Over budget: merge or forget memories"), "{text}");
    assert!(text.contains(&"x".repeat(BUDGET)), "standing is never cut");
}

#[test]
fn the_due_list_is_oldest_review_after_first() {
    let memories = vec![
        memory(
            "opus/memory/late.md",
            "Late review",
            "Late.",
            Some("alpha"),
            false,
            "2026-11-01",
            Some("2026-11-05"),
            "open",
        ),
        memory(
            "opus/memory/early.md",
            "Early review",
            "Early.",
            Some("alpha"),
            false,
            "2026-11-01",
            Some("2026-11-01"),
            "open",
        ),
        memory(
            "opus/memory/future.md",
            "Future review",
            "Future.",
            Some("alpha"),
            false,
            "2026-11-01",
            Some("2026-12-01"),
            "open",
        ),
        memory("opus/memory/none.md", "No review", "None.", Some("alpha"), false, "2026-11-01", None, "open"),
    ];
    let text = due_packet("opus", &memories, None, "2026-11-09", None, &[]);
    assert!(text.starts_with("# Your memory due for review (opus, 2 notes)"), "{text}");
    let early = text.find("Early review").unwrap();
    let late = text.find("Late review").unwrap();
    assert!(early < late, "{text}");
    assert!(!text.contains("Future review") && !text.contains("No review"), "{text}");
    let none = due_packet("opus", &memories, Some("other"), "2026-11-09", None, &[]);
    assert!(none.contains("(opus, 0 notes)"), "{none}");
    assert!(none.contains("Nothing is due for review."), "{none}");
}

#[test]
fn a_damaged_memory_is_named_in_the_problems_line() {
    let root = scratch("damaged");
    write(&root, "opus/memory/bad.md", "---\nkind: request\ntitle: Not a memory\n---\n\nBody.\n");
    write(&root, "opus/memory/unknown.md", "---\nkind: memory\nstatus: maybe\ntitle: Unknown\n---\n\nBody.\n");
    write(&root, "opus/memory/bad-date.md", "---\nkind: memory\nreview_after: soon\ntitle: Bad date\n---\n\nBody.\n");
    let (memories, problems) = load_memories(&root, "opus").unwrap();
    assert_eq!(memories.len(), 0);
    assert_eq!(problems.len(), 3, "{problems:?}");
    let packet = packet("opus", &memories, None, "2026-11-09", None, &problems);
    assert!(packet.contains("Problems:"), "{packet}");
    for path in ["opus/memory/bad.md", "opus/memory/unknown.md", "opus/memory/bad-date.md"] {
        assert!(packet.contains(path), "{path} not in {packet}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
