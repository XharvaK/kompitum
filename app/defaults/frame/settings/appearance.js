// Settings › Appearance: light or dark, the window's reset, zoom and fonts.

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { run, fonts, settings, shared } from "kompitum";
import { dropdown } from "./ui.js";

const APPEARANCES = [
  [null, "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];

// What the app uses when settings.json names no font (settings.rs).
const DEFAULT_FONT = "Google Sans Code";
const DEFAULT_UI_FONT = "Google Sans Flex";

function appearancePicker(view, t, cx) {
  const now = view.person.appearance ?? null;
  return h_flex()
    .p(2)
    .gap(2)
    .rounded(8)
    .border_1()
    .border_color(t.border)
    .bg(t.background)
    .children(
      APPEARANCES.map(([value, label]) =>
        div()
          .id(`st-ap-${label}`)
          .px(12)
          .py(4)
          .rounded(6)
          .text_size(13)
          .cursor_pointer()
          .when(now === value, (el) => el.bg(t.raised).text_color(t.foreground).border_1().border_color(t.border))
          .when(now !== value, (el) => el.text_color(t.muted).hover((s) => s.text_color(t.foreground)))
          .on_click((_e, cx) =>
            view.change((p) => {
              if (value === null) delete p.appearance;
              else p.appearance = value;
            }, cx),
          )
          .child(label),
      ),
    );
}

// − value +
function stepper(t, id, value, onMinus, onPlus) {
  const b = (k, label, f) =>
    div()
      .id(`${id}-${k}`)
      .w(28)
      .h(28)
      .flex()
      .items_center()
      .justify_center()
      .cursor_pointer()
      .text_size(15)
      .text_color(t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      .on_click(f)
      .child(label);
  return h_flex()
    .items_center()
    .rounded(7)
    .border_1()
    .border_color(t.border)
    .bg(t.raised)
    .overflow_hidden()
    .child(b("minus", "\u2212", onMinus))
    .child(div().w(54).text_size(13).text_color(t.foreground).flex().justify_center().child(value))
    .child(b("plus", "+", onPlus));
}

// One look (REDESIGN): no layout or colour pickers, only the way back to the Desk as it ships.
function layoutReset(view, t) {
  const current = shared("layout") ?? "desk";
  return view.button(t, "st-layout-reset", current === "desk" ? "Reset" : "Back to Desk", () => {
    run("layout.reset", { name: "desk" });
    if (current !== "desk") run("layout.switch", { name: "desk" });
  });
}

function fontMenu(view, which) {
  view.fontMenus ??= {};
  return (view.fontMenus[which] ??= {});
}

function fontDropdown(view, t, cx, which, def) {
  const saved = view.person?.[which] ?? "";
  const names = fonts();
  const items = [{ value: "", label: `Kompitum's default (${def})`, font: def }];
  if (saved && !names.includes(saved)) items.push({ value: saved, label: saved, font: saved });
  for (const name of names) items.push({ value: name, label: name, font: name });
  return dropdown(t, `st-f-${which}`, {
    items,
    value: saved,
    onPick: (name, cx) => {
      view.change((p) => {
        if (name === "") delete p[which];
        else p[which] = name;
      }, cx);
      view.handle.focus();
    },
    state: fontMenu(view, which),
    search: true,
    width: 260,
    limit: 200,
  });
}

export function appearancePage(view, t, cx) {
  const s = settings();
  const p = view.person;
  const own = (key, composed, def) => (p[key] ?? def) !== composed;
  const size = (key, value, d, cx) => view.change((q) => (q[key] = Math.max(8, Math.min(32, (q[key] ?? value) + d))), cx);
  const fontRow = (which, sizeKey, title, family, def, sizeNow) =>
    v_flex()
      .w_full()
      .child(
        view.row(
          t,
          title,
          own(which, family, def) ? "This layout sets its own; yours shows in the others." : null,
          h_flex()
            .gap(8)
            .items_center()
            .child(fontDropdown(view, t, cx, which, def))
            // The panels' text follows zoom: they size their rows to it.
            .when(Boolean(sizeKey), (el) => el.child(stepper(t, `st-${sizeKey}`, `${Math.round(sizeNow)}`, (_e, cx) => size(sizeKey, sizeNow, -1, cx), (_e, cx) => size(sizeKey, sizeNow, 1, cx)))),
          which,
        ),
      );
  return v_flex()
    .w_full()
    .child(view.row(t, "Theme", null, appearancePicker(view, t, cx), "theme"))
    .child(view.row(t, "Window", "Puts the window back as Kompitum ships it: its panes, sizes and drawers.", layoutReset(view, t), "layout"))
    .child(div().h(18))
    .child(view.heading(t, "Size and fonts"))
    .child(
      view.row(
        t,
        "Zoom",
        null,
        h_flex()
          .gap(8)
          .items_center()
          .child(stepper(t, "st-zoom", `${Math.round(s.zoom * 100)}%`, () => run("app.zoom_out"), () => run("app.zoom_in")))
          .child(view.button(t, "st-zoom-reset", "Reset", () => run("app.zoom_reset"))),
        "zoom",
      ),
    )
    .child(fontRow("font", "font_size", "Terminal font", s.font_family, DEFAULT_FONT, s.font_size))
    .child(fontRow("ui_font", null, "Interface font", s.ui_font_family, DEFAULT_UI_FONT));
}
