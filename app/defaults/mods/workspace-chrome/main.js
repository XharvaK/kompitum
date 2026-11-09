import { View, div, image, svg } from "gpui-kit";
import { h_flex, v_flex, Popover } from "gpui-base";
import { theme, run, props, window, shared, setShared, updates, layout, spaces, knownAgents, pickFolder, notify } from "kompitum";

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

export default class WorkspaceChrome extends View {
  init() {
    this.spaceOpen = false;
    this.agents = [];
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
  render() {
    const t = theme();
    const collapsed = narrow();
    if (props()?.footer) {
      const row = (id, icon, label, action) => h_flex().id(id).h(22).px(8).gap(8).items_center().rounded(6)
        .when(collapsed, el => el.relative().w(32).px(0).justify_center().tooltip(label))
        .text_color(t.muted).hover(s => s.bg(t.hover).text_color(t.foreground)).cursor_pointer()
        .on_click(action).child(svg(`mods/workspace-chrome/${icon}.svg`).size(14))
        .when(!collapsed, el => el.child(label));
      const ready = div().size(6).rounded_full().bg(t.accent);
      // One row, centred in a footer as tall as the status bar beside it.
      return h_flex().size_full().bg(t.chrome).border_t(1).border_color(t.border).px(6).items_center().text_size(12)
        .when(collapsed, el => el.px(0).justify_center())
        .child(row("settings", "settings", "Settings", () => run("app.settings"))
          .when(Boolean(updates().app.ready), el => el.child(collapsed ? ready.absolute().top(5).right(6) : ready)));
    }
    return h_flex().size_full().bg(t.chrome).pl(collapsed ? 6 : 14).pr(10).gap(4).items_center().text_size(13.5)
      .when(window()?.platform === "macos" && !window()?.fullscreen && !collapsed, el => el.pl(76))
      .when(!collapsed, el => el.child(h_flex().flex_1().gap(8).items_center().child(image("frame/welcome/kompitum.png").size(20).rounded(5)).child(div().font_semibold().text_color(t.foreground).child("Kompitum"))))
      .when(!collapsed, el => el.child(this.button("search", svg("mods/workspace-chrome/search.svg").size(15), () => run("mod.open", { name: "picker" }))))
      .when(!collapsed, el => el.child(this.spaceMenu(t)))
      // On the macOS rail the traffic lights fill the row; the agents list's » unfolds it.
      .when(!(collapsed && window()?.platform === "macos"), el => el.child(this.button("sidebar-toggle", svg("mods/workspace-chrome/sidebar.svg").size(15), toggleSidebar)));
  }
}
