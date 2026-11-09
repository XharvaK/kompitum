//! Shipped JS descriptions and native rendering regressions.
use super::*;
use gpui::{Entity, IntoElement, Render, TestAppContext, VisualTestContext};
use gpui_shell::{ShellRoot, ShellRuntime, plugin::DescriptionSession, policy::Policy};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

struct Staged(PathBuf);
impl Staged {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(format!(
            "/var/tmp/kompitum-render-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let staged = Self(path);
        for (folder, files) in [("mods", DEFAULT_MODS), ("frame", FRAME_MODS)] {
            for (name, bytes) in files {
                let path = staged.0.join(folder).join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
        }
        staged
    }
    fn entries(&self) -> Vec<String> {
        let mut entries = Vec::new();
        for folder in ["mods", "frame"] {
            for entry in std::fs::read_dir(self.0.join(folder)).unwrap() {
                let entry = entry.unwrap();
                assert!(
                    entry.path().join("main.js").is_file(),
                    "missing main.js: {:?}",
                    entry.path()
                );
                entries.push(format!("{folder}/{}", entry.file_name().to_str().unwrap()));
            }
        }
        entries.sort();
        entries
    }
    fn wrapper(&self, entry: &str, setup: &str) -> PathBuf {
        let dir = self.0.clone();
        std::fs::write(dir.join("main.js"), format!(
            "import Real from './{entry}/main.js'; export default class Scenario extends Real {{ init(props, cx) {{ super.init(props, cx); {setup} }} }}"
        )).unwrap();
        dir
    }
}
impl Drop for Staged {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Default)]
struct Calls {
    unexpected: Vec<String>,
    effects: Vec<(String, Vec<Value>)>,
    shared: BTreeMap<String, Value>,
}

fn updates(app: &str, core: &str) -> Value {
    json!({
        "app": {"version":"0.1.0", "updates":app != "source", "ready": (app == "ready").then_some("0.1.1")},
        "core": {"version":"0.1.5", "ready":(core == "ready").then_some("0.1.6"), "managed":(core == "managed").then_some("homebrew"), "error":(core == "error").then_some("offline"), "updating":core == "updating"},
        "checked":1700000000, "checks":app != "source", "checking":false
    })
}

fn fixture(populated: bool, props: Value, update: Value) -> BTreeMap<String, Value> {
    let program = json!({"key":"pi", "name":"Pi", "icon":null, "tint":true, "tile":"#334455", "color":"#aabbcc", "shell":false, "agent":true});
    let mut values = BTreeMap::from([
        ("props".into(), props),
        ("machines".into(), json!([{ "name": null, "layout": "ready" }])),
        ("spaces".into(), json!([])), ("agents".into(), json!([])), ("terminals".into(), json!([])), ("panes".into(), json!([])), ("focused".into(), Value::Null),
        ("lead".into(), Value::Null),
        ("layout".into(), serde_json::from_str(include_str!("../../defaults/layouts/main.json")).unwrap()),
        ("layouts".into(), json!(crate::window::shipped_layouts())),
        ("theme".into(), serde_json::to_value(&*crate::settings::theme()).unwrap()),
        ("settings".into(), serde_json::to_value(crate::settings::Settings::default()).unwrap()),
        ("themes".into(), json!([{"name":"graphite", "colors": &*crate::settings::theme()}])),  ("fonts".into(), json!(["Monospace", "Sans"])),
        ("updates".into(), update),
        ("telemetry".into(), json!({"enabled":true,"setting":true,"sends":true,"off_because":null,"endpoint":"https://invalid.example", "draft":{},"queued":[],"last_sent":null})),
        ("window".into(), json!({"platform":"linux","titlebar":"none","buttons":true,"maximized":false,"fullscreen":false,"active":true})),
        ("placement".into(), Value::Null),
        ("appLink".into(), Value::Null), ("person".into(), Value::Null),
        ("home".into(), json!("/fixture")), ("appHome".into(), json!("/fixture/app")),
        ("keys".into(), Value::Array(serde_json::from_str::<BTreeMap<String, BTreeMap<String, String>>>(include_str!("../../defaults/keys.json")).unwrap().into_iter().flat_map(|(section, bindings)| bindings.into_iter().map(move |(keys, action)| json!({"section":section,"keys":keys,"action":action}))).collect())),
        ("commands".into(), Value::Array(crate::commands::COMMANDS.iter().map(|c| json!({"name":c.name,"about":c.about,"args":c.args.iter().map(|a| json!({"name":a.name,"required":a.required})).collect::<Vec<_>>()})).collect())),
    ]);
    if populated {
        values.insert("spaces".into(), json!([
            {"id":"s1","index":1,"name":"Local","folder":"/fixture/project","machine":null,"branch":"main","active":true,"status":"working","tabs":[{"id":"tab1","index":1,"name":"Build","active":true,"status":"working","panes":1,"zoomed":false,"settled":false,"settled_at":null,"changed_at":1700000000000i64,"working_since":1700000000000i64}]},
            {"id":"s2","index":2,"name":"Remote","folder":"/remote/project","machine":"remote","branch":null,"active":false,"status":"done","tabs":[{"id":"tab2","index":1,"name":"Done","active":true,"status":"done","panes":1,"zoomed":false,"settled":false,"settled_at":null,"changed_at":null,"working_since":null}]}
        ]));
        values.insert("panes".into(), json!([{"pane":"p1","terminal":"t1","tab":"tab1","title":"Build","cwd":"/fixture/project","branch":"main","status":"working","app":"pi","program":program,"focused":true,"zoomed":false}, {"pane":"p2","terminal":"t2","tab":"tab2","title":"Remote task","cwd":"/remote/project","branch":null,"status":"done","app":"pi","program":program,"focused":false,"zoomed":false}]));
        values.insert("terminals".into(), json!([{"address":"t1","name":"Build","title":"Build","argv":["pi"],"parent":null,"labels":{},"state":"live","status":"working","report":{"app":"pi"},"program":program,"cwd":"/fixture/project"}, {"address":"t2","name":"Remote task","title":"Remote task","argv":["pi"],"parent":null,"labels":{},"state":"live","status":"done","report":{"app":"pi"},"program":program,"cwd":"/remote/project"}]));
        values.insert("agents".into(), json!([{"address":"t1","pane":"p1","space":"s1","space_name":"Local","tab":"tab1","tab_index":1,"tab_name":"Build","status":"working","app":"pi","program":program,"title":"Build","seq":1,"focused":true}, {"address":"t2","pane":"p2","space":"s2","space_name":"Remote","tab":"tab2","tab_index":1,"tab_name":"Done","status":"done","app":"pi","program":program,"title":"Remote task","seq":2,"focused":false}]));
        values.insert(
            "focused".into(),
            json!({"space":"s1","tab":"tab1","pane":"p1","terminal":"t1","agent":"t1","shown":null,"cwd":"/fixture/project"}),
        );
    }
    values
}

fn vault_owner_note() -> Value {
    json!({
        "path": "manager/to/owner/2026-11-08-please-review.md",
        "from": "manager", "to": "owner", "kind": "request", "status": "open",
        "title": "Please review", "date": "2026-11-08T09:00:00+03:00",
        "project": null, "re": [], "read_by": [], "done": null
    })
}

fn vault_agent_note() -> Value {
    json!({
        "path": "worker/to/manager/2026-11-08-progress.md",
        "from": "worker", "to": "manager", "kind": "report", "status": "open",
        "title": "Progress on the build", "date": "2026-11-08T08:30:00+03:00",
        "project": null, "re": [],
        "read_by": [{"agent": "manager", "at": "2026-11-08T08:40:00+03:00"}], "done": null
    })
}

fn vault_report(notes: Vec<Value>, unpushed: u32, problems: Vec<&str>) -> String {
    json!({
        "vault": "/fixture/vault", "agent": "owner", "unpushed": unpushed,
        "problems": problems, "notes": notes
    })
    .to_string()
}

fn module(mut values: BTreeMap<String, Value>, calls: Rc<RefCell<Calls>>) -> HostModule {
    let person = values.get("person").cloned();
    let known_agents = values.remove("knownAgents");
    let workers_json = values.remove("workersJson");
    let status_json = values.remove("statusJson");
    let vault_missing = values.remove("vaultMissing");
    let mut module = HostModule::new("kompitum");
    for (name, value) in values {
        if name == "person" {
            continue;
        }
        module = module.function(name, move |_| Ok(to_host(&value)));
    }
    let shared = calls.clone();
    module = module.function("shared", move |a| {
        Ok(shared
            .borrow()
            .shared
            .get(a.string(0)?)
            .map(to_host)
            .unwrap_or(HostValue::Null))
    });
    let shared = calls.clone();
    module = module.function("setShared", move |a| {
        shared
            .borrow_mut()
            .shared
            .insert(a.string(0)?.into(), from_host(a.value(1)?));
        Ok(HostValue::Null)
    });
    for name in [
        "run",
        "playSound",
        "setAgentOrder",
        "openOverlay",
        "closeOverlay",
        "notify",
        "presentUpdateNotes",
        "openLink",
    ] {
        let calls = calls.clone();
        module = module.function(name, move |a| {
            if name == "run" {
                let command = a.string(0)?;
                let command = crate::commands::find(command)
                    .unwrap_or_else(|| panic!("unknown shipped mod command {command}"));
                let args = a.get(1).map(from_host).unwrap_or(Value::Null);
                crate::commands::validate_args(command, &args).unwrap();
            }
            calls.borrow_mut().effects.push((
                name.into(),
                (0..2).filter_map(|i| a.get(i).map(from_host)).collect(),
            ));
            Ok(if name == "openOverlay" || name == "notify" {
                "fixture-overlay".into()
            } else {
                HostValue::Null
            })
        });
    }
    module = module.async_function("knownAgents", move |_| {
        let list = known_agents.clone();
        Ok(async move { Ok(to_host(&list.unwrap_or(Value::Array(vec![])))) })
    });
    module = module.function("pasteImage", |_| Ok(HostValue::Null));
    module = module.async_function("pickFolder", |_| Ok(async { Ok(HostValue::Null) }));
    module = module.async_function("leadSetup", |_| {
        Ok(async { Ok(to_host(&json!({"mcp": "present", "skill": "present"}))) })
    });
    for name in ["nextCommand", "watchFolder"] {
        module = module.async_function(name, |_| {
            Ok(std::future::pending::<gpui_shell::HostResult>())
        });
    }
    for name in ["exec", "kom", "readTextFile", "writeTextFile"] {
        let calls = calls.clone();
        let person = person.clone();
        let workers_json = workers_json.clone();
        let status_json = status_json.clone();
        let vault_missing = vault_missing.clone();
        module = module.async_function(name, move |a| {
            let first = a.get(0).map(from_host).unwrap_or(Value::Null);
            let reply = match name {
                "readTextFile" if first == "/fixture/app/settings.json" => Some(
                    person
                        .clone()
                        .map_or(Value::Null, |person| Value::String(person.to_string())),
                ),
                "readTextFile" if first == "/fixture/.ssh/config" => Some(Value::Null),
                "readTextFile" if first.as_str().is_some_and(|p| p.starts_with("/fixture/vault/")) => {
                    Some(json!("# Fixture note\n\nA body from the vault.\n"))
                }
                "readTextFile" => first
                    .as_str()
                    .and_then(|p| p.strip_prefix("/fixture/app/layouts/"))
                    .and_then(|p| p.strip_suffix(".json"))
                    .filter(|n| crate::window::shipped_layouts().contains(n))
                    .map(|_| json!("{}")),
                "kom" if first == json!(["machine", "ls", "--json"]) => {
                    Some(json!({"code":0,"stdout":"{\"machines\":[{\"name\":\"remote\",\"state\":\"online\"}]}","stderr":""}))
                }
                // The welcome: a signed-in check asks the core for its exit code only.
                "kom" if first.as_array().and_then(|a| a.first()).and_then(Value::as_str) == Some("exec") => {
                    Some(json!({"code":0,"stdout":"","stderr":""}))
                }
                // The mailbox: one note that needs the Owner, one between agents.
                "kom" if first == json!(["vault", "open", "--json", "--as", "owner"]) && vault_missing.is_some() => Some(json!({
                    "code": 1,
                    "stdout": json!({"error": {"message": "No Git vault at /fixture/vault."}}).to_string(),
                    "stderr": ""
                })),
                "kom" if first == json!(["vault", "open", "--json", "--as", "owner"]) => Some(json!({
                    "code": 0,
                    "stdout": vault_report(vec![vault_owner_note()], 2, vec!["fixture: a note's mark file is damaged"]),
                    "stderr": ""
                })),
                "kom" if first == json!(["vault", "find", "--json", "--limit", "50"]) => Some(json!({
                    "code": 0,
                    "stdout": vault_report(vec![vault_owner_note(), vault_agent_note()], 2, vec![]),
                    "stderr": ""
                })),
                "kom" if first == json!(["vault", "changes", "--json", "--as", "owner"]) => Some(json!({
                    "code": 0,
                    "stdout": vault_report(vec![], 0, vec![]),
                    "stderr": ""
                })),
                "kom" if first == json!(["vault", "read", "manager/to/owner/2026-11-08-please-review.md", "--json", "--as", "owner"]) => Some(json!({
                    "code": 0,
                    "stdout": json!({
                        "vault": "/fixture/vault", "agent": "owner", "unpushed": 0,
                        "problems": [], "note": vault_owner_note()
                    }).to_string(),
                    "stderr": ""
                })),
                "kom" if first == json!(["machine", "add", "host", "--json"]) => {
                    Some(json!({"code":1,"stdout":"{\"error\":{\"kind\":\"host_key\",\"message\":\"unknown host key\"}}","stderr":""}))
                }
                // The Board: one task waiting for review.
                // The Agents page: the core's workers and Claude's own limits.
                "kom" if first == json!(["workers", "--json"]) => {
                    let body = workers_json.clone().unwrap_or_else(|| json!({
                        "mode": "auto", "order": ["cc", "go"], "chain": ["cc", "go"], "next": "cc",
                        "all_out": null, "warnings": [], "clis": [], "asking": [],
                        "workers": [
                            {"key":"cc","label":"Command Code","cli":"command_code","route":"commandcode/deepseek/deepseek-v4.1-flash#max","state":"ready","until_ms":0,"source":null,"window":"","message":"","sentence":"","refusals":0,"spend":{"five_hours":{"runs":0,"cost_usd":0.0},"week":{"runs":0,"cost_usd":0.0},"month":{"runs":0,"cost_usd":0.0}}},
                            {"key":"go","label":"opencode Go","cli":"opencode","route":"opencode-go/deepseek-v4.1-flash#max","state":"ready","until_ms":0,"source":null,"window":"","message":"","sentence":"","refusals":0,"spend":{"five_hours":{"runs":0,"cost_usd":0.0},"week":{"runs":0,"cost_usd":0.0},"month":{"runs":0,"cost_usd":0.0}}}
                        ]
                    }));
                    Some(json!({"code":0,"stdout":body.to_string(),"stderr":""}))
                }
                "kom" if first == json!(["status", "--json"]) => Some(json!({"code":0,"stdout":status_json.clone().unwrap_or_else(|| json!({
                    "tasks": {"review": [{"name": "fix-docs", "repo": "kompitum-core", "worker": "opencode", "by": "manager", "seconds": 540, "cost_usd": 0.06}], "running": [], "queued": []},
                    "claude": null
                })).to_string(),"stderr":""})),
                // The composer's send: one recorded effect, so key tests can read it.
                "kom" if first.as_array().and_then(|a| a.first()).and_then(Value::as_str) == Some("send") => {
                    calls.borrow_mut().effects.push(("kom".into(), vec![first.clone()]));
                    Some(json!({"code":0,"stdout":"","stderr":""}))
                }
                // The Line: one note from Manager.
                "kom" if first == json!(["vault", "thread", "manager", "--json"]) => Some(json!({"code":0,"stdout":json!({
                    "agent": "opus",
                    "notes": [{"path": "manager/to/opus/2026-11-08-tests-pass.md", "from": "manager", "to": ["opus"], "title": "Tests pass", "date": "2026-11-08T09:00:00+03:00", "status": "open"}]
                }).to_string(),"stderr":""})),
                // Settings › Connect: the vault's folder and notes.
                "kom" if first == json!(["vault", "open", "--json", "--as", "opus"]) => Some(json!({"code":0,"stdout":json!({
                    "vault": "/fixture/vault", "agent": "opus", "unpushed": 0, "problems": [], "notes": []
                }).to_string(),"stderr":""})),
                "kom" if first == json!(["vault", "find", "Joined", "--kind", "report", "--json"]) => Some(json!({"code":0,"stdout":json!({
                    "vault": "/fixture/vault", "agent": "opus", "unpushed": 0, "problems": [], "notes": []
                }).to_string(),"stderr":""})),
                // The files panel lists a folder with sh: kind and name pairs.
                "exec" if first == "sh" && a.get(1).map(from_host).and_then(|v| v.get(2).cloned()) == Some(json!("files")) => {
                    Some(json!({"code":0,"stdout":"d\u{0}src\u{0}f\u{0}README.md\u{0}","stderr":""}))
                }
                "exec"
                    if first == "git"
                        && a.get(1).map(from_host) == Some(json!(["-C", "/fixture/vault", "remote", "get-url", "origin"])) =>
                {
                    Some(json!({"code":0,"stdout":"https://example.invalid/opus/vault.git\n","stderr":""}))
                }
                "exec"
                    if first == "git"
                        && [
                            json!(["rev-parse", "--show-toplevel"]),
                            json!(["rev-parse", "--path-format=absolute", "--git-common-dir"]),
                            json!(["status", "--porcelain=v2", "--branch"]),
                            json!(["--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain=v2", "--branch"]),
                            json!(["--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain"]),
                            json!(["rev-parse", "--abbrev-ref", "HEAD"]),
                        ]
                        .contains(&a.get(1).map(from_host).unwrap_or(Value::Null)) =>
                {
                    Some(json!({"code":128,"stdout":"","stderr":"fixture: not a git repository"}))
                }
                _ => None,
            };
            let result = if let Some(reply) = reply {
                Ok(to_host(&reply))
            } else {
                let message = format!("unexpected {name}: {first}, {:?}", a.get(1));
                calls.borrow_mut().unexpected.push(message.clone());
                Err(HostError::new(message))
            };
            Ok(std::future::ready(result))
        });
    }
    module.declarations(DECLARATIONS)
}

#[gpui::test]
fn mod_copies_share_code_without_sharing_props_state_or_lifetime(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("cache");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("main.js"),
        r#"
import { View, div } from 'gpui-kit';
import { props, mark, next } from 'copy';
const evaluation = mark();
const row = globalThis.__template(value => div().child(value));
export default class Panel extends View {
  init(_, cx) {
    if (props() === 'bad') throw Error('bad props');
    this.count = 0;
    this.count++;
    this.async = 'waiting';
    cx.spawn(async cx => { await next(); this.async = props(); cx.notify(); });
  }
  render() { return row(props() + ':' + this.count + ':' + evaluation + ':' + this.async); }
}
"#,
    )
    .unwrap();
    let marks = Rc::new(Cell::new(0));
    let policy = |name: &'static str| {
        let marks = marks.clone();
        Policy::new()
            .with_host_module(
                HostModule::new("copy")
                    .function("props", move |_| Ok(name.into()))
                    .function("mark", move |_| {
                        marks.set(marks.get() + 1);
                        Ok((marks.get() as f64).into())
                    })
                    .async_function("next", |_| Ok(std::future::ready(Ok(HostValue::Null)))),
            )
            .unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    let (first, mut second) = window
        .update(cx, |_, window, cx| {
            let mut first =
                DescriptionSession::load(runtime.clone(), &root, policy("A"), window, cx).unwrap();
            let mut second =
                DescriptionSession::load(runtime.clone(), &root, policy("B"), window, cx).unwrap();
            assert!(first.describe(window, cx).unwrap().contains("A:1:1"));
            assert!(second.describe(window, cx).unwrap().contains("B:1:1"));
            assert!(
                DescriptionSession::load(runtime.clone(), &root, policy("bad"), window, cx)
                    .is_err()
            );
            (first, second)
        })
        .unwrap();
    cx.run_until_parked();
    drop(first);
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            let tree = second.describe(window, cx).unwrap();
            assert!(tree.contains("B:1:1:B"), "{tree}");
        })
        .unwrap();
    drop(second);
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            let mut reopened =
                DescriptionSession::load(runtime.clone(), &root, policy("C"), window, cx).unwrap();
            assert!(reopened.describe(window, cx).unwrap().contains("C:1:1"));
        })
        .unwrap();
    assert_eq!(
        marks.get(),
        1,
        "reopening and failed init must reuse evaluated code"
    );
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn mod_code_revisions_share_successes_and_failures_without_poisoning_old_copies(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("cache");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("main.js"),
        r#"
import { View, div } from 'gpui-kit';
import { version } from './part.js';
export default class Panel extends View { render() { return div().child('version:' + version); } }
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("part.js"),
        "import { mark } from 'copy'; mark(); export const version = 1;",
    )
    .unwrap();
    let marks = Rc::new(Cell::new(0));
    let policy = || {
        let marks = marks.clone();
        Policy::new()
            .with_host_module(HostModule::new("copy").function("mark", move |_| {
                marks.set(marks.get() + 1);
                Ok(HostValue::Null)
            }))
            .unwrap()
    };
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window.update(cx, |_, window, cx| {
        let mut old = DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).unwrap();
        for version in [2, 3] {
            std::fs::write(root.join("part.js"), format!("import {{ mark }} from 'copy'; mark(); export const version = {version};{}", " ".repeat(version))).unwrap();
            for _ in 0..20 {
                let mut new = DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).unwrap();
                assert!(new.describe(window, cx).unwrap().contains(&format!("version:{version}")));
            }
        }
        assert_eq!(marks.get(), 3);
        std::fs::write(root.join("part.js"), "import { mark } from 'copy'; mark(); throw Error('broken save'); export const version = 4;").unwrap();
        for _ in 0..20 {
            assert!(DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).is_err());
        }
        assert_eq!(marks.get(), 4, "a failed revision evaluates once, not on every open");
        assert!(old.describe(window, cx).unwrap().contains("version:1"));
        std::fs::write(root.join("part.js"), "import { mark } from 'copy'; mark(); export const version = 5;").unwrap();
        let mut fixed = DescriptionSession::load(runtime.clone(), &root, policy(), window, cx).unwrap();
        assert!(fixed.describe(window, cx).unwrap().contains("version:5"));
        assert_eq!(marks.get(), 5);
        assert!(DescriptionSession::load(runtime.clone(), &root, Policy::new(), window, cx).is_err(),
            "an ungranted host module must not reuse another policy's linked class");
    }).unwrap();
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn mod_host_imports_keep_sync_async_and_missing_export_surfaces_distinct(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let root = staged.0.join("cache");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("main.js"),
        r#"
import { View, div } from 'gpui-kit';
import { value } from 'copy';
export default class Panel extends View {
  init() { this.kind = typeof value(); }
  render() { return div().child('kind:' + this.kind); }
}
"#,
    )
    .unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|_, _| Empty);
    window
        .update(cx, |_, window, cx| {
            let sync = HostModule::new("copy").function("value", |_| Ok(HostValue::Number(1.0)));
            let mut a = DescriptionSession::load(
                runtime.clone(),
                &root,
                Policy::new().with_host_module(sync).unwrap(),
                window,
                cx,
            )
            .unwrap();
            assert!(a.describe(window, cx).unwrap().contains("kind:number"));
            let asynchronous = HostModule::new("copy")
                .async_function("value", |_| Ok(std::future::ready(Ok(HostValue::Null))));
            let mut b = DescriptionSession::load(
                runtime.clone(),
                &root,
                Policy::new().with_host_module(asynchronous).unwrap(),
                window,
                cx,
            )
            .unwrap();
            assert!(b.describe(window, cx).unwrap().contains("kind:object"));
            let missing = HostModule::new("copy").function("other", |_| Ok(HostValue::Null));
            assert!(
                DescriptionSession::load(
                    runtime.clone(),
                    &root,
                    Policy::new().with_host_module(missing).unwrap(),
                    window,
                    cx
                )
                .is_err()
            );
            assert!(a.describe(window, cx).unwrap().contains("kind:number"));
        })
        .unwrap();
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
}

struct Empty;
impl gpui::Render for Empty {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
    }
}

fn describe(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    populated: bool,
    props: Value,
    update: Value,
    setup: Option<&str>,
) -> anyhow::Result<String> {
    describe_with_values(cx, staged, entry, fixture(populated, props, update), setup)
}

fn describe_with_values(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    setup: Option<&str>,
) -> anyhow::Result<String> {
    describe_values(cx, staged, entry, values, setup, false)
}

fn describe_values(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    setup: Option<&str>,
    settle: bool,
) -> anyhow::Result<String> {
    let calls = Rc::new(RefCell::new(Calls::default()));
    static NEXT_STORE: AtomicU64 = AtomicU64::new(0);
    let policy = Policy::new()
        .with_storage_path(staged.0.join(format!(
            "store-{}.json",
            NEXT_STORE.fetch_add(1, Ordering::Relaxed)
        )))
        .with_capabilities(
            Capabilities::new()
                .storage(true)
                .read_roots([staged.0.clone()]),
        )
        .with_host_module(module(values, calls.clone()))?;
    let runtime = ShellRuntime::new_isolated()?;
    let root = setup
        .map(|s| staged.wrapper(entry, s))
        .unwrap_or_else(|| staged.0.join(entry));
    let window = cx.add_window(|_, _| Empty);
    let result = if settle {
        let mut session = window.update(cx, |_, window, cx| {
            let mut session = DescriptionSession::load(runtime, root, policy, window, cx)?;
            session.describe(window, cx)?;
            Ok::<_, anyhow::Error>(session)
        })??;
        // Let the async reads that init and the first render started land.
        cx.run_until_parked();
        let settled = window.update(cx, |_, window, cx| session.describe(window, cx))??;
        assert!(!settled.is_empty(), "empty description: {entry}");
        Ok(settled)
    } else {
        window.update(cx, |_, window, cx| {
            let mut session = DescriptionSession::load(runtime, root, policy, window, cx)?;
            let first = session.describe(window, cx)?;
            let second = session.describe(window, cx)?;
            assert!(
                !first.is_empty() && !second.is_empty(),
                "empty description: {entry}"
            );
            Ok(second)
        })?
    };
    window.update(cx, |_, window, _| window.remove_window())?;
    // localStorage writes land on the executor: let them finish while the
    // staged folder is still there, or they recreate it after its drop.
    cx.run_until_parked();
    assert!(
        calls.borrow().unexpected.is_empty(),
        "{entry}: {:?}",
        calls.borrow().unexpected
    );
    result
}

/// Like `describe_with_values`, but lets the mod's first async pass finish so
/// the description shows what its host reads answered.
fn describe_settled(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    setup: Option<&str>,
) -> anyhow::Result<String> {
    describe_values(cx, staged, entry, values, setup, true)
}

fn props() -> Value {
    json!({"overlay":"o1","pane":"p1","terminal":"t1","space":"s1","tab":"pane","target":{"pane":"p1","terminal":"t1","id":"s1","space":"s1"},"title":"Fixture","value":"Name","message":"A prefilled message","command":{"name":"space.rename","args":{"id":"s1"}},"notification":{"title":"Finished","body":"Build done","action":{"name":"app.help"}},"action":"new"})
}

#[gpui::test]
fn shipped_initial_descriptions(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let start = Instant::now();
    let staged = Staged::new();
    let entries = staged.entries();
    let mut count = 0;
    for entry in &entries {
        for populated in [false, true] {
            describe(
                cx,
                &staged,
                entry,
                populated,
                props(),
                updates("current", "current"),
                None,
            )
            .unwrap_or_else(|e| {
                panic!(
                    "{entry}/{}: {e:#}",
                    if populated { "populated" } else { "empty" }
                )
            });
            count += 1;
        }
    }
    eprintln!(
        "{count} initial-description cases ({} entries, each twice): {:?}",
        entries.len(),
        start.elapsed()
    );
}

/// The Lead pill opens its hand-off as an overlay of the same mod, told so by
/// its props. The overlay must draw the hand-off with its close button, not a
/// second pill in an empty window with no way out.
#[gpui::test]
fn the_lead_overlay_draws_the_hand_off_with_a_way_out(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["mode"] = json!("handoff");
    p["title"] = json!("Hand the lead to Manager");
    p["markdown"] = json!("Nothing is open.");
    p["send"] = json!("manager");
    let tree = describe(
        cx,
        &staged,
        "mods/lead",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("Hand the lead to Manager"), "{tree}");
    assert!(tree.contains("lead-close"), "{tree}");
    assert!(!tree.contains("Lead · "), "the pill drew instead: {tree}");
}

/// The composer draws its box under a live lead, and one line with Open when
/// no lead terminal is alive.
#[gpui::test]
fn the_composer_renders_with_and_without_a_lead(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut with = fixture(true, props(), updates("current", "current"));
    with.insert("knownAgents".into(), agent_catalog());
    let tree = describe_settled(cx, &staged, "mods/composer", with, None).unwrap();
    assert!(tree.contains("Claude"), "{tree}");
    assert!(tree.contains("composer-send"), "{tree}");
    assert!(!tree.contains("The lead isn't running"), "{tree}");

    let mut without = fixture(false, props(), updates("current", "current"));
    without.insert("knownAgents".into(), agent_catalog());
    let tree = describe_settled(cx, &staged, "mods/composer", without, None).unwrap();
    assert!(tree.contains("The lead isn't running"), "{tree}");
    assert!(tree.contains("composer-open"), "{tree}");
}

/// A listed terminal that has ended is not the lead: the composer shows Open.
#[gpui::test]
fn the_composer_needs_a_live_lead(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.get_mut("terminals").unwrap()[0]["state"] = json!("exited");
    let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
    assert!(tree.contains("The lead isn't running"), "{tree}");
    assert!(tree.contains("composer-open"), "{tree}");
}

fn composer_with_ask(kind: &str, input: Value) -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert(
        "lead".into(),
        json!({
            "state": "asking",
            "ask": {"id": "a1", "terminal": "t1", "kind": kind, "input": input},
        }),
    );
    values
}

/// Each tool's permission card says what Claude wants, not its raw JSON.
#[gpui::test]
fn the_permission_card_says_what_claude_wants(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (tool, input, expected) in [
        ("Bash", json!({"command": "cargo test --all"}), vec!["Run a command", "cargo test --all"]),
        ("Edit", json!({"file_path": "/src/main.rs"}), vec!["Edit /src/main.rs"]),
        ("Write", json!({"file_path": "/tmp/out.txt"}), vec!["Edit /tmp/out.txt"]),
        ("Read", json!({"file_path": "/tmp/log.txt"}), vec!["Read /tmp/log.txt"]),
        ("WebFetch", json!({"url": "https://example.invalid/page"}), vec!["Fetch https://example.invalid/page"]),
        ("WebSearch", json!({"query": "rust lifetimes"}), vec!["Search the web for rust lifetimes"]),
        ("mcp__github__create_issue", json!({"title": "A bug"}), vec!["create_issue on github"]),
        ("Grep", json!({"pattern": "needle"}), vec!["Grep", "needle"]),
    ] {
        let tree = describe_settled(
            cx,
            &staged,
            "mods/composer",
            composer_with_ask("permission", json!({"tool_name": tool, "tool_input": input})),
            None,
        )
        .unwrap_or_else(|e| panic!("{tool}: {e:#}"));
        for expected in expected {
            assert!(tree.contains(expected), "{tool} misses {expected:?}: {tree}");
        }
        assert!(tree.contains("composer-allow"), "{tool}: {tree}");
        assert!(tree.contains("composer-deny"), "{tool}: {tree}");
        assert!(tree.contains("composer-terminal-answer"), "{tool}: {tree}");
        assert!(!tree.contains("composer-allow-always"), "{tool}: allow always without suggestions: {tree}");
    }
}

/// Allow always appears only with a suggestion, and says where the rule lives.
#[gpui::test]
fn allow_always_says_where_the_rule_is_kept(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (destination, words) in [
        ("session", "for this session"),
        ("localSettings", "in this project"),
        ("projectSettings", "in this project"),
        ("userSettings", "for all your projects"),
    ] {
        let values = composer_with_ask(
            "permission",
            json!({
                "tool_name": "Bash",
                "tool_input": {"command": "ls"},
                "permission_suggestions": [{"destination": destination}],
            }),
        );
        let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
        assert!(tree.contains("composer-allow-always"), "{destination}: {tree}");
        assert!(tree.contains(words), "{destination}: {tree}");
    }
}

/// The plan card names the plan and offers both answers.
#[gpui::test]
fn the_plan_card_offers_approve_and_keep_planning(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        composer_with_ask("plan", json!({"tool_name": "ExitPlanMode", "tool_input": {"plan": "Do it."}})),
        None,
    )
    .unwrap();
    assert!(tree.contains("Claude's plan is ready"), "{tree}");
    assert!(tree.contains("composer-approve"), "{tree}");
    assert!(tree.contains("composer-keep-planning"), "{tree}");
    assert!(tree.contains("composer-terminal-answer"), "{tree}");
    assert!(!tree.contains("composer-allow"), "{tree}");
}

/// A question card shows its header, text, options and descriptions.
#[gpui::test]
fn the_question_card_shows_every_question_and_option(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        composer_with_ask(
            "question",
            json!({
                "tool_name": "AskUserQuestion",
                "tool_input": {"questions": [
                    {
                        "question": "Which database?",
                        "header": "Database",
                        "multiSelect": false,
                        "options": [
                            {"label": "Postgres", "description": "The server's own"},
                            {"label": "SQLite", "description": "A single file"},
                        ],
                    },
                    {
                        "question": "Which checks?",
                        "header": "Checks",
                        "multiSelect": true,
                        "options": [
                            {"label": "Tests", "description": "Run the suite"},
                            {"label": "Lint", "description": "Keep it tidy"},
                        ],
                    },
                ]},
            }),
        ),
        None,
    )
    .unwrap();
    for expected in [
        "Database",
        "Which database?",
        "Postgres",
        "The server's own",
        "SQLite",
        "Checks",
        "Which checks?",
        "Tests",
        "Lint",
        "composer-send-answers",
        "composer-terminal-answer",
    ] {
        assert!(tree.contains(expected), "missing {expected:?}: {tree}");
    }
}

struct Mounted(Entity<ShellRoot>);
impl Render for Mounted {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

/// Dispatches `keys` to a mounted composer and returns what it sent with
/// `kom send`. The real focus path matters: GPUI's Input context keeps Up and
/// Down for the caret, so the composer's root must still hear them.
fn composer_sends_after_keys(cx: &mut TestAppContext, setup: &str, keys: &str) -> Vec<Vec<Value>> {
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert("lead".into(), json!({"state": "working", "ask": null}));
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-keys.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let root = staged.wrapper("mods/composer", setup);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.simulate_keystrokes(keys);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    calls
        .effects
        .iter()
        .filter(|(name, _)| name == "kom")
        .filter_map(|(_, args)| args.first()?.as_array().cloned())
        .collect()
}

#[gpui::test]
fn up_and_down_recall_history_from_the_textarea(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let sends = composer_sends_after_keys(
        cx,
        "this.history = ['newest', 'oldest']; this.historyAt = -1; this.setHistory(true, cx); this.text.set_submit_on_enter(true); this.text.focus();",
        "up up enter",
    );
    let sent = sends.iter().find_map(|args| args.get(2).and_then(Value::as_str)).unwrap_or_default();
    assert!(
        sent.contains("oldest") && !sent.contains("newest"),
        "Up twice must recall the second history entry, then Enter sends it: {sends:?}"
    );
}

#[gpui::test]
fn up_moves_the_caret_in_a_non_empty_draft(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let sends = composer_sends_after_keys(
        cx,
        "this.history = ['newest', 'oldest']; this.historyAt = -1; this.text.set_value('typed'); this.text.set_submit_on_enter(true); this.setHistory(false, cx); this.text.focus();",
        "up enter",
    );
    let sent = sends.iter().find_map(|args| args.get(2).and_then(Value::as_str)).unwrap_or_default();
    assert!(
        sent.contains("typed") && !sent.contains("newest"),
        "Up in a draft must move the caret, not recall history: {sends:?}"
    );
}

/// With the card showing, Enter answers it: the Owner's key becomes the hook's
/// reply through `lead.answer`, not a message typed into the terminal.
#[gpui::test]
fn enter_answers_a_permission_card(cx: &mut TestAppContext) {
    let answered = composer_run_effects_after_keys(
        cx,
        composer_with_ask("permission", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})),
        "enter",
    );
    assert!(
        answered.iter().any(|args| {
            args.first() == Some(&json!("lead.answer"))
                && args.get(1) == Some(&json!({"id": "a1", "answer": {"action": "allow"}}))
        }),
        "Enter must answer the card through lead.answer: {answered:?}"
    );
}

/// Escape answers a permission card with a denial, never by typing Esc into
/// the terminal: the composer's own Escape action must see the card first.
#[gpui::test]
fn escape_denies_a_permission_card(cx: &mut TestAppContext) {
    let answered = composer_run_effects_after_keys(
        cx,
        composer_with_ask("permission", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})),
        "escape",
    );
    assert!(
        answered.iter().any(|args| {
            args.first() == Some(&json!("lead.answer"))
                && args.get(1) == Some(&json!({"id": "a1", "answer": {"action": "deny"}}))
        }),
        "Escape must deny the card through lead.answer: {answered:?}"
    );
}

fn composer_run_effects_after_keys(
    cx: &mut TestAppContext,
    values: BTreeMap<String, Value>,
    keys: &str,
) -> Vec<Vec<Value>> {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-card.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window =
        cx.add_window(|window, cx| Mounted(runtime.try_load(staged.0.join("mods/composer"), window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.simulate_keystrokes(keys);
    visual.run_until_parked();
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    calls.effects.iter().filter(|(name, _)| name == "run").map(|(_, args)| args.clone()).collect()
}

/// A mod's localStorage write lands on the executor; one that lands after the
/// staged folder's drop recreates it, and the folder outlives the test.
#[gpui::test]
fn a_described_mod_leaves_no_folder_behind(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let path = staged.0.clone();
    describe(
        cx,
        &staged,
        "mods/agents",
        true,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    drop(staged);
    cx.run_until_parked();
    assert!(
        !path.exists(),
        "left behind: {:?}",
        std::fs::read_dir(&path).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
    );
}

#[gpui::test]
fn brand_leaves_room_for_macos_traffic_lights_only_outside_full_screen(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (fullscreen, inset) in [(false, true), (true, false)] {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.insert(
            "window".into(),
            json!({"platform":"macos","titlebar":"none","fullscreen":fullscreen,"buttons":false}),
        );
        let tree =
            describe_with_values(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
        let root = tree.lines().next().unwrap();
        assert_eq!(
            root.contains(".pl[Number(76.0)]"),
            inset,
            "fullscreen={fullscreen}: {tree}"
        );
    }
}

#[gpui::test]
fn tabs_leave_room_for_macos_traffic_lights(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (platform, titlebar, fullscreen, width, lights, zoom, expected) in [
        ("macos", "none", false, 44, true, 1.0, 32),
        ("macos", "none", false, 44, true, 0.5, 108),
        ("macos", "none", false, 44, true, 2.0, 10),
        ("macos", "none", false, 60, true, 1.0, 16),
        ("macos", "none", false, 76, true, 1.0, 10),
        ("macos", "none", false, 268, true, 1.0, 10),
        ("macos", "none", true, 44, true, 1.0, 10),
        ("macos", "system", false, 44, true, 1.0, 10),
        ("macos", "none", false, 44, false, 1.0, 10),
        ("linux", "none", false, 44, true, 1.0, 10),
        ("windows", "none", false, 44, true, 1.0, 10),
    ] {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.insert("window".into(), json!({"platform":platform,"titlebar":titlebar,"fullscreen":fullscreen,"buttons":false}));
        values.get_mut("settings").unwrap()["zoom"] = json!(zoom);
        let layout = values.get_mut("layout").unwrap();
        layout["children"][0]["size"]["px"] = json!(width);
        if !lights {
            layout.as_object_mut().unwrap().remove("traffic_lights");
        }
        let tree = describe_with_values(cx, &staged, "mods/tabs", values, None).unwrap();
        let root = tree.lines().next().unwrap();
        assert!(
            root.contains(&format!(".pl[Number({expected}.0)]")),
            "{platform}/{titlebar}/{fullscreen}/{width}/{lights}: {tree}"
        );
        assert!(
            tree.contains("tab-tab1"),
            "first tab must remain visible: {tree}"
        );
    }
}

#[gpui::test]
fn windows_tab_buttons_use_matching_vector_icons(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for platform in ["windows", "linux"] {
        for maximized in [false, true] {
            let mut values = fixture(true, props(), updates("current", "current"));
            values.insert(
                "window".into(),
                json!({"platform":platform,"buttons":true,"maximized":maximized}),
            );
            let tree = describe_with_values(cx, &staged, "mods/tabs", values, None).unwrap();
            if platform == "windows" {
                for icon in [
                    "win-min.svg",
                    if maximized {
                        "win-restore.svg"
                    } else {
                        "win-max.svg"
                    },
                    "win-close.svg",
                ] {
                    assert!(tree.contains(icon), "missing {icon}: {tree}");
                }
            } else {
                assert!(!tree.contains("mods/tabs/win-"), "{tree}");
                assert!(tree.contains("mods/pane/close.svg"), "{tree}");
            }
        }
    }
}

#[gpui::test]
fn font_pickers_contain_scroll_on_the_wrapper(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for which in ["font", "ui_font"] {
        let mut p = props();
        p["section"] = json!("appearance");
        let tree = describe(
            cx,
            &staged,
            "frame/settings",
            true,
            p,
            updates("current", "current"),
            Some(&format!("this.picking = '{which}';")),
        )
        .unwrap();
        let scroll = tree
            .lines()
            .find(|line| line.contains(&format!("st-fl-{which}")))
            .unwrap();
        assert!(scroll.contains("overflow_y_scrollbar"), "{tree}");
        assert!(
            !scroll.contains("on_scroll_wheel"),
            "the native scroller must receive the event: {tree}"
        );
        assert_eq!(
            tree.matches("on_scroll_wheel").count(),
            1,
            "the wrapper must stop scroll chaining: {tree}"
        );
    }
}

#[gpui::test]
fn confirm_dialog_background_follows_the_frame_corners(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["title"] = json!("Close space?");
    p["command"] = json!({"name": "space.close", "args": {"id": "s1"}});
    let tree = describe(
        cx,
        &staged,
        "mods/confirm",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    let root = tree
        .lines()
        .find(|l| l.contains(":id[Str(\"confirm\")]"))
        .unwrap();
    assert!(
        root.contains(".rounded[Number(8.0)]"),
        "the dialog background must follow the frame's corners: {root}"
    );
}

#[gpui::test]
fn check_materializes_retained_elements_in_a_rendered_view(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for body in [
        "v_flex().h(400).child(v_virtual_list('rows', 0, 28, (i) => 'a', () => { throw new Error('deferred rows ran'); }))",
        "v_flex().h(400).child(v_virtual_list('rows', 1, 28, (i) => 'a', () => { throw new Error('deferred rows ran'); }))",
        "div().w(100).h(50).left(10).transition('left', 200)",
        "div().w(100).h(50).transition('width', 200)",
        "div().w(100).h(50).left(10).spring('left')",
        "div().opacity(0.5).transition('opacity', 200)",
    ] {
        std::fs::write(staged.0.join("main.js"), format!(
            "import {{ View, div }} from 'gpui-kit'; import {{ v_flex, v_virtual_list }} from 'gpui-base'; export default class Demo extends View {{ render() {{ return {body}; }} }}"
        )).unwrap();
        let runtime = ShellRuntime::new_isolated().unwrap();
        let window = cx.add_window(|_, _| Empty);
        let description = window
            .update(cx, |_, window, cx| runtime.check(&staged.0, window, cx))
            .unwrap()
            .unwrap();
        assert!(!description.is_empty(), "{body}");
        gpui::AnyWindowHandle::from(window)
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[gpui::test]
fn rebuilt_machine_offers_connect_only_for_identity_mismatch(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (state, why, offered) in [
        (
            "unreachable",
            "a different core answers there (\"new-id\")",
            true,
        ),
        ("unreachable", "connection refused", false),
        ("unreachable", "a different core answers there", false),
        (
            "connected",
            "a different core answers there (\"new-id\")",
            false,
        ),
    ] {
        let setup = format!(
            "this.machines = [{{name:'cloud', state:{}, why:{}, target:'host'}}]; this.folded.add('cloud');",
            json!(state),
            json!(why)
        );
        let tree = describe(
            cx,
            &staged,
            "mods/spaces",
            false,
            props(),
            updates("current", "current"),
            Some(&setup),
        )
        .unwrap();
        assert_eq!(
            tree.contains("machine-connect-cloud"),
            offered,
            "{state}/{why}: {tree}"
        );
        assert_eq!(tree.contains("rebuilt ·"), offered, "{tree}");
        assert_eq!(
            tree.contains("This machine was rebuilt or replaced. Click to connect to the new one."),
            offered,
            "{tree}"
        );
        assert!(
            !tree.contains("machine-reconnect-"),
            "reconnect belongs in the machine row: {tree}"
        );
    }
}

#[gpui::test]
fn machine_picker_keeps_long_lists_scrollable(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "mods/machine-add",
        false,
        props(),
        updates("current", "current"),
        Some(
            "this.hosts = Array.from({length: 30}, (_, i) => ({alias: `host-${i}`, aliases: []}));",
        ),
    )
    .unwrap();
    let scroll = tree.lines().find(|line| line.contains("ma-hosts")).unwrap();
    assert!(
        scroll.contains("overflow_y_scroll") && !scroll.contains("overflow_y_scrollbar"),
        "{tree}"
    );
    assert!(scroll.contains(".min_h[Number(0.0)]"), "{tree}");
    assert!(tree.contains("ma-row-host-29"), "{tree}");
    assert_eq!(
        tree.matches(".flex_none").count(),
        60,
        "rows must not shrink into the viewport: {tree}"
    );
}

#[gpui::test]
fn machine_add_needing_input_keeps_the_picker_and_shows_instructions(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_host_module(module(
            fixture(false, props(), updates("current", "current")),
            calls.clone(),
        ))
        .unwrap();
    let root = staged.wrapper(
        "mods/machine-add",
        "this.typed = 'host'; this.add('host', cx);",
    );
    let window = cx.add_window(|_, _| Empty);
    let mut session = window
        .update(cx, |_, window, cx| {
            DescriptionSession::load(
                ShellRuntime::new_isolated().unwrap(),
                root,
                policy,
                window,
                cx,
            )
            .unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            let tree = session.describe(window, cx).unwrap();
            assert!(tree.contains("machine-add"), "{tree}");
            assert!(tree.contains("ma-message-host"), "{tree}");
            assert!(tree.contains("kom machine add host"), "{tree}");
            window.remove_window();
        })
        .unwrap();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    assert!(
        !calls
            .effects
            .iter()
            .any(|(name, _)| name == "openOverlay" || name == "closeOverlay"),
        "the picker stays open without a setup dialog"
    );
}

#[gpui::test]
fn option_key_settings_are_only_shown_on_macos(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for platform in ["linux", "windows", "macos"] {
        let mut p = props();
        p["section"] = json!("keyboard");
        let mut values = fixture(true, p, updates("current", "current"));
        values.insert("window".into(), json!({"platform":platform}));
        let tree = describe_with_values(cx, &staged, "frame/settings", values, None).unwrap();
        for value in ["auto", "on", "off", "left", "right"] {
            assert_eq!(
                tree.contains(&format!("st-option-{value}")),
                platform == "macos",
                "{platform}: {tree}"
            );
        }
        assert!(
            tree.contains("st-s-keyboard"),
            "{platform}: Keyboard must be in the nav: {tree}"
        );
    }
}

#[gpui::test]
fn connect_lists_a_found_system_and_other(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(
            json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null}),
            Value::Null,
        ),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-c-grokbot"), "{tree}");
    assert!(tree.contains("st-c-other"), "{tree}");
}

#[gpui::test]
fn connect_hides_a_system_it_does_not_find(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(Value::Null, Value::Null),
        None,
    )
    .unwrap();
    assert!(!tree.contains("st-c-grokbot"), "{tree}");
    assert!(tree.contains("st-c-other"), "{tree}");
}

#[gpui::test]
fn connect_shows_a_joined_system_as_connected(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(
            json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null}),
            json!({"connect": {"grokbot": {"agent": "grok"}}}),
        ),
        None,
    )
    .unwrap();
    assert!(tree.contains("Connected as grok"), "{tree}");
}

#[gpui::test]
fn connect_prompt_holds_the_lead_and_vault(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(
            json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null}),
            Value::Null,
        ),
        Some("this.connect.showPrompt = true;"),
    )
    .unwrap();
    assert!(tree.contains("--to opus"), "{tree}");
    assert!(tree.contains("/fixture/vault"), "{tree}");
}

fn connect_values(found: Value, person: Value) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("connect");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("appLink".into(), found);
    values.insert("person".into(), person);
    values
}

#[gpui::test]
fn settings_sections_and_updates(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let start = Instant::now();
    let staged = Staged::new();
    for section in ["appearance", "agents", "notifications", "machines", "about"] {
        let mut p = props();
        p["section"] = json!(section);
        describe(
            cx,
            &staged,
            "frame/settings",
            true,
            p,
            updates("current", "current"),
            None,
        )
        .unwrap();
    }
    for app in ["source", "current", "ready"] {
        for core in ["current", "ready", "error", "managed", "updating"] {
            let mut p = props();
            p["section"] = json!("about");
            let tree = describe(
                cx,
                &staged,
                "frame/settings",
                true,
                p,
                updates(app, core),
                None,
            )
            .unwrap_or_else(|e| panic!("updates/{app}/{core}: {e:#}"));
            if app == "current" && core == "current" {
                assert!(
                    tree.contains("check-app") && tree.contains("check-core"),
                    "both update buttons must exist: {tree}"
                );
            }
        }
    }
    for (checking, checked, asked) in [
        (false, None, false),
        (false, None, true),
        (true, None, false),
        (true, None, true),
        (true, Some(1700000000u64), false),
        (true, Some(1700000000), true),
        (false, Some(1700000000), false),
        (false, Some(1700000000), true),
    ] {
        let mut p = props();
        p["section"] = json!("about");
        let mut u = updates("current", "current");
        u["checked"] = json!(checked);
        u["checking"] = json!(checking);
        let setup = asked.then_some("this.asked = Date.now();");
        let tree = describe(cx, &staged, "frame/settings", true, p, u, setup).unwrap();
        let succeeded = !checking && checked.is_some();
        assert_eq!(
            tree.contains("Up to date"),
            succeeded,
            "success requires a completed, successful check: {tree}"
        );
        if !checking && checked.is_none() {
            assert!(
                tree.contains("check-app") && tree.contains("check-core"),
                "failed checks must remain retryable: {tree}"
            );
        }
        assert!(
            tree.contains(if checking {
                "Checking"
            } else if succeeded {
                "Up to date"
            } else {
                "Check your connection"
            }),
            "check status missing: {tree}"
        );
    }
    eprintln!("27 settings cases, each twice: {:?}", start.elapsed());
}

fn welcome_values(person: Value, known: Value) -> BTreeMap<String, Value> {
    let mut values = fixture(false, props(), updates("current", "current"));
    values.insert("person".into(), person);
    values.insert("knownAgents".into(), known);
    values
}

#[gpui::test]
fn welcome_step_one_offers_install_and_the_command_for_this_os(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/welcome",
        welcome_values(json!({"lead": {"program": "opencode"}}), welcome_agents()),
        Some("this.step = 1;"),
    )
    .unwrap();
    assert!(tree.contains("Install"), "{tree}");
    assert!(
        tree.contains("curl -fsSL https://opencode.ai/v2/install | bash"),
        "OpenCode's Linux command: {tree}"
    );
    assert!(!tree.contains("w-get-opencode"), "it has a command here: {tree}");
}

#[gpui::test]
fn welcome_step_two_draws_the_folder_from_settings(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/welcome",
        welcome_values(
            json!({"lead": {"program": "claude", "folder": "/fixture/projects"}}),
            welcome_agents(),
        ),
        Some("this.step = 2;"),
    )
    .unwrap();
    assert!(tree.contains("/fixture/projects"), "{tree}");
    assert!(tree.contains("w-choose-folder"), "{tree}");
}

#[gpui::test]
fn welcome_step_three_shows_the_toggle_only_for_claude(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (program, shown) in [("claude", true), ("opencode", false)] {
        let tree = describe_settled(
            cx,
            &staged,
            "frame/welcome",
            welcome_values(json!({"lead": {"program": program}}), welcome_agents()),
            Some("this.step = 3;"),
        )
        .unwrap();
        assert_eq!(
            tree.contains("Let Claude Code use Kompitum"),
            shown,
            "{program}: {tree}"
        );
    }
}

#[gpui::test]
fn settings_general_draws_the_home_folder_row(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("general");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), json!({"lead": {"folder": "/fixture/projects"}}));
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    assert!(tree.contains("Home folder"), "{tree}");
    assert!(tree.contains("/fixture/projects"), "{tree}");
    assert!(tree.contains("st-general-folder"), "{tree}");
    assert!(tree.contains("Show again"), "{tree}");
}

#[gpui::test]
fn settings_nav_lists_every_section_and_an_unknown_one_opens_appearance(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    for id in [
        "general",
        "appearance",
        "agents",
        "vault",
        "connect",
        "notifications",
        "machines",
        "keyboard",
        "about",
    ] {
        assert!(tree.contains(&format!("st-s-{id}")), "missing {id}: {tree}");
    }
    assert!(tree.contains("st-close"), "{tree}");
    assert!(tree.contains("st-feedback"), "{tree}");
    assert!(tree.contains("Send feedback"), "{tree}");

    let mut p = props();
    p["section"] = json!("no-such-section");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-zoom"), "unknown sections open Appearance: {tree}");
    assert!(tree.contains("st-s-appearance"), "{tree}");
}

#[gpui::test]
fn appearance_draws_theme_colours_and_layout_rows(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        Some("this.themeOpen = true;"),
    )
    .unwrap();
    assert!(tree.contains("Theme"), "{tree}");
    assert!(tree.contains("Colours"), "{tree}");
    assert!(tree.contains("Layout"), "{tree}");
    assert!(tree.contains("st-colours-graphite"), "{tree}");
    for layout in ["main", "desk", "win98", "terminals"] {
        assert!(tree.contains(&format!("st-layout-{layout}")), "missing {layout}: {tree}");
    }
    assert!(!tree.contains("Everything: terminals, panels, text."), "{tree}");
}

#[gpui::test]
fn about_combines_the_versions_and_says_how_updates_arrive(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (app, sentence) in [
        ("current", "Checks for new versions of the app and its core every hour."),
        ("source", "Built from source; updates come from your checkout."),
    ] {
        let mut p = props();
        p["section"] = json!("about");
        let tree = describe(
            cx,
            &staged,
            "frame/settings",
            true,
            p,
            updates(app, "current"),
            None,
        )
        .unwrap();
        assert!(tree.contains("Kompitum 0.1.0 · core 0.1.5"), "{tree}");
        assert!(tree.contains(sentence), "{tree}");
        assert!(!tree.contains("KOMPITUM-CORE"), "{tree}");
    }
}

#[gpui::test]
fn vault_says_not_set_without_a_vault(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("vault");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("vaultMissing".into(), json!(true));
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    assert!(tree.contains("Notes your agents leave each other, kept in Git."), "{tree}");
    assert!(tree.contains("Not set"), "{tree}");
    assert!(tree.contains("Couldn't update"), "{tree}");
    assert!(tree.contains("st-vault-retry"), "{tree}");
    assert!(tree.contains("No Git vault at /fixture/vault."), "{tree}");
    assert!(!tree.contains("Unknown"), "{tree}");
    assert!(!tree.contains("Reading…"), "{tree}");
}

#[gpui::test]
fn connect_opens_with_one_line_and_hides_how_it_works(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let found = json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(found.clone(), Value::Null),
        None,
    )
    .unwrap();
    assert!(tree.contains("Bring another agent into Kompitum."), "{tree}");
    assert!(tree.contains("How it works"), "{tree}");
    assert!(!tree.contains("Connect copies a prompt for them"), "{tree}");
    assert!(tree.contains("st-c-connect-grokbot"), "{tree}");
    assert!(tree.contains("st-c-prompt-grokbot"), "{tree}");
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(found, Value::Null),
        Some("this.connect.showHow = true;"),
    )
    .unwrap();
    assert!(
        tree.contains("Connect copies a prompt for them; they show as connected once their Joined note reaches the vault"),
        "{tree}"
    );
}

#[gpui::test]
fn shortcuts_sheet_uses_titles_and_collapses_numbered_rows(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "mods/help",
        true,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("Keyboard shortcuts"), "{tree}");
    assert!(tree.contains("Alt"), "{tree}");
    assert!(tree.contains("Go to space 1–9"), "{tree}");
    assert!(tree.contains("Go to tab 1–9"), "{tree}");
    assert!(!tree.contains("space select 1"), "{tree}");
}

fn agent_catalog() -> Value {
    json!([
        {
            "key": "claude", "name": "Claude Code", "icon": "icons/claude.svg", "tint": false,
            "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
            "run": ["claude"], "installed": true,
            "flags": {"model": ["--model", "{model}"], "effort": ["--effort", "{effort}"]},
            "efforts": ["low", "medium", "high", "xhigh", "max"],
            "models": [{"id": "default", "label": "Default"}, {"id": "sonnet", "label": "Sonnet"}],
            "reports": "status_line"
        },
        {
            "key": "command-code", "name": "Command Code", "icon": "icons/command-code.svg", "tint": true,
            "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
            "run": ["commandcode"], "installed": true,
            "flags": {"model": ["-m", "{model}"], "effort": ["--effort", "{effort}"]},
            "efforts": ["low", "medium", "high", "max"],
            "models": [{"id": "claude-sonnet-4-6", "label": "Claude Sonnet 4.6"}],
            "worker": "command_code", "route": "commandcode/{model}#{effort}",
            "plans": [{"id": "go", "label": "Go", "windows": {"5H": 3, "Week": 6, "Month": 10}}]
        },
        {
            "key": "opencode", "name": "OpenCode", "icon": "icons/opencode.svg", "tint": true,
            "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
            "run": ["opencode"], "installed": true,
            "flags": {"model": ["-m", "{model}#{effort}"]},
            "efforts": ["low", "medium", "high", "max"],
            "models": [{"id": "opencode-go/deepseek-v4.1-flash", "label": "DeepSeek V4.1 Flash"}],
            "worker": "opencode", "route": "{model}#{effort}",
            "plans": [{"id": "go", "label": "Go", "models": {"opencode-go/deepseek-v4.1-flash": {"5H": 20, "Week": 50, "Month": 100}}}]
        }
    ])
}

/// The welcome's catalog: the fields part 2 added and part 6's install, login and signed-in.
fn welcome_agents() -> Value {
    json!([
        {
            "key": "opencode", "name": "OpenCode", "icon": "icons/opencode.svg", "tint": true,
            "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
            "run": ["opencode"], "installed": false,
            "flags": {"model": ["-m", "{model}#{effort}"]},
            "efforts": ["low", "medium", "high", "max"],
            "models": [{"id": "opencode-go/deepseek-v4.1-flash", "label": "DeepSeek V4.1 Flash"}],
            "install": {
                "macos": ["sh", "-c", "curl -fsSL https://opencode.ai/v2/install | bash"],
                "linux": ["sh", "-c", "curl -fsSL https://opencode.ai/v2/install | bash"],
                "page": "https://opencode.ai/docs/"
            },
            "login": ["opencode", "auth", "login"]
        },
        {
            "key": "claude", "name": "Claude Code", "icon": "icons/claude.svg", "tint": false,
            "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
            "run": ["claude"], "installed": true,
            "flags": {"model": ["--model", "{model}"], "effort": ["--effort", "{effort}"]},
            "efforts": ["low", "medium", "high", "xhigh", "max"],
            "models": [{"id": "default", "label": "Default"}, {"id": "sonnet", "label": "Sonnet"}],
            "install": {
                "windows": ["powershell", "-NoProfile", "-Command", "irm https://claude.ai/install.ps1 | iex"],
                "linux": ["sh", "-c", "curl -fsSL https://claude.ai/install.sh | bash"],
                "page": "https://code.claude.com/docs/en/setup"
            },
            "login": ["claude", "auth", "login"],
            "signed_in": ["claude", "auth", "status"]
        }
    ])
}

fn agent_workers(order: Value, workers: Value) -> Value {
    json!({
        "mode": "auto", "order": order, "chain": order, "next": null, "all_out": null,
        "warnings": [], "clis": [], "asking": [], "workers": workers
    })
}

fn agent_worker(
    key: &str,
    label: &str,
    cli: &str,
    route: &str,
    state: &str,
    window: &str,
    until: u64,
) -> Value {
    json!({
        "key": key, "label": label, "cli": cli, "route": route, "state": state,
        "until_ms": until, "source": null, "window": window, "message": "",
        "sentence": if window.is_empty() { String::new() } else { format!("{label} is out of its {window} limit, back at 10:00.") },
        "refusals": 0,
        "spend": {
            "five_hours": {"runs": 0, "cost_usd": 0.0},
            "week": {"runs": 0, "cost_usd": 0.0},
            "month": {"runs": 0, "cost_usd": 0.0}
        }
    })
}

fn agents_values(known: Value, workers: Value, status: Value) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("agents");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("knownAgents".into(), known);
    values.insert("workersJson".into(), workers);
    values.insert("statusJson".into(), status);
    values
}

#[gpui::test]
fn setting_search_finds_rows_and_says_when_nothing_matches(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        props(),
        updates("current", "current"),
        Some("this.search = 'zoom';"),
    )
    .unwrap();
    assert!(tree.contains("Appearance › Zoom"), "{tree}");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        props(),
        updates("current", "current"),
        Some("this.search = 'qqq';"),
    )
    .unwrap();
    assert!(tree.contains(r#"No setting matches \"qqq\""#), "{tree}");
}

#[gpui::test]
fn agent_plans_show_claude_limits_and_exhaustion(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker(
                "cc",
                "Command Code",
                "command_code",
                "commandcode/deepseek/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            ),
            agent_worker(
                "go",
                "opencode Go",
                "opencode",
                "opencode-go/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            )
        ]),
    );
    let status = json!({
        "tasks": {"review": [], "running": [], "queued": []},
        "claude": {
            "five_hour_used_percentage": 42.0, "five_hour_resets_at": 4102444800u64,
            "seven_day_used_percentage": 61.0, "seven_day_resets_at": 4102444800u64
        }
    });
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers.clone(), status),
        None,
    )
    .unwrap();
    assert!(tree.contains("5H 42% · resets"), "{tree}");
    assert!(tree.contains("Week 61% · resets"), "{tree}");
    assert!(tree.contains("st-plan-command-code-go"), "{tree}");
    assert!(tree.contains("st-plan-opencode-go"), "{tree}");
    let status = json!({
        "tasks": {"review": [], "running": [], "queued": []},
        "claude": {
            "five_hour_used_percentage": 100.0, "five_hour_resets_at": 4102444800u64,
            "seven_day_used_percentage": 61.0, "seven_day_resets_at": 4102444800u64
        }
    });
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers.clone(), status.clone()),
        None,
    )
    .unwrap();
    assert!(tree.contains("5H exhausted · resets"), "{tree}");
}

#[gpui::test]
fn agent_workers_follow_the_core_order_and_show_out_windows(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["go", "cc"]),
        json!([
            agent_worker(
                "cc",
                "Command Code",
                "command_code",
                "commandcode/deepseek/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            ),
            agent_worker(
                "go",
                "opencode Go",
                "opencode",
                "opencode-go/deepseek-v4.1-flash#max",
                "out",
                "5-hour",
                4102444800000u64
            )
        ]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers, status),
        None,
    )
    .unwrap();
    let go = tree
        .find("st-worker-go")
        .unwrap_or_else(|| panic!("go row missing: {tree}"));
    let cc = tree
        .find("st-worker-cc")
        .unwrap_or_else(|| panic!("cc row missing: {tree}"));
    assert!(go < cc, "workers must follow the core's order: {tree}");
    assert!(tree.contains("5H exhausted · resets"), "{tree}");
}

#[gpui::test]
fn agent_lead_pickers_show_cli_model_and_reasoning(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker(
                "cc",
                "Command Code",
                "command_code",
                "commandcode/deepseek/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            ),
            agent_worker(
                "go",
                "opencode Go",
                "opencode",
                "opencode-go/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            )
        ]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers.clone(), status.clone()),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-lead-cli-claude"), "{tree}");
    assert!(tree.contains("st-lead-cli-opencode"), "{tree}");
    assert!(tree.contains("st-lead-model"), "{tree}");
    assert!(tree.contains("Default"), "{tree}");
    assert!(tree.contains("st-lead-effort"), "{tree}");
    assert!(!tree.contains("st-lead-effort-low"), "the reasoning menu stays closed: {tree}");
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers.clone(), status.clone()),
        Some("this.agentPage.effortOpen = true;"),
    )
    .unwrap();
    assert!(tree.contains("st-lead-effort-low"), "{tree}");
    assert!(tree.contains("st-lead-effort-max"), "{tree}");
}

#[gpui::test]
fn animated_images_advance_with_explicit_and_implicit_ids(cx: &mut TestAppContext) {
    use gpui::{
        IntoElement as _,
        http_client::{AsyncBody, FakeHttpClient, Response},
    };
    use image::{Delay, Frame, Rgba, RgbaImage, codecs::gif::GifEncoder};
    use std::{sync::Arc, time::Duration};

    cx.update(gpui_shell::init);
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            encoder
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(2, 2, Rgba(color)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(20, 1),
                ))
                .unwrap();
        }
    }
    let client = FakeHttpClient::create(move |_| {
        let bytes = bytes.clone();
        async move {
            Ok(Response::builder()
                .status(200)
                .body(AsyncBody::from(bytes))
                .unwrap())
        }
    });
    cx.update(|cx| cx.set_http_client(client));
    let staged = Staged::new();
    for explicit in [false, true] {
        let runtime = ShellRuntime::new_isolated().unwrap();
        let url = format!("https://images.example/animation-{explicit}.gif");
        let source = format!(
            "import {{ View, image }} from 'gpui-kit'; export default class Demo extends View {{ render() {{ return image('{url}'){}.size(20); }} }}",
            if explicit { ".id('clip')" } else { "" }
        );
        std::fs::write(staged.0.join("main.js"), source).unwrap();
        let context = cx.add_empty_window();
        let view = context.update(|window, cx| {
            window.activate_window();
            let root = runtime.try_load(&staged.0, window, cx).unwrap();
            root.read(cx).content().clone()
        });
        let draw = |context: &mut gpui::VisualTestContext| {
            let view = view.clone();
            context.draw(
                gpui::Point::default(),
                gpui::size(gpui::px(40.), gpui::px(40.)),
                move |_, _| view.into_any_element(),
            );
        };
        draw(context);
        context.run_until_parked();
        draw(context);
        assert!(context.update(|window, _| window.is_window_active()));
        let resource = gpui::Resource::Uri(url.into());
        let data: Arc<gpui::RenderImage> = context.update(|_, cx| {
            cx.fetch_asset::<gpui::ImgResourceLoader>(&resource)
                .unwrap()
                .unwrap()
        });
        assert_eq!(data.frame_count(), 2);
        assert!(!context.update(|window, _| window.has_image_atlas_entry(&data)));
        // GPUI's image clock is wall time, not the test executor's clock.
        std::thread::sleep(Duration::from_millis(25));
        draw(context);
        assert!(
            context.update(|window, _| window.has_image_atlas_entry(&data)),
            "both animation frames must have painted (explicit id: {explicit})"
        );
    }
}

#[gpui::test]
fn settings_keep_notes_in_the_whats_new_window(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = json!([
        {"version":"1.8.0","date":"2026-10-26","items":[{"headline":"Newest headline","detail":"Newest details","fixed":false,"media":[]},{"headline":"","detail":"Newest fix","fixed":true,"media":[]}],"spotlight":true},
        {"version":"1.7.0","date":"2026-10-25","items":[{"headline":"Older headline","detail":"Older details","fixed":false,"media":[]},{"headline":"Another older headline","detail":"Hidden detail","fixed":false,"media":[]}],"spotlight":true}
    ]);
    let mut update = updates("ready", "current");
    update["notes"] = notes.clone();
    let mut p = props();
    p["section"] = json!("about");
    let tree = describe(cx, &staged, "frame/settings", true, p, update.clone(), None).unwrap();
    assert!(
        tree.contains("st-whats-new")
            && tree.contains("st-app-update")
            && tree.contains("st-check-core")
    );
    assert!(!tree.contains("Newest details") && !tree.contains("Older headline"));
    let mut p = props();
    p["notes"] = notes;
    let tree = describe(cx, &staged, "frame/whatsnew", true, p, update, None).unwrap();
    assert!(tree.contains("Newest details") && tree.contains("Older details"));
    assert!(tree.contains("Also fixed") && tree.contains("Newest fix"));
    assert!(tree.contains("Oct 26") && !tree.contains("2026-10-26"));
}

#[gpui::test]
fn local_state_and_overlay_initial_descriptions(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let start = Instant::now();
    let staged = Staged::new();
    for page in 1..=3 {
        let tree = describe(
            cx,
            &staged,
            "frame/welcome",
            true,
            props(),
            updates("current", "current"),
            Some(&format!("this.step = {page};")),
        )
        .unwrap();
        let expected = ["Pick your lead", "Pick a home folder", "Connect agents"][page - 1];
        assert!(tree.contains(expected), "welcome page {page}: {tree}");
    }
    for query in ["settings", "no-such-command"] {
        let tree = describe(
            cx,
            &staged,
            "frame/palette",
            true,
            props(),
            updates("current", "current"),
            Some(&format!("this.query = {query:?};")),
        )
        .unwrap();
        // The query sits in a native text field; what the list shows tells it apart.
        let expected = if query == "settings" {
            "Settings"
        } else {
            "Nothing matches"
        };
        assert!(tree.contains(expected), "palette query {query}: {tree}");
    }
    // Reviewed overlay contract table, not extracted from arbitrary JS call sites.
    let scenarios = [
        (
            "mods/menu",
            json!({"tab":"pane","target":{"pane":"p1","terminal":"t1"}}),
        ),
        (
            "mods/menu",
            json!({"tab":"tab","target":{"id":"tab1","space":"s1"}}),
        ),
        ("mods/menu", json!({"tab":"space","target":{"id":"s1"}})),
        ("mods/prompt", json!({"machines":true})),
        (
            "mods/confirm",
            json!({"title":"Close space?","command":{"name":"space.close","args":{"id":"s1"}}}),
        ),
        (
            "mods/toast",
            json!({"title":"Done","body":"Build complete","level":"success","action":{"name":"app.help"}}),
        ),
        ("mods/peek", json!({"terminal":"t1"})),
        ("mods/peek", json!({"terminal":"missing"})),
        ("mods/worktree", json!({"space":"s1","action":"new"})),
        ("mods/worktree", json!({"space":"s1","action":"remove"})),
    ];
    let entries = staged.entries();
    for (entry, patch) in scenarios {
        assert!(
            entries.iter().any(|e| e == entry),
            "orphan scenario: {entry}"
        );
        let mut p = props();
        p.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        describe(
            cx,
            &staged,
            entry,
            true,
            p,
            updates("current", "current"),
            None,
        )
        .unwrap();
    }
    eprintln!(
        "18 local-state/overlay cases, each twice: {:?}",
        start.elapsed()
    );
}

#[gpui::test]
fn duplicate_element_negative_control(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let dir = staged.0.join("negative");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("main.js"), "import { View, div } from 'gpui-kit'; export default class Bad extends View { render() { const child = div().child('reused'); return div().child(div().child(child)).child(div().child(child)); } }").unwrap();
    let error = describe(
        cx,
        &staged,
        "negative",
        false,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("already added to a parent"),
        "{error:#}"
    );
}

#[test]
fn shipped_layouts_and_keys() {
    let staged = Staged::new();
    let mods: BTreeSet<_> = staged
        .entries()
        .into_iter()
        .filter_map(|p| p.strip_prefix("mods/").map(str::to_owned))
        .collect();
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("defaults/layouts");
    let mut layouts = BTreeSet::new();
    fn references(v: &Value, mods: &BTreeSet<String>) {
        match v {
            Value::Object(o) => {
                if let Some(Value::String(name)) = o.get("mod") {
                    assert!(mods.contains(name), "unknown layout mod {name}");
                }
                for v in o.values() {
                    references(v, mods);
                }
            }
            Value::Array(a) => {
                for v in a {
                    references(v, mods);
                }
            }
            _ => {}
        }
    }
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str::<Node>(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        references(&serde_json::from_str::<Value>(&text).unwrap(), &mods);
        layouts.insert(path.file_stem().unwrap().to_str().unwrap().to_owned());
    }
    assert_eq!(
        layouts,
        crate::window::shipped_layouts()
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    let mod_commands = ["ribbon.next"];
    for text in [
        include_str!("../../defaults/keys.json"),
        include_str!("../../defaults/keys.macos.json"),
    ] {
        crate::keys::parse(text).unwrap();
        let sections: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_str(text).unwrap();
        for bindings in sections.values() {
            for action in bindings.values() {
                let name = action.split(':').next().unwrap();
                if let Some(command) = crate::commands::find(name) {
                    let args = match action.split_once(':') {
                        Some((_, value)) => {
                            assert!(
                                !command.args.is_empty(),
                                "unexpected shipped key argument {action}"
                            );
                            crate::commands::shorthand(command, value)
                        }
                        None => Value::Null,
                    };
                    crate::commands::validate_args(command, &args).unwrap();
                    assert!(
                        crate::commands::missing(command, &args).is_none(),
                        "missing shipped key argument {action}"
                    );
                }
                assert!(
                    action == "terminal"
                        || action.starts_with("send:")
                        || crate::commands::COMMANDS.iter().any(|c| c.name == name)
                        || (mod_commands.contains(&name)
                            && mods.contains(name.split('.').next().unwrap())),
                    "unknown shipped key action {action}"
                );
            }
        }
    }
}

#[gpui::test]
fn picker_scrolls_the_selection_into_view(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    let tabs: Vec<Value> = (1..=12)
        .map(|i| json!({"id":format!("tab{i}"),"index":i,"name":format!("Task {i}"),"active":i==1,"status":"done","panes":1,"zoomed":false,"settled":false,"settled_at":null,"changed_at":null,"working_since":null}))
        .collect();
    values.insert("spaces".into(), json!([
        {"id":"s1","index":1,"name":"Local","folder":"/fixture/project","machine":null,"branch":"main","active":true,"status":"working","tabs":tabs}
    ]));
    // Selecting the last row (with the arrows) must scroll it into the visible 320px.
    let tree = describe_with_values(
        cx,
        &staged,
        "mods/picker",
        values,
        Some("this.at = 999; cx.notify();"),
    )
    .unwrap();
    let top = tree
        .lines()
        .find(|l| l.contains("pk-space-here"))
        .unwrap_or_else(|| panic!("selected row missing: {tree}"));
    let px: f64 = top
        .split(".top[Number(")
        .nth(1)
        .unwrap_or_else(|| panic!("selected row must be placed inside the scrolled list: {tree}"))
        .split(")")
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        px <= 320.0 - 32.0,
        "selected row at {px}px is below the visible list: {tree}"
    );
}

#[gpui::test]
fn status_bar_shows_the_real_palette_key(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut bound = fixture(true, props(), updates("current", "current"));
    bound.insert(
        "keys".into(),
        json!([{"section":"global","keys":"ctrl-p","action":"app.palette"}]),
    );
    let tree = describe_with_values(cx, &staged, "mods/workspace-status", bound, None).unwrap();
    assert!(
        tree.contains("Ctrl+P"),
        "hint must show the real binding: {tree}"
    );
    assert!(!tree.contains("Shift P"), "stale hardcoded hint: {tree}");

    let mut unbound = fixture(true, props(), updates("current", "current"));
    unbound.insert(
        "keys".into(),
        json!([{"section":"global","keys":"ctrl-k","action":"app.help"}]),
    );
    let tree = describe_with_values(cx, &staged, "mods/workspace-status", unbound, None).unwrap();
    assert!(
        tree.contains("commands"),
        "unbound hint still offers commands: {tree}"
    );
    assert!(
        !tree.contains("Shift P"),
        "unbound hint must not invent a key: {tree}"
    );
}

#[gpui::test]
fn the_mailbox_lists_both_tabs_and_a_row_per_note(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/mailbox",
        fixture(true, props(), updates("current", "current")),
        Some("this.tab = 'all';"),
    )
    .unwrap();
    assert!(tree.contains("Needs you"), "{tree}");
    assert!(tree.contains("All notes"), "{tree}");
    assert!(tree.contains("Please review"), "{tree}");
    assert!(tree.contains("Progress on the build"), "{tree}");
    assert!(
        tree.contains("fixture: a note's mark file is damaged"),
        "problems must be drawn in words: {tree}"
    );
}

#[gpui::test]
fn the_mailbox_opens_a_note_with_answer_and_close(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["note"] = json!("manager/to/owner/2026-11-08-please-review.md");
    let tree = describe_settled(
        cx,
        &staged,
        "frame/mailbox",
        fixture(true, p, updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("Please review"), "{tree}");
    assert!(
        tree.contains("mail-answer") && tree.contains("Answer"),
        "{tree}"
    );
    assert!(
        tree.contains("mail-close") && tree.contains("Close"),
        "{tree}"
    );
    assert!(
        tree.contains("A body from the vault"),
        "the body is read from the vault: {tree}"
    );
}

#[gpui::test]
fn the_mailbox_answer_opens_a_multi_line_field_with_send(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(cx, &staged, "frame/mailbox", fixture(true, props(), updates("current", "current")),
        Some(
            "this.selected = { path: 'manager/to/owner/2026-11-08-please-review.md', from: 'manager', \
             to: 'owner', kind: 'request', title: 'Please review', date: '2026-11-08T09:00:00+03:00' }; \
             this.answerNote(cx);",
        ),
    )
    .unwrap();
    assert!(
        tree.contains("mail-send") && tree.contains("Send"),
        "{tree}"
    );
    assert!(
        tree.contains("mail-cancel") && tree.contains("Cancel"),
        "{tree}"
    );
    assert!(
        tree.to_lowercase().contains("textarea"),
        "the field must be multi-line: {tree}"
    );
}

#[gpui::test]
fn the_vault_section_draws_its_folder_and_mailbox_button(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("vault");
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        fixture(true, p, updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-s-vault"), "{tree}");
    assert!(tree.contains("/fixture/vault"), "{tree}");
    assert!(tree.contains("2 notes not sent yet"), "{tree}");
    assert!(
        tree.contains("fixture: a note's mark file is damaged"),
        "{tree}"
    );
    assert!(tree.contains("1 note"), "{tree}");
    assert!(tree.contains("Open the mailbox"), "{tree}");
}

#[gpui::test]
fn the_notifications_section_draws_the_popup_toggles(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("notifications");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("Pop-ups"), "{tree}");
    assert!(tree.contains("Notes between agents"), "{tree}");
    assert!(tree.contains("Quiet while a game is fullscreen"), "{tree}");
    assert!(tree.contains("st-notes-between-agents"), "{tree}");
    assert!(tree.contains("st-quiet-fullscreen"), "{tree}");
}
