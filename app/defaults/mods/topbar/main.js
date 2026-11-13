// The Desk's top bar: a breadcrumb of where you are (the shown tab's folder, the tab, its branch and
// machine), then the side panel's toggle, and the window's buttons. The bell and the mailbox moved to the
// sidebar's foot (workspace-chrome). It replaces the tabs mod in Desk; the tabs mod stays for other layouts.
// The same mod, with `panel: true`, is the side panel's header: its picker of Files, Changes and Tasks.

import { View, div, svg, image } from "gpui-kit";
import { h_flex } from "gpui-base";
import { spaces, panes, run, theme, window, shared, layout, settings, home, props } from "kompitum";

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
    // The last surface shown per space, kept after the panel closes, so the toggle reopens it.
    this.last = JSON.parse(localStorage.getItem("panel-last") ?? "{}");
  }

  // Records the surface on show, once the panel's state is settled (a restore still landing isn't a choice).
  rememberSurface(space, panel, body) {
    if (!space || this.pending || !panel || panel.hidden || !body?.active) return;
    if (this.last[space.id] === body.active) return;
    this.last[space.id] = body.active;
    localStorage.setItem("panel-last", JSON.stringify(this.last));
  }

  // The surface the toggle reopens: the space's last one while it is still enabled, else Files.
  reopenSurface(space, enabled) {
    const last = this.last[space?.id];
    if (enabled.includes(last)) return last;
    return enabled.includes("files") ? "files" : enabled[0];
  }

  // The side panel's header (the copy in the panel column): a word per enabled surface, the terminal
  // button and close. It doesn't run syncPanel; only the bar copy remembers the panel per space.
  head(t) {
    const body = find(layout(), "panel-body");
    const enabled = shared("workspace.panels") ?? ["files", "changes", "tasks"];
    const word = (id, label) => {
      const on = body?.active === id;
      return h_flex().id(`panel-pick-${id}`).h(26).px(10).items_center().rounded(7).cursor_pointer().text_size(12.5)
        .when(on, (el) => el.bg(t.selected).text_color(t.foreground).font_medium())
        .when(!on, (el) => el.text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)))
        .on_click(() => run("layout.select", { stack: "panel-body", child: id }))
        .child(label);
    };
    const iconButton = (id, icon, size, tip, action) => div().id(id).size(26).flex_none().flex().items_center().justify_center()
      .rounded(6).text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip(tip)
      .on_click(action).child(svg(icon).size(size));
    const words = [["files", "Files"], ["changes", "Changes"], ["tasks", "Tasks"]]
      .filter(([id]) => enabled.includes(id))
      .map(([id, label]) => word(id, label));
    return h_flex().id("panel-head").size_full().px(8).gap(2).items_center().border_b(1).border_color(t.border)
      .children(words)
      .child(div().flex_1())
      .child(iconButton("panel-terminal", "mods/workspace-chrome/terminal.svg", 15, "Open a terminal beside it",
        () => run("pane.split", { direction: "right" })))
      .child(iconButton("panel-close", "mods/workspace-chrome/x.svg", 12, "Close", () => run("layout.hide", { id: "panel" })));
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
    if (props()?.panel) return this.head(t);
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
    const enabled = shared("workspace.panels") ?? ["files", "changes", "tasks"];
    this.syncPanel(space, panel, body, enabled);
    this.rememberSurface(space, panel, body);
    const stop = (_e, cx) => cx.stop_propagation();
    const button = (id, label, action) => div().id(id).size(26).flex_none().flex().items_center().justify_center().rounded(6).text_color(t.dim).hover(s => s.bg(t.hover).text_color(t.foreground))
      .on_mouse_down("left", stop).on_click(action).child(label);
    const open = Boolean(panel && !panel.hidden);
    const togglePanel = () => {
      if (open) {
        run("layout.hide", { id: "panel" });
        return;
      }
      run("layout.select", { stack: "panel-body", child: this.reopenSurface(space, enabled) });
      run("layout.show", { id: "panel" });
    };
    const toggle = button("panel-toggle", svg("mods/topbar/panel-right.svg").size(16), togglePanel)
      .text_color(open ? t.foreground : t.muted)
      .when(open, (el) => el.bg(t.selected))
      .tooltip(open ? "Hide the side panel" : "Show the side panel");
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
      .when(enabled.length > 0, el => el.child(toggle))
      .when(w?.buttons, el => el.child(button("minimize", w.platform === "windows" ? svg("mods/tabs/win-min.svg").size(16) : "−", () => run("window.minimize")))
        .child(button("maximize", w.platform === "windows" ? svg(`mods/tabs/${w.maximized ? "win-restore" : "win-max"}.svg`).size(16) : "□", () => run("window.maximize")))
        .child(button("close-window", w.platform === "windows" ? svg("mods/tabs/win-close.svg").size(16) : svg("mods/pane/close.svg").size(13), () => run("window.close"))));
  }
}
