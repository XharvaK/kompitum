import { View, div, image, svg } from "gpui-kit";
import { h_flex, v_flex, Popover } from "gpui-base";
import { theme, run, props, window, shared, setShared, updates, layout, spaces, knownAgents, pickFolder, notify, kom, agents, lead, openOverlay } from "kompitum";

function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

// The sidebar's rail goes by its width, not a flag: dragging its edge narrows it too.
const narrow = () => (find(layout(), "sidebar")?.size?.px ?? Infinity) < 160;

// To the rail (on macOS wide enough for the traffic lights), or back to the width it had.
function toggleSidebar() {
  const width = find(layout(), "sidebar")?.size?.px;
  if (!narrow()) setShared("sidebar.width", width);
  // Mods edited before the rail went by width still read the flag.
  setShared("sidebar.collapsed", !narrow());
  run("layout.resize", { id: "sidebar", px: narrow() ? (shared("sidebar.width") ?? 268) : window()?.platform === "macos" ? 76 : 44 });
}

function firstLine(text) {
  return String(text ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";
}

export default class WorkspaceChrome extends View {
  // Only the nav copy reads the tasks, and only the foot reads the notes: the brand row needs neither.
  init(_props, cx) {
    this.spaceOpen = false;
    this.agents = [];
    this.review = 0;
    this.busy = 0;
    this.tasksError = null;
    this.notes = 0;
    if (props()?.footer) {
      this.readNotes(cx);
      cx.timer.every(60000, (cx) => this.readNotes(cx));
    }
    if (!props()?.nav) return;
    this.readTasks(cx);
    cx.timer.every(10000, (cx) => this.readTasks(cx));
  }

  // The notes waiting for the Owner, counted from the vault for the bell. A failed read keeps the last count.
  readNotes(cx) {
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["vault", "open", "--json", "--as", "owner"]);
        if (r?.code === 0) {
          const data = JSON.parse(r.stdout || "{}");
          this.notes = Array.isArray(data?.notes) ? data.notes.filter((n) => n && typeof n === "object").length : 0;
        }
      } catch {}
      cx.notify();
    });
  }

  // The tasks from `kom status --json`: those waiting for review, and those running or queued. A failed
  // read keeps the last counts and names its error on the Tasks row.
  readTasks(cx) {
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["status", "--json"]);
        if (r?.code !== 0) {
          this.tasksError = firstLine(r?.stderr || r?.stdout) || `exit ${r?.code ?? "?"}`;
          cx.notify();
          return;
        }
        const data = JSON.parse(r.stdout || "{}");
        const review = Array.isArray(data?.tasks?.review) ? data.tasks.review.length : data?.waiting?.review;
        this.review = typeof review === "number" ? review : 0;
        this.busy = ["running", "queued"].reduce((n, key) => n + (Array.isArray(data?.tasks?.[key]) ? data.tasks[key].length : 0), 0);
        this.tasksError = null;
      } catch (error) {
        this.tasksError = firstLine(error?.message ?? error) || "It didn't work.";
      }
      cx.notify();
    });
  }

  // The Tasks row's tooltip: what runs and what waits for review, or the read's error.
  tasksTip() {
    if (this.tasksError) return `Tasks · couldn't read them: ${this.tasksError}`;
    const parts = [];
    if (this.busy > 0) parts.push(`${this.busy} running`);
    if (this.review > 0) parts.push(`${this.review} to review`);
    return parts.length ? parts.join(" · ") : "No tasks";
  }

  // Which agents are installed, read each time the menu opens, so one installed meanwhile shows.
  loadAgents(cx) {
    cx.spawn(async (cx) => {
      try {
        this.agents = await knownAgents();
      } catch {}
      cx.notify();
    });
  }

  button(id, label, action) {
    const t = theme();
    return div().id(id).size(26).flex_none().flex().items_center().justify_center().rounded(6).text_color(t.muted).hover(s => s.bg(t.hover).text_color(t.foreground))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click(action).child(label);
  }

  closeSpaceMenu(cx) {
    this.spaceOpen = false;
    cx.notify();
  }

  // A picked folder is a space already, or becomes one.
  openFolder(cx) {
    this.closeSpaceMenu(cx);
    cx.spawn(async (cx) => {
      try {
        const folder = await pickFolder({ prompt: "Open" });
        if (!folder) return;
        const space = spaces().find((p) => p.folder === folder);
        if (space) run("space.select", { id: space.id });
        else run("space.new", { folder });
      } catch (e) {
        notify({ title: "Couldn't open that folder", body: String(e?.message ?? e), level: "error" });
      }
    });
  }

  // The space's "+": the current space, then a new terminal or any installed agent.
  spaceMenu(t) {
    const space = spaces().find((p) => p.active);
    const row = (id, action) => h_flex()
      .id(id)
      .min_h(30)
      .px(8)
      .gap(8)
      .items_center()
      .rounded(6)
      .cursor_pointer()
      .text_size(13)
      .hover((s) => s.bg(t.hover))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click(action);
    const content = v_flex()
      .w(280)
      .p(4)
      .gap(1)
      .rounded(10)
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .child(
        row("space-current", (_e, cx) => this.openFolder(cx))
          .tooltip("Open another folder as the space")
          .child(svg("mods/workspace-chrome/folder.svg").size(15).flex_none().text_color(t.muted))
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .child(div().truncate().font_semibold().text_color(t.foreground).child(space?.name ?? ""))
              .child(div().truncate().text_size(11.5).text_color(t.dim).child(space?.folder ?? "")),
          ),
      )
      .child(div().h(1).my(4).bg(t.border))
      .child(
        row("space-new-terminal", (_e, cx) => { run("tab.new", {}); this.closeSpaceMenu(cx); })
          .child(svg("mods/workspace-chrome/terminal.svg").size(15).flex_none().text_color(t.muted))
          .child(div().text_color(t.foreground).child("New terminal")),
      )
      .children(
        this.agents.filter((agent) => agent.installed).map((agent) =>
          row(`space-new-${agent.key}`, (_e, cx) => {
            run("tab.new", { argv: agent.run });
            this.closeSpaceMenu(cx);
          })
            .when(Boolean(agent.icon), (el) => el.child(agent.tint
              ? svg(agent.icon).size(14).flex_none().text_color(t.muted)
              : image(agent.icon).w(14).h(14).flex_none()))
            .child(div().text_color(t.foreground).child(agent.name))),
      );
    const trigger = h_flex().id("space-menu").w(28).h(28).flex_none().items_center().justify_center().rounded(7)
      .cursor_pointer().text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
      .tooltip("Space and new terminals")
      .child(svg("mods/workspace-chrome/plus.svg").size(16));
    return Popover.new("space-menu-menu")
      .anchor("top_left")
      .open(Boolean(this.spaceOpen))
      .on_open_change((open, cx) => {
        this.spaceOpen = open;
        if (open) this.loadAgents(cx);
        cx.notify();
      })
      .trigger(trigger)
      .content(content);
  }
  // A sidebar row: an icon, and its label beside it unless the rail is narrow.
  navRow(t, collapsed, id, icon, label, tip, action) {
    return h_flex().id(id).h(36).px(8).gap(10).items_center().rounded(8).cursor_pointer().relative()
      .text_size(14).text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
      .when(collapsed, (el) => el.justify_center().px(0))
      .tooltip(tip)
      .on_click(action)
      .child(svg(icon).size(16).flex_none())
      .when(!collapsed, (el) => el.child(label));
  }

  // The sidebar's rows (the nav block): Search opens the palette; Tasks opens the Board and says what it holds.
  nav(t, collapsed) {
    const { review, busy } = this;
    const tasks = this.navRow(t, collapsed, "sidebar-tasks", "mods/workspace-chrome/list-checks.svg", "Tasks", this.tasksTip(),
      () => openOverlay({ mod: "board", kind: "popup", width: 420, height: 560 }))
      .when(!collapsed, (el) => el.child(div().flex_1()))
      .when(!collapsed && review > 0, (el) => el.child(div().id("sidebar-tasks-review").h(18).min_w(18).px(5).rounded(9).flex().items_center().justify_center()
        .bg(t.accent_fill).text_color(t.accent_foreground).text_size(11).font_semibold().child(review > 99 ? "99+" : String(review))))
      .when(!collapsed && review <= 0 && busy > 0, (el) => el.child(div().id("sidebar-tasks-count").text_size(12).text_color(t.dim).child(String(busy))))
      .when(collapsed && review > 0, (el) => el.child(div().size(6).rounded_full().bg(t.accent).absolute().top(8).right(8)));
    return v_flex().size_full().px(8).py(4)
      .child(this.navRow(t, collapsed, "sidebar-search", "mods/workspace-chrome/search.svg", "Search", "Search", () => run("app.palette")))
      .child(tasks);
  }

  // The sidebar's foot: Settings and the bell. The bell counts what needs you (the lead's question, blocked
  // agents, ready updates) plus the notes waiting for the Owner; tasks don't count. The rail keeps only the bell.
  foot(t, collapsed) {
    const iconButton = (id, icon, tip, action) => h_flex().id(id).w(28).h(28).flex_none().items_center().justify_center().rounded(7)
      .cursor_pointer().text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip(tip)
      .on_click(action).child(svg(icon).size(16));
    const u = updates();
    const needs = agents().filter((a) => a.status === "blocked").length + (lead()?.ask ? 1 : 0)
      + (u.app.ready ? 1 : 0) + (u.core.ready && !u.core.updating ? 1 : 0);
    const count = needs + this.notes;
    const parts = [needs > 1 ? `${needs} need you` : needs === 1 ? "1 needs you" : "",
      this.notes > 1 ? `${this.notes} notes` : this.notes === 1 ? "1 note" : ""];
    const bellTip = count === 0 ? "Nothing needs you" : parts.filter(Boolean).join(" · ");
    const bell = iconButton("sidebar-bell", "mods/topbar/bell.svg", bellTip,
      () => openOverlay({ mod: "bell", kind: "popup", width: 360, height: 440, at: "pointer" }))
      .relative()
      .when(count > 0, (el) => el.child(div().id("sidebar-bell-count").absolute().top(-2).right(-2).h(15).min_w(15).px(4).rounded(8).flex().items_center().justify_center()
        .bg(t.accent_fill).text_color(t.accent_foreground).text_size(10.5).font_semibold().child(count > 99 ? "99+" : String(count))));
    if (collapsed) return h_flex().size_full().px(0).justify_center().items_center().child(bell);
    const settingsButton = iconButton("settings", "mods/workspace-chrome/settings.svg", "Settings", () => run("app.settings"))
      .when(Boolean(u.app.ready), (el) => el.relative().child(div().size(6).rounded_full().bg(t.accent).absolute().top(4).right(4)));
    return h_flex().size_full().px(10).gap(2).items_center()
      .child(settingsButton)
      .child(div().flex_1())
      .child(bell);
  }

  render() {
    const t = theme();
    const collapsed = narrow();
    if (props()?.nav) return this.nav(t, collapsed);
    if (props()?.footer) return this.foot(t, collapsed);
    return h_flex().size_full().pl(collapsed ? 6 : 14).pr(10).gap(4).items_center().text_size(14)
      .when(window()?.platform === "macos" && !window()?.fullscreen && !collapsed, el => el.pl(76))
      .when(!collapsed, el => el.child(h_flex().flex_1().gap(8).items_center().child(image("frame/welcome/kompitum.png").size(20).rounded(5)).child(div().font_semibold().text_color(t.foreground).child("Kompitum"))))
      .when(!collapsed, el => el.child(this.spaceMenu(t)))
      // On the macOS rail the traffic lights fill the row; the agents list's » unfolds it.
      .when(!(collapsed && window()?.platform === "macos"), el => el.child(this.button("sidebar-toggle", svg("mods/workspace-chrome/sidebar.svg").size(15), toggleSidebar)));
  }
}
