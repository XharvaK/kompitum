// The keybind list (Kompitum's prefix+?), grouped, filterable by typing.
// Reads keys.json through keys(); edit keys.json to change any of them.

import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { keys, props, closeOverlay, theme, shared } from "kompitum";

const GROUPS = [
  ["Spaces", /^space\./],
  ["Tabs", /^tab\./],
  ["Panes", /^pane\./],
  ["Agents", /^agent\./],
  ["Terminal", /^terminal\.|^send:/],
  ["Window", /./],
];

// On the desk layout the groups are the palette's (frame/palette/main.js): this
// table is its DESK, by action and title, and every other command falls under
// More. Keep it in step with that one.
const DESK_GROUPS = ["Lead", "Agents", "Terminals", "Window", "More"];
const DESK = [
  ["Lead", "lead.open", "Open the lead"],
  ["Agents", "agent.next", "Next agent"],
  ["Agents", "agent.prev", "Previous agent"],
  ["Terminals", "tab.new", "New terminal"],
  ["Terminals", "mod.open:picker", "Go to terminal…"],
  ["Terminals", "tab.rename", "Rename terminal"],
  ["Terminals", "tab.close", "Close terminal"],
  ["Terminals", "tab.next", "Next terminal"],
  ["Terminals", "tab.prev", "Previous terminal"],
  ["Window", "app.mailbox", "Mailbox"],
  ["Window", "app.settings", "Settings"],
  ["Window", "app.help", "Keys"],
  ["Window", "app.zoom_in", "Zoom in"],
  ["Window", "app.zoom_out", "Zoom out"],
  ["Window", "app.zoom_reset", "Reset zoom"],
];

// The shown layout (desk when none is shown yet, as settings reads it).
const onDesk = () => (shared("layout") ?? "desk") === "desk";

// The group a command's row sits in.
function groupOf(action) {
  if (onDesk()) return DESK.find(([, a]) => a === action)?.[0] ?? "More";
  return GROUPS.find(([, re]) => re.test(action))[0];
}

// The palette's names (frame/palette/main.js), so both screens call a command
// the same thing. Keep this table in step with that one.
const NAMED = [
  ["tab.new", undefined, "New tab"],
  ["mod.open", "picker", "Go to space or tab…"],
  ["tab.rename", undefined, "Rename tab"],
  ["tab.close", undefined, "Close tab"],
  ["tab.next", undefined, "Next tab"],
  ["tab.prev", undefined, "Previous tab"],
  ["pane.split", "right", "Split right"],
  ["pane.split", "down", "Split down"],
  ["pane.close", undefined, "Close pane"],
  ["pane.zoom", undefined, "Zoom pane"],
  ...["left", "right", "up", "down"].map((d) => ["pane.go", d, `Focus pane ${d}`]),
  ...["left", "right", "up", "down"].map((d) => ["pane.swap", d, `Swap pane ${d}`]),
  ["space.new", undefined, "New space"],
  ["space.rename", undefined, "Rename space"],
  ["space.close", undefined, "Close space"],
  ["space.next", undefined, "Next space"],
  ["space.prev", undefined, "Previous space"],
  ["layout.reset", undefined, "Reset layout"],
  ["layout.clone", undefined, "Duplicate layout"],
  ["mod.clone", undefined, "Duplicate mod"],
  ["terminal.open", undefined, "New terminal"],
  ["terminal.copy", undefined, "Copy"],
  ["terminal.paste", undefined, "Paste"],
  ["agent.next", undefined, "Next agent"],
  ["agent.prev", undefined, "Previous agent"],
  ["lead.open", undefined, "Open the lead"],
  ["window.minimize", undefined, "Minimize window"],
  ["window.maximize", undefined, "Maximize window"],
  ["window.close", undefined, "Close window"],
  ["app.settings", undefined, "Settings"],
  ["app.help", undefined, "Keys"],
  ["app.welcome", undefined, "Welcome"],
  ["app.whats_new", undefined, "What's new"],
  ["app.reset", undefined, "Reset to the default screen"],
  ["app.zoom_in", undefined, "Zoom in"],
  ["app.zoom_out", undefined, "Zoom out"],
  ["app.zoom_reset", undefined, "Reset zoom"],
  ["app.quit", undefined, "Quit Kompitum"],
];

// Commands the palette keeps out of its list, but the sheet still needs words for.
const TITLES = {
  "space.select": "Go to space",
  "tab.select": "Go to tab",
  "layout.toggle": "Show or hide the sidebar",
  "layout.switch": "Switch layout",
  "terminal.paste_image": "Paste an image",
  terminal: "Give the key to the terminal",
  "app.hide": "Hide Kompitum",
  "app.hide_others": "Hide others",
  "app.show_all": "Show all",
};

function titleOf(name) {
  const [noun, verb] = name.split(".");
  if (!verb) return noun;
  return `${verb.replace(/_/g, " ")} ${noun}`.replace(/^./, (c) => c.toUpperCase());
}

function titleFor(action) {
  if (action.startsWith("send:")) return "Type " + JSON.stringify(action.slice(5));
  const desk = DESK.find(([, a]) => a === action);
  if (desk && onDesk()) return desk[2];
  const [name, arg] = action.split(":");
  const exact = NAMED.find(([n, a]) => n === name && (a === undefined || a === arg));
  if (exact) return exact[2];
  if (TITLES[name]) return ["space.select", "tab.select"].includes(name) && arg ? `${TITLES[name]} ${arg}` : TITLES[name];
  return arg ? `${titleOf(name)} ${arg}` : titleOf(name);
}

const KEY_NAMES = {
  ctrl: "Ctrl",
  shift: "Shift",
  alt: "Alt",
  cmd: "Cmd",
  pagedown: "Page Down",
  pageup: "Page Up",
  backspace: "Backspace",
  insert: "Insert",
  enter: "Enter",
  tab: "Tab",
  escape: "Esc",
  left: "Left",
  right: "Right",
  up: "Up",
  down: "Down",
};

// "ctrl-shift-p" -> ["Ctrl", "Shift", "P"]; "ctrl--" -> ["Ctrl", "-"].
function keyParts(k) {
  const raw = k.endsWith("--") ? [...k.slice(0, -2).split("-"), "-"] : k.split("-");
  return raw.map((p) => KEY_NAMES[p] ?? (/^f\d+$/.test(p) ? p.toUpperCase() : p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)));
}

// Alt 1..9 and Ctrl 1..9 read as one row each: "Alt 1–9 · Go to space 1–9".
function numberedRow(list, base) {
  const rows = list
    .filter((k) => k.action.startsWith(base + ":") && /^\d+$/.test(k.action.slice(base.length + 1)))
    .map((k) => ({ ...k, n: Number(k.action.slice(base.length + 1)) }))
    .sort((a, b) => a.n - b.n);
  if (rows.length < 2) return null;
  if (rows[0].n !== 1 || !rows.every((row, i) => row.n === i + 1)) return null;
  const prefix = rows[0].keys.replace(/\d+$/, "");
  if (!rows.every((row) => row.keys.replace(/\d+$/, "") === prefix)) return null;
  const last = rows[rows.length - 1].n;
  return {
    base,
    keys: `${prefix}1\u2013${last}`,
    label: `${TITLES[base]} 1\u2013${last}`,
    section: rows[0].section,
  };
}

// One row per command, with all of its keys side by side, in keys() order.
// Numbered keys (space.select:1 ... tab.select:9) collapse into one row.
function rowsFor(list) {
  const ranges = [numberedRow(list, "space.select"), numberedRow(list, "tab.select")].filter(Boolean);
  const byBase = new Map(ranges.map((row) => [row.base, row]));
  const rows = new Map();
  for (const k of list) {
    const base = ["space.select", "tab.select"].find(
      (name) => k.action.startsWith(name + ":") && /^\d+$/.test(k.action.slice(name.length + 1)),
    );
    if (base && byBase.has(base)) {
      const range = byBase.get(base);
      if (!rows.has(base)) rows.set(base, { action: base, label: range.label, keys: [{ keys: range.keys, section: range.section }] });
      continue;
    }
    if (!rows.has(k.action)) rows.set(k.action, { action: k.action, label: titleFor(k.action), keys: [] });
    rows.get(k.action).keys.push({ keys: k.keys, section: k.section });
  }
  return [...rows.values()];
}

export default class Help extends View {
  init(_props, cx) {
    this.filter = "";
    this.input = InputState.new({ placeholder: "Type to filter…" });
    this.input.on("change", (_e, cx) => { this.filter = this.input.value(); cx.notify(); });
    this.input.focus();
    cx.timer.after(600, () => this.input.focus());
    this.handle = cx.focus_handle();
  }

  render(cx) {
    const t = theme();
    const f = this.filter.toLowerCase();
    const shown = rowsFor(keys()).filter((row) => {
      if (!f) return true;
      return `${row.keys.map((k) => `${k.keys} ${k.section}`).join(" ")} ${row.label}`.toLowerCase().includes(f);
    });
    const names = onDesk() ? DESK_GROUPS : GROUPS.map(([name]) => name);
    const groups = names
      .map((name) => [name, shown.filter((row) => groupOf(row.action) === name)])
      .filter(([, rows]) => rows.length > 0);
    const keycap = (p) =>
      div().px(5).rounded(4).border_1().border_color(t.border).bg(t.raised).text_size(11).text_color(t.dim).child(p);
    return v_flex()
      .id("help")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key === "escape") return closeOverlay(props().overlay);
      })
      .size_full()
      .min_h(0)
      .p(18)
      .gap(14)
      .bg(t.popover)
      .text_size(12)
      .child(
        h_flex()
          .h(22)
          .items_center()
          .justify_between()
          .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Keyboard shortcuts"))
          .child(div().text_size(11).text_color(t.dim).child("Esc")),
      )
      .child(Input.new(this.input).h(36).flex_none().px(10).rounded(9).border_1().border_color(t.accent).bg(t.background).text_size(12.8).text_color(t.foreground))
      .child(
        v_flex()
          .flex_1()
          .min_h(0)
          .overflow_y_scrollbar()
          .py(2)
          .gap(10)
          .children(
            groups.map(([name, rows]) =>
              v_flex()
                .gap(2)
                .child(div().text_size(11.5).font_medium().text_color(t.dim).child(name))
                .children(
                  rows.map((row) =>
                    h_flex()
                      .h(32)
                      .px(8)
                      .rounded(7)
                      .items_center()
                      .child(
                        h_flex()
                          .min_w(200)
                          .flex_none()
                          .gap(14)
                          .items_center()
                          .children(
                            row.keys.map((k) =>
                              h_flex()
                                .gap(3)
                                .items_center()
                                .children(keyParts(k.keys).map(keycap))
                                .when(k.section !== "global", (el) => el.child(div().pl(4).text_size(10.5).text_color(t.dim).child(k.section))),
                            ),
                          ),
                      )
                      .child(div().text_color(t.foreground).child(row.label)),
                  ),
                ),
            ),
          ),
      );
  }
}
