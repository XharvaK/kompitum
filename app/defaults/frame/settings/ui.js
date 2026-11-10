// The rework's shared controls (audit/REWORK-2026-11-09.md, "Design"): one dropdown and one icon button,
// so every picker in Settings looks and behaves the same.

import { div, svg, image } from "gpui-kit";
import { v_flex, h_flex, Input, InputState, Popover } from "gpui-base";

const CHEVRON = "mods/workspace-chrome/chevron-down.svg";
// Popover and Input claim Enter and the arrows with bindings, which GPUI runs
// before on_key_down. These bindings sit in a context on the list itself, so
// they come first while one is open and mean nothing anywhere else.
const KEYS = "SettingsDropdown";
let keysBound = false;

function bindKeys(cx) {
  if (keysBound) return;
  keysBound = true;
  const bindings = [];
  for (const key of ["down", "up", "enter", "escape"]) {
    const action = `settings-dropdown-${key}`;
    // On the list itself, and at the depth of a search field inside it: the
    // second outranks Input's own arrows and Enter, which sit at that depth.
    bindings.push({ keystroke: key, action, context: KEYS });
    bindings.push({ keystroke: key, action, context: `${KEYS} > Input` });
  }
  cx.bind_keys(bindings);
}

function itemIcon(item, size, t) {
  if (!item?.icon) return null;
  return item.tint
    ? svg(item.icon).size(size).flex_none().text_color(t.muted)
    : image(item.icon).w(size).h(size).flex_none();
}

// A dropdown: a trigger that shows the pick and a floating list to choose from.
// `items`: [{ value, label, icon?, tint?, detail?, font? }]. `state`: an object the caller keeps across renders
// (it holds whether the list is open, the search field and the highlighted row; `accent` puts the search
// highlight on the trigger). `search`: a field over the list that filters it. `onOpen(cx)`: called when the list
// opens, for lists that load on demand. `limit`: draw at most that many matching rows, then a dim line saying to
// type to narrow it.
export function dropdown(t, id, { items, value, onPick, state, search = false, width = 240, anchor = "top_right", empty = "Choose…", onOpen = null, limit = 0 }) {
  const picked = items.find((item) => item.value === value);
  const query = (state.query ?? "").trim().toLowerCase();
  const matching = query
    ? items.filter((item) => `${item.label ?? ""} ${item.detail ?? ""}`.toLowerCase().includes(query))
    : items;
  const shown = limit > 0 ? matching.slice(0, limit) : matching;
  // The keyboard and the pointer share one highlight, kept on the caller's state. It stays in range when the
  // filter shrinks the list, and starts on the first row whenever there is none yet.
  const high = shown.length ? Math.max(0, Math.min(shown.length - 1, Number.isInteger(state.hi) ? state.hi : 0)) : -1;
  const close = (cx) => {
    state.open = false;
    state.query = "";
    state.input?.set_value("");
    cx.notify();
  };
  const pick = (item, cx) => {
    if (!item) return;
    close(cx);
    onPick(item.value, cx);
  };
  const key = (name, cx) => {
    if (name === "down" || name === "up") {
      if (shown.length) {
        const step = name === "down" ? 1 : shown.length - 1;
        state.hi = (high + step) % shown.length;
        cx.notify();
      }
    } else if (name === "enter") {
      // With nothing shown, Enter stays ours: it must not close the list.
      if (high >= 0) pick(shown[high], cx);
    } else if (name === "escape") {
      close(cx);
    } else return;
    cx.stop_propagation();
  };
  const row = (item, index) => {
    const on = item.value === value;
    return h_flex()
      .id(`${id}-${item.value}`)
      .w_full()
      .min_h(30)
      .px(8)
      .py(5)
      .gap(8)
      .items_center()
      .rounded(6)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .when(index === high, (el) => el.bg(t.hover))
      .on_hover((hovered, cx) => {
        if (!hovered || state.hi === index) return;
        state.hi = index;
        cx.notify();
      })
      .on_click((_e, cx) => pick(item, cx))
      .when(Boolean(item.icon), (el) => el.child(itemIcon(item, 15, t)))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .child(div().truncate().text_size(13).text_color(t.foreground).when(Boolean(item.font), (el) => el.font_family(item.font)).child(item.label))
          .when(Boolean(item.detail), (el) => el.child(div().truncate().text_size(11.5).text_color(t.dim).child(item.detail))),
      )
      .child(div().w(14).flex_none().when(on, (el) => el.child(svg("frame/settings/check.svg").size(14).text_color(t.accent))));
  };
  const list = v_flex()
    .id(`${id}-list`)
    .w_full()
    .max_h(300)
    .p(4)
    .gap(1)
    .overflow_y_scrollbar()
    .children(shown.map(row))
    .when(matching.length === 0, (el) => el.child(div().px(8).py(6).text_size(12.5).text_color(t.dim).child("Nothing matches.")))
    .when(limit > 0 && matching.length > limit, (el) =>
      el.child(div().px(8).py(6).text_size(12.5).text_color(t.dim).child("Type to narrow the list.")));
  const content = v_flex()
    .w(width)
    .rounded(10)
    .border_1()
    .border_color(t.border)
    .bg(t.popover)
    .overflow_hidden()
    // The list scrolls natively first; its wrapper keeps the wheel from reaching the page under it.
    .on_scroll_wheel((_e, cx) => cx.stop_propagation())
    .when(Boolean(state.focus), (el) => el.track_focus(state.focus))
    .key_context(KEYS)
    .on_action("settings-dropdown-down", (_e, cx) => key("down", cx))
    .on_action("settings-dropdown-up", (_e, cx) => key("up", cx))
    .on_action("settings-dropdown-enter", (_e, cx) => key("enter", cx))
    .on_action("settings-dropdown-escape", (_e, cx) => key("escape", cx))
    .when(search && Boolean(state.input), (el) =>
      el.child(div().px(10).py(6).border_b(1).border_color(t.border).child(Input.new(state.input).h(26).text_size(13).text_color(t.foreground))))
    .child(list);
  const trigger = h_flex()
    .id(id)
    .h(30)
    .max_w(width)
    .pl(10)
    .pr(8)
    .gap(7)
    .items_center()
    .rounded(8)
    .border_1()
    .border_color(state.open || state.accent ? t.accent : t.input)
    .bg(t.raised)
    .cursor_pointer()
    .hover((s) => s.bg(t.hover))
    .when(Boolean(picked?.icon), (el) => el.child(itemIcon(picked, 15, t)))
    .child(div().flex_1().min_w(0).truncate().text_size(13).text_color(picked ? t.foreground : t.muted)
      .when(Boolean(picked?.font), (el) => el.font_family(picked.font))
      .child(picked?.label ?? empty))
    .child(svg(CHEVRON).size(12).flex_none().text_color(t.dim));
  const popover = Popover.new(`${id}-menu`)
    .anchor(anchor)
    .open(Boolean(state.open))
    .when(Boolean(state.focus), (el) => el.track_focus(state.focus))
    .on_open_change((open, cx) => {
      if (open) {
        state.open = true;
        state.hi = 0;
        bindKeys(cx);
        // Retained state is created in an event handler, never in render.
        state.focus ??= cx.focus_handle();
        if (search && !state.input) {
          state.input = InputState.new({ placeholder: "Search" });
          state.input.on("change", (_e, cx) => { state.query = state.input.value(); state.hi = 0; cx.notify(); });
        }
        cx.notify();
        if (search) {
          state.input?.focus();
          cx.timer.after(0, () => state.input?.focus());
        } else {
          state.focus?.focus();
          cx.timer.after(0, () => state.focus?.focus());
        }
        onOpen?.(cx);
      } else close(cx);
    })
    .trigger(trigger);
  // Only fill the content slot while the list is open, so a closed dropdown is
  // just its trigger (and its description holds no rows).
  return state.open ? popover.content(content) : popover;
}

// A 28 px icon button: muted at rest, the foreground on hover, a tooltip naming it.
export function iconButton(t, id, icon, tooltip, onClick) {
  return h_flex()
    .id(id)
    .w(28)
    .h(28)
    .flex_none()
    .items_center()
    .justify_center()
    .rounded(7)
    .cursor_pointer()
    .text_color(t.muted)
    .hover((s) => s.bg(t.hover).text_color(t.foreground))
    .tooltip(tooltip)
    .on_click(onClick)
    .child(svg(icon).size(16));
}
