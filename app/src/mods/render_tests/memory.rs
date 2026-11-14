//! Render tests for Settings › Memory: the agent's notes, grouped, opened,
//! edited and forgotten through `kom vault`.
#![allow(unused_imports)]

use super::*;

fn memory_values(memories: Option<Value>, fail: Option<&str>) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("memory");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), json!({}));
    if let Some(memories) = memories {
        values.insert(
            "memoryJson".into(),
            json!({"agent": "claude", "memories": memories, "problems": []}),
        );
    }
    if let Some(fail) = fail {
        values.insert("memoryFail".into(), json!(fail));
    }
    values
}

/// The described page and the `kom` calls it made, with the setup run after init.
fn memory_after_setup(
    cx: &mut TestAppContext,
    values: BTreeMap<String, Value>,
    setup: &str,
) -> (String, Vec<Vec<Value>>) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    static NEXT_STORE: AtomicU64 = AtomicU64::new(0);
    let policy = Policy::new()
        .with_storage_path(staged.0.join(format!(
            "store-memory-{}.json",
            NEXT_STORE.fetch_add(1, Ordering::Relaxed)
        )))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("frame/settings", setup);
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| {
            let mut session = DescriptionSession::load(runtime, root, policy, window, cx)?;
            session.describe(window, cx)?;
            Ok::<_, anyhow::Error>(session)
        })
        .unwrap()
        .unwrap();
    cx.run_until_parked();
    let settled = window
        .update(cx, |_, window, cx| session.describe(window, cx))
        .unwrap()
        .unwrap();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    (
        settled,
        calls
            .effects
            .iter()
            .filter(|(name, args)| {
                name == "kom"
                    && args.first().and_then(Value::as_array).is_some_and(|argv| {
                        argv.first().and_then(Value::as_str) == Some("vault")
                            && matches!(
                                argv.get(1).and_then(Value::as_str),
                                Some("remember") | Some("forget")
                            )
                    })
            })
            .map(|(_, args)| args[0].as_array().cloned().unwrap_or_default())
            .collect(),
    )
}

fn line_with<'a>(tree: &'a str, id: &str) -> &'a str {
    tree.lines()
        .find(|line| line.contains(id))
        .unwrap_or_else(|| panic!("no {id}: {tree}"))
}

#[gpui::test]
fn memories_are_grouped_general_first_then_projects_and_forgotten_folds(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        memory_values(None, None),
        None,
    )
    .unwrap();
    for shown in [
        "Memory",
        "What Claude remembers between sessions. Standing ones are read in full at every start.",
        "General",
        "beta",
        "Forgotten \u{b7} 1",
        "Always answer in English",
        "Beta newest",
        "Old general",
        "Standing \u{b7} Review after 2026-12-01 \u{b7} Updated 2026-11-03T09:00+00:00",
        "Updated 2026-11-05T09:00+00:00",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    let general = tree.find("General").unwrap();
    let beta = tree.find("beta").unwrap();
    let rule = tree.find("Always answer in English").unwrap();
    let old = tree.find("Old general").unwrap();
    let newest = tree.find("Beta newest").unwrap();
    let forgotten = tree.find("Forgotten \u{b7} 1").unwrap();
    assert!(general < rule && rule < old, "General keeps the newest first: {tree}");
    assert!(old < beta, "General comes before the projects: {tree}");
    assert!(beta < newest, "the project's heading comes before its rows: {tree}");
    assert!(newest < forgotten, "Forgotten comes last: {tree}");
    assert!(
        !tree.contains("Gone memory"),
        "Forgotten is folded by default: {tree}"
    );

    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        memory_values(None, None),
        Some("this.memoryForgottenOpen = true;"),
    )
    .unwrap();
    assert!(tree.contains("Gone memory"), "{tree}");
}

#[gpui::test]
fn a_row_opens_to_its_markdown_and_its_buttons(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        memory_values(None, None),
        Some("this.memoryOpenPath = 'claude/memory/beta.md';"),
    )
    .unwrap();
    for shown in [
        "Beta fact.",
        "st-memory-edit-1",
        "st-memory-forget-1",
        "Edit",
        "Forget",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}

#[gpui::test]
fn save_writes_the_memory_with_the_project_and_standing_it_had(cx: &mut TestAppContext) {
    let values = memory_values(None, None);
    let (_, calls) = memory_after_setup(
        cx,
        values,
        "this.memoryOpenPath = 'claude/memory/beta.md'; \
         this.memoryEdit = {path:'claude/memory/beta.md', title:' Beta newest ', body:'Beta fact.', project:'beta', standing:false}; \
         this.memorySave(cx);",
    );
    assert_eq!(
        calls,
        vec![vec![
            "vault",
            "remember",
            "--replace",
            "claude/memory/beta.md",
            "--title",
            "Beta newest",
            "--body",
            "Beta fact.",
            "--project",
            "beta",
        ]]
    );

    let values = memory_values(None, None);
    let (_, calls) = memory_after_setup(
        cx,
        values,
        "this.memoryOpenPath = 'claude/memory/rule.md'; \
         this.memoryEdit = {path:'claude/memory/rule.md', title:'Always answer in English', body:'Use English.', project:null, standing:true}; \
         this.memorySave(cx);",
    );
    assert_eq!(
        calls,
        vec![vec![
            "vault",
            "remember",
            "--replace",
            "claude/memory/rule.md",
            "--title",
            "Always answer in English",
            "--body",
            "Use English.",
            "--standing",
        ]]
    );
}

#[gpui::test]
fn save_is_disabled_for_an_empty_or_long_title_or_a_long_body(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (title, body, enabled) in [
        ("\"\"", "\"A body.\"", false),
        ("\"   \"", "\"A body.\"", false),
        ("\"x\".repeat(121)", "\"A body.\"", false),
        ("\"x\".repeat(120)", "\"A body.\"", true),
        ("\"A title\"", "\"x\".repeat(4001)", false),
        ("\"A title\"", "\"x\".repeat(4000)", true),
    ] {
        let setup = format!(
            "this.memoryOpenPath = 'claude/memory/beta.md'; \
             this.memoryEdit = {{path:'claude/memory/beta.md', title:{title}, body:{body}, project:null, standing:false}};"
        );
        let tree = describe_settled(
            cx,
            &staged,
            "frame/settings",
            memory_values(None, None),
            Some(&setup),
        )
        .unwrap();
        let save = line_with(&tree, "st-memory-save");
        assert_eq!(
            save.contains("on_click"),
            enabled,
            "{title} / {body}: {save}"
        );
    }
}

#[gpui::test]
fn forget_writes_the_reason_and_is_disabled_until_there_is_one(cx: &mut TestAppContext) {
    let values = memory_values(None, None);
    let (_, calls) = memory_after_setup(
        cx,
        values,
        "this.memoryOpenPath = 'claude/memory/beta.md'; \
         this.memoryForgetPath = 'claude/memory/beta.md'; \
         this.memoryWhyText = 'The vault moved.'; \
         this.memoryForget(cx);",
    );
    assert_eq!(
        calls,
        vec![vec![
            "vault",
            "forget",
            "--why",
            "The vault moved.",
            "claude/memory/beta.md",
        ]]
    );

    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (why, enabled) in [("", false), ("   ", false), ("It moved.", true)] {
        let setup = format!(
            "this.memoryOpenPath = 'claude/memory/beta.md'; \
             this.memoryForgetPath = 'claude/memory/beta.md'; this.memoryWhyText = {why:?};"
        );
        let tree = describe_settled(
            cx,
            &staged,
            "frame/settings",
            memory_values(None, None),
            Some(&setup),
        )
        .unwrap();
        let button = line_with(&tree, "st-memory-forget-now");
        assert_eq!(button.contains("on_click"), enabled, "{why:?}: {button}");
    }
}

#[gpui::test]
fn an_empty_memory_says_so(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        memory_values(Some(json!([])), None),
        None,
    )
    .unwrap();
    assert!(
        tree.contains("Claude hasn't remembered anything yet."),
        "{tree}"
    );
}

#[gpui::test]
fn a_failed_read_shows_its_error_and_retry(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        memory_values(None, Some("No Git vault at /fixture/vault.")),
        None,
    )
    .unwrap();
    for shown in [
        "Couldn't read your memory",
        "st-memory-retry",
        "Retry",
        "No Git vault at /fixture/vault.",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}

#[gpui::test]
fn a_failed_save_stays_under_the_open_row(cx: &mut TestAppContext) {
    let mut values = memory_values(None, None);
    values.insert("memoryWriteFail".into(), json!("The vault could not be pulled."));
    let (tree, calls) = memory_after_setup(
        cx,
        values,
        "this.memoryOpenPath = 'claude/memory/beta.md'; \
         this.memoryEdit = {path:'claude/memory/beta.md', title:'Beta newest', body:'Beta fact.', project:'beta', standing:false}; \
         this.memorySave(cx);",
    );
    assert_eq!(calls.len(), 1);
    assert!(tree.contains("Beta fact."), "the row stays open: {tree}");
    assert!(
        tree.contains("The vault could not be pulled."),
        "the error is under the row: {tree}"
    );
}
