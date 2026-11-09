//! Render tests for the rework (audit/REWORK-2026-11-09.md): see the parent module for the helpers.
#![allow(unused_imports)]

use super::*;

fn opened_owner_note() -> Value {
    let mut p = props();
    p["note"] = json!("manager/to/owner/2026-11-08-please-review.md");
    p
}

/// The reader draws the note's Markdown body, not only its title and meta line.
#[gpui::test]
fn the_reader_draws_the_notes_body(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/mailbox",
        fixture(true, opened_owner_note(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("A body from the vault"), "{tree}");
    let body = tree
        .lines()
        .find(|line| line.contains("mail-body-text"))
        .unwrap_or_else(|| panic!("the body is not a Markdown view: {tree}"));
    assert!(body.contains("A body from the vault"), "{body}");
}

/// The front matter block is not part of the body the reader draws.
#[gpui::test]
fn the_reader_leaves_the_front_matter_out_of_the_body(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/mailbox",
        fixture(true, opened_owner_note(), updates("current", "current")),
        Some(
            "this.readBody = () => { this.body = '---\\ntitle: Hidden title\\nkind: request\\n---\\n\\n# The body heading\\n\\nThe body under it.'; };",
        ),
    )
    .unwrap();
    assert!(tree.contains("The body heading"), "{tree}");
    assert!(tree.contains("The body under it"), "{tree}");
    assert!(!tree.contains("Hidden title"), "{tree}");
    assert!(!tree.contains("title:"), "{tree}");
}

/// A note with no text says so instead of drawing an empty body.
#[gpui::test]
fn the_reader_says_when_a_note_has_no_text(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/mailbox",
        fixture(true, opened_owner_note(), updates("current", "current")),
        Some("this.readBody = () => { this.body = ''; };"),
    )
    .unwrap();
    assert!(tree.contains("This note has no text."), "{tree}");
}

/// The header and the list never show the note's file path.
#[gpui::test]
fn the_reader_shows_no_file_path(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/mailbox",
        fixture(true, opened_owner_note(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(
        !tree.contains("manager/to/owner/2026-11-08-please-review.md"),
        "{tree}"
    );
    assert!(!tree.contains("/fixture/vault/"), "{tree}");
}

/// A note with no recipient reads only its sender, without a dangling arrow.
#[gpui::test]
fn a_note_without_a_recipient_reads_only_its_sender(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    // Set the list before the vault's answer lands; the synchronous description
    // draws this state, and the reader still reads the fixture's body.
    let tree = describe(
        cx,
        &staged,
        "frame/mailbox",
        true,
        props(),
        updates("current", "current"),
        Some(
            "this.vault = '/fixture/vault'; \
             this.loaded = true; \
             this.needs = [{ path: 'note.md', from: 'Scout', to: '', kind: 'request', \
                            title: 'A note from Scout', date: '2026-11-08T09:00:00+03:00', read_by: [] }]; \
             this.all = []; \
             this.select(this.needs[0], cx);",
        ),
    )
    .unwrap();
    assert!(tree.contains("Scout"), "{tree}");
    assert!(!tree.contains("Scout →"), "a dangling arrow: {tree}");
}
