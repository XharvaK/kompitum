// The rework's shared controls (audit/REWORK-2026-11-09.md, "Design"): one dropdown and one icon button,
// so every picker in Settings looks and behaves the same.

import { div, svg, image } from "gpui-kit";
import { v_flex, h_flex, Input, InputState, Popover } from "gpui-base";

const CHEVRON = "mods/workspace-chrome/chevron-down.svg";

function itemIcon(item, size, t) {
  if (!item?.icon) return null;
  return item.tint
    ? svg(item.icon).size(size).flex_none().text_color(t.muted)
    : image(item.icon).w(size).h(size).flex_none();
}

// A dropdown: a trigger that shows the pick and a floating list to choose from.
// `items`: [{ value, label, icon?, tint?, detail?, font? }]. `state`: an object the caller keeps across renders
// (it holds whether the list is open and the search field). `search`: a field over the list that filters it.
export function dropdown(t, id, { items, value, onPick, state, search = false, width = 240, anchor = "top_right", empty = "Choose…" }) {
  const picked = items.find((item) => item.value === value);
  if (search && !state.input) {
    state.input = InputState.new({ placeholder: "Search" });
    state.input.on("change", (_e, cx) => { state.query = state.input.value(); cx.notify(); });
  }
  const query = (state.query ?? "").trim().toLowerCase();
  const shown = query ? items.filter((item) => String(item.label).toLowerCase().includes(query)) : items;
  const close = (cx) => {
    state.open = false;
    state.query = "";
    state.input?.set_value("");
    cx.notify();
  };
  const row = (item) => {
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
      .on_click((_e, cx) => {
        close(cx);
        onPick(item.value, cx);
      })
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
    .on_scroll_wheel((_e, cx) => cx.stop_propagation())
    .children(shown.map(row))
    .when(shown.length === 0, (el) => el.child(div().px(8).py(6).text_size(12.5).text_color(t.dim).child("Nothing matches.")));
  const content = v_flex()
    .w(width)
    .rounded(10)
    .border_1()
    .border_color(t.border)
    .bg(t.popover)
    .overflow_hidden()
    .when(search, (el) =>
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
    .border_color(state.open ? t.accent : t.input)
    .bg(t.raised)
    .cursor_pointer()
    .hover((s) => s.bg(t.hover))
    .when(Boolean(picked?.icon), (el) => el.child(itemIcon(picked, 15, t)))
    .child(div().min_w(0).truncate().text_size(13).text_color(picked ? t.foreground : t.muted).child(picked?.label ?? empty))
    .child(svg(CHEVRON).size(12).flex_none().text_color(t.dim));
  return Popover.new(`${id}-menu`)
    .anchor(anchor)
    .open(Boolean(state.open))
    .on_open_change((open, cx) => {
      if (open) {
        state.open = true;
        cx.notify();
        if (search) cx.timer.after(0, () => state.input?.focus());
      } else close(cx);
    })
    .trigger(trigger)
    .content(content);
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
