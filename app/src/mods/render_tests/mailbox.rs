//! Render tests for the rework (audit/REWORK-2026-11-09.md): see the parent module for the helpers.
#![allow(unused_imports)]

use super::*;
use gpui::{Modifiers, MouseButton};

fn opened_owner_note() -> Value {
    let mut p = props();
    p["note"] = json!("manager/to/owner/2026-11-08-please-review.md");
    p
}

/// Mounts the mailbox, clicks each id, and returns the rendered tree with every
/// `kom` argv it ran. The tree comes from the live view's snapshot, so it shows
/// what the clicks left behind rather than a fresh initial description.
fn mailbox_after_clicks(
    cx: &mut TestAppContext,
    clicks: &[&'static str],
) -> (String, Vec<Vec<Value>>) {
    mailbox_after_clicks_and_wait(cx, clicks, std::time::Duration::ZERO)
}

fn mailbox_after_clicks_and_wait(
    cx: &mut TestAppContext,
    clicks: &[&'static str],
    wait: std::time::Duration,
) -> (String, Vec<Vec<Value>>) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-mail-undo.json"))
        .with_capabilities(
            Capabilities::new()
                .storage(true)
                .read_roots([staged.0.clone()]),
        )
        .with_host_module(module(
            fixture(true, opened_owner_note(), updates("current", "current")),
            calls.clone(),
        ))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.0.join("frame/mailbox");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    for id in clicks {
        let at = visual
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} is laid out"))
            .center();
        visual.simulate_mouse_move(at, None, Modifiers::default());
        visual.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    if wait > std::time::Duration::ZERO {
        visual.executor().advance_clock(wait);
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    let content = window
        .update(cx, |mounted, _, cx| mounted.0.read(cx).content().clone())
        .unwrap();
    let script = content
        .downcast::<gpui_shell::ScriptView>()
        .unwrap_or_else(|_| panic!("the mailbox is not a script view"));
    let tree = visual.update(|_, cx| {
        script
            .read(cx)
            .snapshot()
            .expect("the mailbox has a rendered tree")
            .debug_tree()
    });
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    let commands = calls
        .effects
        .iter()
        .filter(|(name, _)| name == "kom")
        .filter_map(|(_, args)| args.first()?.as_array().cloned())
        .collect();
    (tree, commands)
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

fn wrote_done(commands: &[Vec<Value>]) -> bool {
    commands
        .iter()
        .any(|argv| argv.first() == Some(&json!("vault")) && argv.get(1) == Some(&json!("done")))
}

/// A closed note keeps its way back: the footer offers Undo, and nothing is
/// written to the vault yet, because a done mark cannot be taken back.
#[gpui::test]
fn closing_a_note_offers_undo_in_the_footer(cx: &mut TestAppContext) {
    let (tree, commands) = mailbox_after_clicks(cx, &["mail-close"]);
    assert!(tree.contains("Closed. Undo"), "{tree}");
    assert!(tree.contains("mail-undo"), "{tree}");
    assert!(
        !wrote_done(&commands),
        "the vault was written before Undo had its six seconds: {commands:?}"
    );
}

/// Undo inside the window cancels the close and brings the note back open.
#[gpui::test]
fn undoing_a_close_brings_the_note_back_without_closing_it(cx: &mut TestAppContext) {
    let (tree, commands) = mailbox_after_clicks(cx, &["mail-close", "mail-undo"]);
    assert!(tree.contains("Please review"), "the note is back: {tree}");
    assert!(
        tree.contains("A body from the vault"),
        "the note is open for reading again: {tree}"
    );
    assert!(tree.contains("mail-close"), "its actions are back: {tree}");
    assert!(!tree.contains("mail-undo"), "the offer is gone: {tree}");
    assert!(
        !wrote_done(&commands),
        "Undo must not have closed the note: {commands:?}"
    );
}

/// Close asks nothing first; the undo window is the confirmation.
#[gpui::test]
fn closing_a_note_asks_no_confirmation(cx: &mut TestAppContext) {
    let (tree, _) = mailbox_after_clicks(cx, &["mail-close"]);
    assert!(!tree.to_lowercase().contains("confirm"), "{tree}");
    assert!(!tree.contains("Are you sure?"), "{tree}");
}

/// When the six seconds pass without Undo, the close reaches the vault.
#[gpui::test]
fn a_close_is_written_once_the_six_seconds_elapse(cx: &mut TestAppContext) {
    let (tree, commands) = mailbox_after_clicks_and_wait(
        cx,
        &["mail-close"],
        std::time::Duration::from_millis(6000),
    );
    assert!(wrote_done(&commands), "the close was written: {commands:?}");
    assert!(!tree.contains("mail-undo"), "the offer is gone: {tree}");
}
