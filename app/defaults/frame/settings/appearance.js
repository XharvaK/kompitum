// Settings › Appearance: the theme, colours, layout, zoom and fonts.

import { div, image } from "gpui-kit";
import { v_flex, h_flex, Input } from "gpui-base";
import { run, fonts, themes, layouts, settings, shared } from "kompitum";

const APPEARANCES = [
  [null, "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];

// Shipped layout previews (window.rs's LAYOUT_PREVIEWS, and Control's picture
// inside its mod). Layouts without one get a quiet placeholder tile.
const LAYOUT_PREVIEWS = {
  control: "mods/control-panel/preview.svg",
  desk: "layouts/desk.svg",
  focus: "layouts/focus.svg",
  office: "layouts/office.svg",
  pasture: "layouts/pasture.svg",
  watch: "layouts/watch.svg",
  win98: "layouts/win98.svg",
};

const LAYOUT_TITLES = {
  main: "Kompitum",
  terminals: "Terminals",
  win98: "Kompitum 98",
  desk: "Desk",
  watch: "Watch",
  focus: "Focus",
  office: "Office",
  control: "Control",
  pasture: "Pasture",
};

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

// The shown theme's name; a click opens the shipped and personal themes.
function themePicker(view, t, cx) {
  const current = settings().theme;
  const open = view.themeOpen;
  const head = div()
    .id("st-colours")
    .px(12)
    .py(6)
    .rounded(7)
    .border_1()
    .border_color(open ? t.accent : t.border)
    .bg(t.raised)
    .cursor_pointer()
    .text_size(13)
    .text_color(t.foreground)
    .hover((s) => s.bg(t.hover))
    .on_click((_e, cx) => {
      view.themeOpen = !open;
      cx.notify();
    })
    .child(current);
  const list = v_flex()
    .w(240)
    .p(4)
    .gap(2)
    .rounded(8)
    .border_1()
    .border_color(t.border)
    .bg(t.background)
    .on_scroll_wheel((_e, cx) => cx.stop_propagation())
    .children(
      themes().map((item) => {
        const on = item.name === current;
        return h_flex()
          .id(`st-colours-${item.name}`)
          .w_full()
          .px(9)
          .py(6)
          .gap(8)
          .items_center()
          .rounded(6)
          .cursor_pointer()
          .when(on, (el) => el.bg(t.selected))
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => {
            view.themeOpen = false;
            run("app.theme", { name: item.name });
            cx.notify();
          })
          .child(
            div()
              .w(12)
              .h(12)
              .flex_none()
              .rounded(3)
              .bg(item.colors?.background ?? t.background)
              .border_1()
              .border_color(t.border),
          )
          .child(div().text_size(13).text_color(t.foreground).child(item.name));
      }),
    );
  return v_flex()
    .items_end()
    .gap(6)
    .child(head)
    .when(open, (el) => el.child(list));
}

// Small preview tiles for the layouts on disk; the shown one is outlined.
function layoutPicker(view, t, cx) {
  const current = shared("layout") ?? "desk";
  return h_flex()
    .flex_wrap()
    .gap(8)
    .max_w(440)
    .children(
      layouts().map((name) => {
        const on = name === current;
        const preview = LAYOUT_PREVIEWS[name];
        return v_flex()
          .id(`st-layout-${name}`)
          .w(104)
          .gap(5)
          .p(5)
          .rounded(8)
          .cursor_pointer()
          .border_1()
          .border_color(on ? t.accent : t.border)
          .when(on, (el) => el.bg(t.selected))
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => {
            run("layout.switch", { name });
            cx.notify();
          })
          .child(
            div()
              .w_full()
              .h(52)
              .rounded(5)
              .overflow_hidden()
              .bg(t.background)
              .border_1()
              .border_color(t.border)
              .when(Boolean(preview), (el) => el.child(image(preview).w_full().h_full()))
              .when(!preview, (el) =>
                el.child(
                  h_flex()
                    .size_full()
                    .p(6)
                    .gap(3)
                    .items_center()
                    .child(div().w(14).h_full().rounded(2).bg(t.hover))
                    .child(v_flex().flex_1().gap(3)
                      .child(div().h(6).rounded(2).bg(t.hover))
                      .child(div().flex_1().rounded(2).bg(t.hover))),
                ),
              ),
          )
          .child(div().text_size(11.5).text_color(on ? t.foreground : t.muted).child(LAYOUT_TITLES[name] ?? name));
      }),
    );
}

// The font in use, written in itself; a click opens the list.
function fontButton(view, t, which, family) {
  const open = view.picking === which;
  return div()
    .id(`st-f-${which}`)
    .w(220)
    .px(12)
    .py(6)
    .rounded(7)
    .border_1()
    .border_color(open ? t.accent : t.border)
    .bg(t.raised)
    .cursor_pointer()
    .hover((s) => s.bg(t.hover))
    .on_click((_e, cx) => {
      view.picking = open ? null : which;
      view.filter = "";
      view.fontInput.set_value("");
      if (view.picking) view.fontInput.focus();
      else view.handle.focus();
      cx.notify();
    })
    .child(div().font_family(family).text_size(13.5).text_color(t.foreground).child(family));
}

function fontList(view, t, which, def) {
  const q = view.filter.toLowerCase();
  const matches = fonts().filter((f) => f.toLowerCase().includes(q)).slice(0, 200);
  const choose = (name, cx) => {
    view.change((p) => {
      if (name === null) delete p[which];
      else p[which] = name;
    }, cx);
    view.picking = null;
    view.handle.focus();
    cx.notify();
  };
  const item = (id, label, family, onClick) =>
    div()
      .id(id)
      .px(10)
      .py(5)
      .rounded(6)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_click(onClick)
      .child(div().font_family(family).text_size(13.5).text_color(t.foreground).child(label));
  return v_flex()
    .w_full()
    .mb(8)
    .rounded(8)
    .border_1()
    .border_color(t.border)
    .bg(t.background)
    // Let the child's native scroll run before stopping the page's wheel event.
    .on_scroll_wheel((_e, cx) => cx.stop_propagation())
    .child(
      div()
        .px(12)
        .py(8)
        .border_b(1)
        .border_color(t.border)
        .text_size(13)
        .child(Input.new(view.fontInput).h(28).text_color(t.foreground)),
    )
    .child(
      div()
        .id(`st-fl-${which}`)
        .h(200)
        .p(4)
        .overflow_y_scrollbar()
        .when(!q, (el) => el.child(item(`st-fd-${which}`, `Default (${def})`, def, (_e, cx) => choose(null, cx))))
        .children(matches.map((f, i) => item(`st-fi-${which}-${i}`, f, f, (_e, cx) => choose(f, cx)))),
    );
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
            .child(fontButton(view, t, which, family))
            // The panels' text follows zoom: they size their rows to it.
            .when(Boolean(sizeKey), (el) => el.child(stepper(t, `st-${sizeKey}`, `${Math.round(sizeNow)}`, (_e, cx) => size(sizeKey, sizeNow, -1, cx), (_e, cx) => size(sizeKey, sizeNow, 1, cx)))),
          which,
        ),
      )
      .when(view.picking === which, (el) => el.child(fontList(view, t, which, def)));
  return v_flex()
    .w_full()
    .child(view.row(t, "Theme", null, appearancePicker(view, t, cx), "theme"))
    .child(view.row(t, "Colours", null, themePicker(view, t, cx), "colours"))
    .child(view.row(t, "Layout", null, layoutPicker(view, t, cx), "layout"))
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
