// The Desk's top bar: a breadcrumb of where you are (the shown tab's folder, the tab, its branch and
// machine), then Files and Changes, and the window's buttons. The bell and the mailbox moved to the
// sidebar's foot (workspace-chrome). It replaces the tabs mod in Desk; the tabs mod stays for other layouts.

import { View, div, svg, image } from "gpui-kit";
import { h_flex } from "gpui-base";
import { spaces, panes, run, theme, window, shared, layout, settings, home } from "kompitum";

function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

// The folder as the pane header shows it: the home folder as "~", on Windows too.
function folder(cwd) {
  if (!cwd) return "";
  const h = home();
  if (!h || !cwd.startsWith(h)) return cwd;
  const rest = cwd.slice(h.length);
  return rest === "" || rest[0] === "/" || rest[0] === "\\" ? "~" + rest : cwd;
}

export default class TopBar extends View {
  init() {
    this.panels = JSON.parse(localStorage.getItem("panels") ?? "{}");
  }

  // The side panel is remembered per space: switching shows that space's choice
  // (closed if it never had one), and opening or closing it records it.
  syncPanel(space, panel, body, enabled) {
    if (!space || !panel) return;
    const now = panel.hidden ? null : body?.active ?? null;
    if (space.id !== this.space) {
      const first = this.space === undefined;
      this.space = space.id; this.pending = null;
      if (!(first && !(space.id in this.panels))) {
        const stored = this.panels[space.id] ?? null, want = enabled.includes(stored) ? stored : null;
        if (want !== now) {
          this.pending = { want, until: Date.now() + 1000 };
          if (want) { run("layout.select", { stack: "panel-body", child: want }); run("layout.show", { id: "panel" }); }
          else run("layout.hide", { id: "panel" });
          return;
        }
      }
    }
    // Until the restore lands (or something else won), the old state isn't a choice.
    if (this.pending) {
      if (this.pending.want !== now && Date.now() < this.pending.until) return;
      this.pending = null;
    }
    if (this.panels[space.id] === now) return;
    this.panels[space.id] = now;
    localStorage.setItem("panels", JSON.stringify(this.panels));
  }

  render() {
    const t = theme();
    const root = layout(), sidebar = find(root, "sidebar"), w = window();
    // Old saved rails and a dragged-narrow sidebar can be smaller than the window buttons.
    const lightsPadding = w?.platform === "macos" && w.titlebar === "none" && !w.fullscreen && root?.traffic_lights
      ? Math.max(10, (root.traffic_lights[0] + 64) / settings().zoom - (sidebar?.hidden ? 0 : sidebar?.size?.px ?? 0))
      : 10;
    const space = spaces().find((p) => p.active);
    const shown = space?.tabs?.find((c) => c.active);
    const program = shown
      ? panes().find((p) => p.tab === shown.id && p.program?.agent)?.program ?? panes().find((p) => p.tab === shown.id)?.program
      : null;
    const name = shown ? (/^tb\d+$/.test(shown.name) ? program?.name ?? "Terminal" : shown.name) : "";
    // The shown tab's folder: its focused pane's, else its first pane's that has one.
    const tabPanes = shown ? panes().filter((p) => p.tab === shown.id) : [];
    const dir = folder((tabPanes.find((p) => p.focused && p.cwd) ?? tabPanes.find((p) => p.cwd))?.cwd);
    const dirText = dir.length > 48 ? `…${dir.slice(-47)}` : dir;
    const panel = find(root, "panel"), body = find(root, "panel-body");
    const enabled = shared("workspace.panels") ?? ["files", "changes"];
    this.syncPanel(space, panel, body, enabled);
    const stop = (_e, cx) => cx.stop_propagation();
    const button = (id, label, action) => div().id(id).size(26).flex_none().flex().items_center().justify_center().rounded(6).text_color(t.dim).hover(s => s.bg(t.hover).text_color(t.foreground))
      .on_mouse_down("left", stop).on_click(action).child(label);
    const panelButton = (id, label, icon) => {
      const on = panel && !panel.hidden && body?.active === id;
      return h_flex().id(`panel-${id}`).h(28).px(8).gap(6).flex_none().items_center().rounded(7).cursor_pointer()
        .text_size(12.5).bg(on ? t.selected : t.chrome).text_color(on ? t.foreground : t.muted)
        .hover((s) => s.bg(t.hover).text_color(t.foreground)).on_mouse_down("left", stop)
        .on_click(() => {
          if (on) run("layout.hide", { id: "panel" });
          else { run("layout.select", { stack: "panel-body", child: id }); run("layout.show", { id: "panel" }); }
        }).child(svg(icon).size(14)).child(label);
    };
    const machine = space?.machine, branch = space?.branch;
    const crumbs = h_flex().id("topbar-crumbs").min_w(0).gap(6).items_center()
      .when(Boolean(dirText), (el) => el.child(div().id("topbar-folder").min_w(0).truncate().text_size(13).text_color(t.dim).tooltip(dir).child(dirText)))
      .when(Boolean(dirText) && Boolean(shown), (el) => el.child(div().text_size(13).text_color(t.dim).child("/")))
      .when(Boolean(shown), (el) => el.child(h_flex().id("topbar-tab").min_w(0).gap(7).items_center()
        .when(Boolean(program?.icon), (el) => el.child(program.tint ? svg(program.icon).size(14).text_color(t.muted) : image(program.icon).size(14)))
        .child(div().min_w(0).truncate().text_size(13).font_medium().text_color(t.foreground).child(name))))
      .when(Boolean(branch), (el) => el.child(h_flex().id("topbar-branch").gap(5).flex_none()
        .child(svg("mods/workspace-chrome/branch.svg").size(13))
        .child(div().text_size(12).text_color(t.dim).child(branch))))
      .when(Boolean(machine), (el) => el.child(h_flex().id("topbar-machine").gap(5).flex_none()
        .child(svg("mods/workspace-chrome/server.svg").size(13))
        .child(div().text_size(12).text_color(t.dim).child(machine))));
    return h_flex().size_full().min_w(0).px(10).pl(lightsPadding).gap(4).items_center().bg(t.chrome)
      .child(crumbs)
      .child(div().flex_1())
      .when(enabled.includes("files"), el => el.child(panelButton("files", "Files", "mods/files/file.svg")))
      .when(enabled.includes("changes"), el => el.child(panelButton("changes", "Changes", "mods/changes/changes.svg")))
      .when(w?.buttons, el => el.child(button("minimize", w.platform === "windows" ? svg("mods/tabs/win-min.svg").size(16) : "−", () => run("window.minimize")))
        .child(button("maximize", w.platform === "windows" ? svg(`mods/tabs/${w.maximized ? "win-restore" : "win-max"}.svg`).size(16) : "□", () => run("window.maximize")))
        .child(button("close-window", w.platform === "windows" ? svg("mods/tabs/win-close.svg").size(16) : svg("mods/pane/close.svg").size(13), () => run("window.close"))));
  }
}
