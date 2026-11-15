// How notifications look. Anything can post one with notify({ title, body, level, icon, action }); replace
// this mod to restyle them all. A click runs its action (e.g. go to the agent) and dismisses it; × only
// dismisses it. The body's first line wraps to two lines, and Kompitum sizes the toast to fit them.

import { View, div, image, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, run, theme } from "kompitum";

const DOT = { error: "blocked", warning: "working", success: "done", info: "accent" };
const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

export default class Toast extends View {
  render(cx) {
    const t = theme();
    const p = props() ?? {};
    const body = firstLine(p.body);
    const close = div()
      .id("toast-close")
      .size(24)
      .flex_none()
      .flex()
      .items_center()
      .justify_center()
      .rounded(6)
      .text_color(t.dim)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
      .tooltip("Dismiss")
      .accessibility_label("Dismiss")
      .on_click((_e, cx) => {
        cx.stop_propagation();
        closeOverlay(p.overlay);
      })
      .child(svg("mods/pane/close.svg").size(12));
    return h_flex()
      .id("toast")
      .size_full()
      .pl(16)
      .pr(12)
      .py(12)
      .gap(12)
      .items_start()
      .bg(t.popover)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
      .on_click(() => {
        if (p.action?.name) run(p.action.name, p.action.args ?? {});
        closeOverlay(p.overlay);
      })
      .child(p.icon ? image(p.icon).w(16).h(16).mt(2).flex_none() : div().size(8).mt(6).rounded(4).flex_none().bg(t[DOT[p.level] ?? "accent"]))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(2)
          .child(div().truncate().font_semibold().text_size(13).line_height("18px").text_color(t.foreground).child(p.title ?? ""))
          // Two lines at most; the bell's Earlier list keeps the toast once it leaves.
          .when(Boolean(body), (el) => el.child(div().id("toast-body").max_h(32).overflow_hidden().text_size(12).line_height("16px").text_color(t.muted).child(body)))
          .when(Boolean(p.action?.label), (el) => el.child(div().text_size(12).line_height("16px").text_color(t.accent).child(p.action.label))),
      )
      .child(close);
  }
}
