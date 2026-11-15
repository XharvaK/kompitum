// The Desk's top bar: a breadcrumb of where you are (the shown tab's folder, the tab, its branch and
// machine); on the right, Limits and the bell, then the side panel's toggle and the window's buttons. The bell
// opens its list in the corner where toasts show, under it. It replaces the tabs mod in Desk; the tabs mod stays
// for other layouts.
// The same mod, with `panel: true`, is the side panel's header: its picker of Files, Changes and Tasks.

import { View, div, svg, image } from "gpui-kit";
import { h_flex } from "gpui-base";
import { spaces, panes, run, theme, window, shared, layout, settings, home, props, openOverlay, agents, lead, updates, kom, text } from "kompitum";

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

// Claude's windows that count, as the composer counts them: a percentage, and a reset not yet past.
function claudeWindows(usage) {
  const now = Date.now() / 1000;
  return [{ short: text("topbar.window-5h"), limit: usage?.five_hour }, { short: text("topbar.window-week"), limit: usage?.seven_day }]
    .map(({ short, limit }) => ({ short, pct: limit?.used_percentage, resets: limit?.resets_at }))
    .filter((w) => typeof w.pct === "number" && Number.isFinite(w.pct))
    .filter((w) => !(typeof w.resets === "number" && w.resets < now));
}

export default class TopBar extends View {
  init(_props, cx) {
    this.panels = JSON.parse(localStorage.getItem("panels") ?? "{}");
    // The last surface shown per space, kept after the panel closes, so the toggle reopens it.
    this.last = JSON.parse(localStorage.getItem("panel-last") ?? "{}");
    // Only the bar reads the notes, for the bell; the panel's header copy has no bell.
    this.notes = 0;
    if (props()?.panel) return;
    this.readNotes(cx);
    cx.timer.every(60000, (cx) => this.readNotes(cx));
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

  // Limits, then the bell. Limits names Claude's windows in its tooltip, and a dot shows when the higher one is at
  // 75 or more. The bell counts what needs you (the lead's question, blocked agents, ready updates) plus the
  // notes waiting; tasks don't count. Both open in the corner where toasts show.
  status(t, button) {
    const windows = claudeWindows(lead()?.usage);
    const top = Math.max(0, ...windows.map((w) => w.pct));
    const limitsTip = [text("topbar.limits"), ...windows.map((w) => text("topbar.limit", { window: w.short, percent: Math.round(w.pct) }))].join(" · ");
    const limits = button("topbar-limits", svg("mods/workspace-chrome/gauge.svg").size(16),
      () => openOverlay({ mod: "limits", kind: "popup", width: 340, height: 480, at: "top-right" }))
      .text_color(t.muted)
      .tooltip(limitsTip)
      .accessibility_label(limitsTip)
      .when(top >= 75, (el) => el.relative().child(div().size(6).rounded_full().absolute().top(4).right(4)
        .bg(top >= 100 ? t.blocked : t.working)));
    const u = updates();
    const needs = agents().filter((a) => a.status === "blocked").length + (lead()?.ask ? 1 : 0)
      + (u.app.ready ? 1 : 0) + (u.core.ready && !u.core.updating ? 1 : 0);
    const count = needs + this.notes;
    const parts = [needs > 1 ? `${needs} need you` : needs === 1 ? "1 needs you" : "",
      this.notes > 1 ? `${this.notes} notes` : this.notes === 1 ? "1 note" : ""];
    const bell = button("topbar-bell", svg("mods/topbar/bell.svg").size(16),
      () => openOverlay({ mod: "bell", kind: "popup", width: 360, height: 440, at: "top-right" }))
      .relative()
      .text_color(t.muted)
      .tooltip(count === 0 ? text("topbar.nothing-needs-you") : parts.filter(Boolean).join(" · "))
      .accessibility_label(count === 0 ? text("topbar.nothing-needs-you") : parts.filter(Boolean).join(" · "))
      .when(count > 0, (el) => el.child(div().id("topbar-bell-count").absolute().top(-2).right(-2).h(16).min_w(16).px(4).rounded(8)
        .flex().items_center().justify_center().bg(t.accent_fill).text_color(t.accent_foreground).text_size(11).font_semibold()
        .child(count > 99 ? "99+" : String(count))));
    return [limits, bell];
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
      return h_flex().id(`panel-pick-${id}`).h(26).px(10).items_center().rounded(7).cursor_pointer().text_size(12)
        .keep_focus(true)
        .when(on, (el) => el.bg(t.selected).text_color(t.foreground).font_medium())
        .when(!on, (el) => el.text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected)))
        .on_click(() => run("layout.select", { stack: "panel-body", child: id }))
        .child(label);
    };
    const iconButton = (id, icon, size, tip, action) => div().id(id).size(28).flex_none().flex().items_center().justify_center()
      .rounded(6).text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected)).tooltip(tip)
      .accessibility_label(tip).keep_focus(true)
      .on_click(action).child(svg(icon).size(size));
    const words = [["files", text("topbar.files")], ["changes", text("topbar.changes")], ["tasks", text("topbar.tasks")]]
      .filter(([id]) => enabled.includes(id))
      .map(([id, label]) => word(id, label));
    return h_flex().id("panel-head").size_full().px(8).gap(2).items_center().border_b(1).border_color(t.border)
      .children(words)
      .child(div().flex_1())
      .child(iconButton("panel-terminal", "mods/workspace-chrome/terminal.svg", 15, text("topbar.open-terminal"),
        () => run("pane.split", { direction: "right" })))
      .child(iconButton("panel-close", "mods/workspace-chrome/x.svg", 12, text("topbar.close"), () => run("layout.hide", { id: "panel" })));
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
    const name = shown ? (/^tb\d+$/.test(shown.name) ? program?.name ?? text("topbar.terminal") : shown.name) : "";
    // The shown tab's folder: its focused pane's, else its first pane's that has one.
    const tabPanes = shown ? panes().filter((p) => p.tab === shown.id) : [];
    const dir = folder((tabPanes.find((p) => p.focused && p.cwd) ?? tabPanes.find((p) => p.cwd))?.cwd);
    const dirText = dir.length > 48 ? `…${dir.slice(-47)}` : dir;
    const panel = find(root, "panel"), body = find(root, "panel-body");
    const enabled = shared("workspace.panels") ?? ["files", "changes", "tasks"];
    this.syncPanel(space, panel, body, enabled);
    this.rememberSurface(space, panel, body);
    const stop = (_e, cx) => cx.stop_propagation();
    const button = (id, label, action, name) => div().id(id).size(28).flex_none().flex().items_center().justify_center().rounded(6).text_color(t.dim).hover(s => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
      .keep_focus(true).when(Boolean(name), (el) => el.accessibility_label(name)).on_mouse_down("left", stop).on_click(action).child(label);
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
      .tooltip(open ? text("topbar.hide-panel") : text("topbar.show-panel"))
      .accessibility_label(open ? text("topbar.hide-panel") : text("topbar.show-panel"));
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
      .children(this.status(t, button))
      // Space, not a line, sets the bell apart from the panel's toggle and the window's buttons.
      .child(div().w(8).flex_none())
      .when(enabled.length > 0, el => el.child(toggle))
      .when(w?.buttons, el => el.child(button("minimize", w.platform === "windows" ? svg("mods/tabs/win-min.svg").size(16) : "−", () => run("window.minimize"), text("topbar.minimize")))
        .child(button("maximize", w.platform === "windows" ? svg(`mods/tabs/${w.maximized ? "win-restore" : "win-max"}.svg`).size(16) : "□", () => run("window.maximize"), w.maximized ? text("topbar.restore") : text("topbar.maximize")))
        .child(button("close-window", w.platform === "windows" ? svg("mods/tabs/win-close.svg").size(16) : svg("mods/pane/close.svg").size(13), () => run("window.close"), text("topbar.close-window"))
          // Windows draws its own close button red on hover, and so does ours there (UD-FAM-001).
          .when(w.platform === "windows", (el) => el.hover((s) => s.bg("#C42B1C").text_color("#FFFFFF")).active((s) => s.bg("#C42B1CE6").text_color("#FFFFFF")))));
  }
}
