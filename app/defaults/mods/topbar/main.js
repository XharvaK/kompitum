// The Desk's top bar (audit/REWORK-2026-11-09.md, W3): Back to the lead, what is
// shown, then the Board's bell, the mailbox, Files and Changes, and the window's
// buttons. It replaces the tabs mod in Desk; the tabs mod stays for other layouts.

import { View, div, svg, image } from "gpui-kit";
import { h_flex } from "gpui-base";
import { spaces, panes, agents, terminals, lead, kom, run, openOverlay, theme, window, shared, layout, settings } from "kompitum";

function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

function firstLine(text) {
  return String(text ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";
}

export default class TopBar extends View {
  init(_props, cx) {
    this.panels = JSON.parse(localStorage.getItem("panels") ?? "{}");
    this.review = 0;
    this.readReview(cx);
    cx.timer.every(10000, (cx) => this.readReview(cx));
  }

  // The tasks waiting for review, from `kom status --json`. A failed read
  // keeps the last count and names the error on the bell.
  readReview(cx) {
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["status", "--json"]);
        if (r?.code !== 0) {
          this.reviewError = firstLine(r?.stderr || r?.stdout) || `exit ${r?.code ?? "?"}`;
          cx.notify();
          return;
        }
        const waiting = JSON.parse(r.stdout || "{}")?.waiting?.review;
        this.review = typeof waiting === "number" ? waiting : 0;
        this.reviewError = null;
      } catch (error) {
        this.reviewError = firstLine(error?.message ?? error) || "It didn't work.";
      }
      cx.notify();
    });
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

  badgeCount() {
    const blocked = agents().filter((a) => a.status === "blocked").length;
    const ask = lead()?.ask ? 1 : 0;
    const review = Number.isFinite(this.review) ? this.review : 0;
    return blocked + ask + review;
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
    const leadTerminal = terminals().find((x) => x.state === "live" && x.labels?.lead);
    const leadTab = leadTerminal ? panes().find((p) => p.terminal === leadTerminal.address)?.tab : null;
    const back = Boolean(leadTerminal && shown && leadTab !== shown.id);
    const panel = find(root, "panel"), body = find(root, "panel-body");
    const enabled = shared("workspace.panels") ?? ["files", "changes"];
    this.syncPanel(space, panel, body, enabled);
    const stop = (_e, cx) => cx.stop_propagation();
    const iconButton = (id, icon, tooltip, action) => h_flex().id(id).w(28).h(28).flex_none().items_center().justify_center().rounded(7)
      .cursor_pointer().text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip(tooltip)
      .on_mouse_down("left", stop).on_click(action).child(svg(icon).size(16));
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
    const count = this.badgeCount();
    const bellTip = this.reviewError
      ? `Board · couldn't read the tasks waiting for review: ${this.reviewError}`
      : "Board";
    const bell = iconButton("topbar-bell", "mods/topbar/bell.svg", bellTip, () => openOverlay({ mod: "board", kind: "popup", width: 420, height: 560 }))
      .relative()
      .when(count > 0, (el) => el.child(div().id("topbar-bell-count").absolute().top(-2).right(-2).h(15).min_w(15).px(4).rounded(8).flex().items_center().justify_center()
        .bg(t.accent_fill).text_color(t.accent_foreground).text_size(10.5).font_semibold().child(count > 99 ? "99+" : String(count))));
    return h_flex().size_full().min_w(0).px(10).pl(lightsPadding).gap(4).items_center().bg(t.chrome).border_b(1).border_color(t.border)
      .when(back, (el) => el.child(h_flex().id("topbar-back").h(28).px(8).gap(6).flex_none().items_center().rounded(7).cursor_pointer()
        .text_size(12.5).text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip("Back to the lead")
        .on_mouse_down("left", stop).on_click(() => run("lead.open"))
        .child(svg("mods/workspace-chrome/chevron-left.svg").size(14)).child("Back")))
      .when(Boolean(shown), (el) => el.child(h_flex().id("topbar-tab").min_w(0).gap(7).items_center()
        .when(Boolean(program?.icon), (el) => el.child(program.tint ? svg(program.icon).size(14).text_color(t.muted) : image(program.icon).size(14)))
        .child(div().min_w(0).truncate().text_size(13).text_color(t.foreground).child(name))))
      .child(div().flex_1())
      .child(bell)
      .child(iconButton("topbar-mail", "mods/topbar/inbox.svg", "Mailbox", () => run("app.mailbox")))
      .when(enabled.includes("files"), el => el.child(panelButton("files", "Files", "mods/files/file.svg")))
      .when(enabled.includes("changes"), el => el.child(panelButton("changes", "Changes", "mods/changes/changes.svg")))
      .when(w?.buttons, el => el.child(button("minimize", w.platform === "windows" ? svg("mods/tabs/win-min.svg").size(16) : "−", () => run("window.minimize")))
        .child(button("maximize", w.platform === "windows" ? svg(`mods/tabs/${w.maximized ? "win-restore" : "win-max"}.svg`).size(16) : "□", () => run("window.maximize")))
        .child(button("close-window", w.platform === "windows" ? svg("mods/tabs/win-close.svg").size(16) : svg("mods/pane/close.svg").size(13), () => run("window.close"))));
  }
}
