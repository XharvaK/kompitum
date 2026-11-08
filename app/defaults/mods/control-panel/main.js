import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { agents, terminals, spaces, panes, props, run, theme, window as appWindow,
  shared, setShared, setAgentOrder, lc, layout, openOverlay, closeOverlay } from "limen";
import frames from "./frames.js";
import { CELL, FONT, ROW, HISTORY_LIMIT, READ_INTERVAL, EXIT_INTERVAL, STATUS, plain, lastLine,
  truncate, elapsed, collectRows, sortRows, remember, missingRows, confirmExits, gridSpec,
  panelSlices, readableRows, nodeVisible } from "./model.js";

const liveRows = () => collectRows(agents(), terminals(), spaces());
const windowState = () => appWindow() ?? { active: false };
const motion = () => shared("control-panel.still") !== true && windowState().active === true;
const active = () => windowState().active === true && nodeVisible(layout(), "control-panel-overview");
const color = status => status === "blocked" ? theme().blocked :
  status === "working" || status === "done" ? theme().accent : status === "idle" ? theme().dim : theme().muted;
const text = (value, tint = theme().foreground) => div().text_size(FONT).text_color(tint).child(String(value));
const base = () => v_flex().text_size(FONT).font_family("JetBrains Mono").text_color(theme().foreground);
const identity = a => `${a.title}\n${a.space_name} / ${a.tab_name}\n${a.address}${a.child ? " (child, opens peek)" : ""}`;
const focus = a => {
  if (["dead", "unavailable"].includes(a.status)) return;
  run(a.child ? "terminal.peek" : "agent.focus", { terminal: a.address });
};
function button(id, label, click, enabled = true) {
  return div().id(id).h(28).px(8).flex_none().flex().items_center().text_size(FONT)
    .text_color(enabled ? theme().muted : theme().dim)
    .when(enabled, el => el.cursor_pointer().hover(s => s.bg(theme().hover).text_color(theme().foreground)))
    .on_mouse_down("left", (_event, cx) => cx.stop_propagation())
    .on_click((_event, cx) => { cx.stop_propagation(); if (enabled) click(cx); }).child(label);
}
function glyph(a) {
  const spec = frames[a.status] ?? frames.unavailable;
  const cell = text(spec.still, color(a.status)).id(`glyph-${a.address}`)
    .w(CELL).min_w(CELL).max_w(CELL).flex_none().overflow_hidden();
  if (motion() && spec.ms) cell.frames(spec.frames, spec.ms);
  return cell;
}
function readHistory() {
  try {
    const saved = JSON.parse(localStorage.getItem("control-panel.history") ?? "[]");
    return Array.isArray(saved) ? saved.filter(a => a && typeof a.address === "string").slice(-HISTORY_LIMIT) : [];
  } catch (_) { return []; }
}

export default class ControlPanel extends View {
  init(_props, cx) {
    this.role = props()?.role ?? "overview";
    this.size = { width: 1000, height: 432 };
    this.measured = false;
    this.starts = { blocked: 0, rest: 0 };
    this.wheel = { blocked: 0, rest: 0 };
    this.history = this.role === "overview" ? readHistory() : [];
    this.liveKey = null;
    this.cache = new Map();
    this.pending = false;
    this.exitPending = false;
    this.exitLabel = "";
    this.closed = false;
    if (this.role === "overview") {
      cx.timer.every(1000, cx => { if (active()) cx.notify(); });
      cx.timer.every(READ_INTERVAL, cx => this.refreshLines(cx));
      cx.timer.every(EXIT_INTERVAL, cx => this.refreshExits(cx));
    }
  }
  close() { this.closed = true; }
  rows() {
    const allSpaces = spaces(), live = collectRows(agents(), terminals(), allSpaces);
    const key = JSON.stringify([live.map(a => [a.address, a.title, a.space, a.space_name, a.tab, a.tab_name,
      a.child, a.status, a.rawDone, a.status_ms]),
      allSpaces.map(s => [s.id, s.tabs.map(t => [t.id, t.changed_at, t.working_since])])]);
    if (key !== this.liveKey) {
      this.liveKey = key;
      this.history = remember(this.history, live, Date.now(), allSpaces);
      localStorage.setItem("control-panel.history", JSON.stringify(this.history));
    }
    const clocks = new Map(this.history.map(a => [a.address, a.state_started_ms]));
    return sortRows([...live.map(a => ({ ...a, state_started_ms: clocks.get(a.address) })),
      ...missingRows(this.history, live, terminals())], allSpaces);
  }
  visibleRows() {
    return readableRows(panelSlices(this.rows(), this.size.height, this.starts));
  }
  refreshLines(cx) {
    if (this.closed || this.pending || !this.measured || !active()) return;
    const rows = this.visibleRows();
    const visible = new Set(rows.map(a => a.address));
    // Output stays in memory, and only for rows in the current viewport.
    for (const address of this.cache.keys()) if (!visible.has(address)) this.cache.delete(address);
    this.pending = true;
    cx.spawn(async cx => {
      try {
        for (const row of rows) {
          if (this.closed || !active()) break;
          if (!this.visibleRows().some(a => a.address === row.address)) continue;
          const previous = this.cache.get(row.address);
          if (previous && Date.now() - previous.at < READ_INTERVAL) continue;
          const at = Date.now();
          this.cache.set(row.address, { at, text: previous?.text ?? "", error: false });
          try {
            const reply = await lc(["read", row.address, "--lines", "8"]);
            if (reply.code !== 0) throw new Error("Read unavailable");
            if (this.closed || !active() || !this.visibleRows().some(a => a.address === row.address)) {
              this.cache.delete(row.address); continue;
            }
            this.cache.set(row.address, { at, text: lastLine(reply.stdout), error: false });
          } catch (_) {
            if (!this.closed && active() && this.visibleRows().some(a => a.address === row.address)) {
              this.cache.set(row.address, { at, text: "read unavailable", error: true });
            } else this.cache.delete(row.address);
          }
        }
      } finally {
        this.pending = false;
        if (!this.closed && active()) cx.notify();
      }
    });
  }
  refreshExits(cx) {
    if (this.closed || this.exitPending || !active()) return;
    if (!missingRows(this.history, liveRows(), terminals()).some(a => a.status === "unavailable")) return;
    this.exitPending = true;
    cx.spawn(async cx => {
      try {
        const reply = await lc(["ls", "--all", "--json"]);
        if (reply.code !== 0) throw new Error("Exit check unavailable");
        if (this.closed) return;
        this.history = confirmExits(this.history, JSON.parse(reply.stdout), new Set(terminals().map(t => t.address)));
        localStorage.setItem("control-panel.history", JSON.stringify(this.history));
        this.exitLabel = "";
      } catch (_) { this.exitLabel = "exit check unavailable"; }
      finally { this.exitPending = false; if (!this.closed && active()) cx.notify(); }
    });
  }
  topbar() {
    const rows = liveRows(), blocked = rows.filter(a => a.status === "blocked");
    return h_flex().id("control-panel-bar").size_full().pl(windowState().platform === "macos" ? 88 : 24).pr(16)
      .gap(24).items_center().bg(theme().chrome).text_size(FONT).font_family("JetBrains Mono")
      .child(text("CONTROL PANEL").font_semibold())
      .child(text(`${String(rows.length).padStart(2, "0")} agents / ${String(spaces().length).padStart(2, "0")} spaces`, theme().muted))
      .child(div().flex_1())
      .child(text(blocked.length ? `${String(blocked.length).padStart(2, "0")} need you` : "all clear", blocked.length ? theme().blocked : theme().muted))
      .child(button("control-panel-still", shared("control-panel.still") === true ? "Motion off" : "Motion on",
        () => setShared("control-panel.still", shared("control-panel.still") !== true)))
      .child(button("control-panel-command", "Command", () => run("app.palette")))
      .when(windowState().buttons, el => el.children([
        button("control-panel-min", "Min", () => run("window.minimize")),
        button("control-panel-max", "Max", () => run("window.maximize")),
        button("control-panel-close", "Close", () => run("window.close"))]));
  }
  row(a, grid) {
    const c = grid.columns, cached = this.cache.get(a.address);
    const state = a.rawDone ? "finished" : STATUS[a.status];
    let output = cached?.text ?? "awaiting read";
    if (a.status === "dead") output = `${a.exit?.reason ?? "exited"}${a.exit?.code != null ? ` (code ${a.exit.code})` : ""}`;
    if (a.status === "unavailable") output = "exit not confirmed";
    const detail = `${identity(a)}\n${state}${a.rawDone ? " (child report, not unseen done)" : ""}\n${output}`;
    const row = h_flex().id(`control-panel-row-${a.address}`).w(grid.width).h(ROW).flex_none()
      .px(grid.padding).gap(grid.gap).items_center().text_size(FONT)
      .bg(a.focused ? theme().selected : theme().canvas).tooltip(detail)
      .hover(s => s.bg(theme().hover)).on_click(() => focus(a));
    if (!["dead", "unavailable"].includes(a.status)) row.cursor_pointer();
    return row
      .child(h_flex().id(`state-cell-${a.address}`).w(c[0] * CELL).min_w(c[0] * CELL).max_w(c[0] * CELL)
        .flex_none().overflow_hidden().gap(CELL).items_center().child(glyph(a)).child(text(state, color(a.status))))
      .child(text(truncate(a.address, c[1]), a.focused ? theme().accent : theme().muted).w(c[1] * CELL).flex_none())
      .child(text(truncate(a.title, c[2]), a.status === "dead" ? theme().muted : theme().foreground).w(c[2] * CELL).flex_none())
      .child(text(truncate(`${a.space_name} / ${a.tab_name}`, c[3]), theme().muted).w(c[3] * CELL).flex_none())
      .child(text(elapsed(["dead", "unavailable"].includes(a.status) ? null : a.state_started_ms ?? a.status_ms, Date.now()), theme().muted).w(c[4] * CELL).flex_none().text_right())
      .child(text(truncate(output || "(empty screen)", c[5]), cached?.error ? theme().muted : theme().dim).w(c[5] * CELL).flex_none());
  }
  labels(grid) {
    return h_flex().h(28).w(grid.width).px(grid.padding).gap(grid.gap).items_center().flex_none()
      .children(["STATE", "ADDRESS", "TASK", "SPACE / TAB", "ELAPSED", "LAST LINE"].map((value, i) =>
        text(value, theme().dim).w(grid.columns[i] * CELL).flex_none().when(i === 4, el => el.text_right())));
  }
  scroll(kind, direction, slice, cx) {
    this.starts[kind] = Math.max(0, Math.min(slice.offset + direction, Math.max(0, slice.total - slice.capacity)));
    cx.notify();
  }
  section(kind, slice, grid) {
    const blocked = kind === "blocked";
    const range = slice.total ? `${slice.offset + 1}:${slice.offset + slice.rows.length} / ${slice.total}` : "0";
    const area = base().id(`control-panel-${kind}`).flex_none()
      .child(h_flex().h(32).flex_none().px(24).items_center().gap(16)
        .child(text(blocked ? "NEEDS YOU" : "AGENTS", blocked && slice.total ? theme().blocked : theme().muted).font_semibold())
        .child(text(blocked ? (slice.total ? "pinned across every space" : "no agents waiting") : "all spaces, all tabs", theme().dim))
        .child(div().flex_1())
        .child(text(range, theme().dim))
        .when(slice.total > slice.capacity, el => el
          .child(button(`${kind}-prev`, "Prev", cx => this.scroll(kind, -slice.capacity, slice, cx), slice.offset > 0))
          .child(button(`${kind}-next`, "Next", cx => this.scroll(kind, slice.capacity, slice, cx), slice.offset + slice.capacity < slice.total))));
    const list = base().id(`control-panel-${kind}-rows`).h(slice.capacity * ROW).flex_none().overflow_hidden()
      .on_scroll_wheel((event, cx) => {
        cx.stop_propagation();
        this.wheel[kind] += event.delta_lines ? event.delta_lines.y * ROW : event.delta.y;
        const rows = Math.trunc(this.wheel[kind] / ROW);
        if (rows) { this.wheel[kind] -= rows * ROW; this.scroll(kind, -rows, slice, cx); }
      }).children(slice.rows.map(a => this.row(a, grid)));
    if (!slice.rows.length) list.child(text(blocked ? "Clear. Nothing needs your input." : "No agents reporting a state.", theme().dim).px(24).h(ROW));
    return area.child(list);
  }
  overview() {
    const rows = this.rows(), grid = gridSpec(this.size.width);
    const slices = panelSlices(rows, this.size.height, this.starts);
    this.starts = { blocked: slices.blocked.offset, rest: slices.rest.offset };
    setAgentOrder([...slices.blocked.rows, ...rows.filter(a => a.status === "blocked" && !slices.blocked.rows.includes(a)),
      ...rows.filter(a => !["blocked", "dead", "unavailable"].includes(a.status))].map(a => a.address));
    const info = windowState().active !== true ? "reads paused" : this.exitLabel || "screen reads every 5s";
    return base().id("control-panel-overview-root").size_full().min_h(0).bg(theme().canvas).overflow_hidden()
      .on_size_change((size, cx) => {
        if (!this.measured || size.width !== this.size.width || size.height !== this.size.height) {
          this.size = size; this.measured = size.width > 0 && size.height > 0; cx.notify();
        }
      })
      .child(this.labels(grid))
      .child(this.section("blocked", slices.blocked, grid))
      .child(this.section("rest", slices.rest, grid))
      .child(div().flex_1())
      .child(h_flex().h(28).px(24).items_center().justify_between().flex_none()
        .child(text("Select a row to focus. Scroll each list to browse.", theme().dim))
        .child(text(info, theme().dim)));
  }
  livebar() {
    const space = spaces().find(s => s.active), tab = space?.tabs.find(t => t.active);
    return h_flex().size_full().px(24).items_center().gap(24).bg(theme().chrome).text_size(FONT).font_family("JetBrains Mono")
      .child(text("TERMINAL", theme().muted))
      .child(text(space && tab ? `${plain(space.name)} / ${plain(tab.name)}` : "No active tab", theme().foreground).truncate().flex_1().min_w(0))
      .child(button("control-panel-tabs", "Browse tabs", () => openOverlay({ mod: "control-panel", props: { role: "tabs" }, kind: "popup", width: 480, height: 360 })));
  }
  pane() {
    const p = panes().find(p => p.pane === props()?.pane);
    return h_flex().size_full().px(16).gap(16).items_center().bg(theme().chrome).text_size(FONT).font_family("JetBrains Mono")
      .on_click(() => { if (p?.pane) run("pane.focus", { pane: p.pane }); })
      .child(text(p?.terminal ?? "pane", p?.focused ? theme().accent : theme().muted))
      .child(text(plain(p?.title) || "Terminal", theme().muted).truncate().flex_1().min_w(0))
      .child(text(p?.status ? STATUS[p.status] ?? p.status : "shell", p?.status ? color(p.status) : theme().dim));
  }
  tabs() {
    const all = spaces();
    return base().size_full().bg(theme().popover).p(16).gap(8)
      .child(text("SPACES / TABS", theme().muted).h(28).flex_none())
      .child(base().id("control-panel-tab-list").flex_1().min_h(0).overflow_y_scrollbar()
        .children(all.flatMap(space => (space.tabs ?? []).map(tab =>
          button(`tab-${tab.id}`, `${plain(space.name)} / ${plain(tab.name)}${tab.settled ? " (settled)" : ""}`, () => {
            closeOverlay(props().overlay); run("space.select", { id: space.id }); run("tab.select", { id: tab.id });
          }))))
        .when(!all.length, el => el.child(text("No spaces yet. Use Command to start.", theme().muted))));
  }
  render() {
    if (this.role === "topbar") return this.topbar();
    if (this.role === "livebar") return this.livebar();
    if (this.role === "pane") return this.pane();
    if (this.role === "tabs") return this.tabs();
    return this.overview();
  }
}
