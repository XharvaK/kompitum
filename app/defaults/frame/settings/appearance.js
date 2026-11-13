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

// The fixed colours of each tile's picture, so it shows the look whatever is chosen.
const WINDOW_LOOKS = {
  Light: { window: "#f4f3ef", sidebar: "#e9e7e1", bars: "#cfccc4", lines: "#bdb9b0", composer: "#ffffff" },
  Dark: { window: "#1c1e23", sidebar: "#15171a", bars: "#33373e", lines: "#454a53", composer: "#2a2d33" },
};

// A small window: a sidebar with three bars, and a main area with two message lines and a composer.
function windowArt(c) {
  return h_flex()
    .w(112)
    .h(72)
    .bg(c.window)
    .child(v_flex().w(28).h_full().p(6).gap(5).bg(c.sidebar).children([0, 1, 2].map(() => div().h(4).w(16).rounded(2).bg(c.bars))))
    .child(
      v_flex()
        .flex_1()
        .h_full()
        .justify_end()
        .child(v_flex().px(8).pt(10).gap(5).child(div().h(4).w(52).rounded(2).bg(c.lines)).child(div().h(4).w(36).rounded(2).bg(c.lines)))
        .child(div().h(12).rounded(4).mx(8).mb(8).bg(c.composer)),
    );
}

// System: the left half of Light's window and the right half of Dark's.
function tileArt(which) {
  if (which !== "System") return windowArt(WINDOW_LOOKS[which]);
  return h_flex()
    .w(112)
    .h(72)
    .child(div().w(56).h_full().overflow_hidden().child(windowArt(WINDOW_LOOKS.Light)))
    .child(div().w(56).h_full().overflow_hidden().child(div().ml(-56).child(windowArt(WINDOW_LOOKS.Dark))));
}

// The Theme row's three tiles: a picture and its name. A click does what the old picker did.
function appearanceTiles(view, t) {
  const now = view.person.appearance ?? null;
  return h_flex()
    .gap(12)
    .children(
      APPEARANCES.map(([value, label]) => {
        const chosen = now === value;
        return v_flex()
          .id(`st-ap-${label}`)
          .gap(6)
          .items_center()
          .cursor_pointer()
          .on_click((_e, cx) =>
            view.change((p) => {
              if (value === null) delete p.appearance;
              else p.appearance = value;
            }, cx),
          )
          .child(
            div()
              .w(112)
              .h(72)
              .rounded(8)
              .overflow_hidden()
              .border_2()
              .border_color(chosen ? t.accent : t.border)
              .when(!chosen, (el) => el.hover((s) => s.border_color(t.input)))
              .child(tileArt(label)),
          )
          .child(div().text_size(12.5).text_color(chosen ? t.foreground : t.muted).child(label));
      }),
    );
}

// The fonts' sample: a reply, a diff and a terminal, in the chosen fonts and sizes.
function sample(t, s) {
  const ui = s.ui_font_family ?? DEFAULT_UI_FONT;
  const mono = s.font_family ?? DEFAULT_FONT;
  const size = Math.floor((s.font_size ?? 14) * 0.85);
  const diffLine = (text, color) =>
    div()
      .px(10)
      .py(2)
      .text_size(size)
      .font_family(mono)
      .text_color(color)
      .bg(color + "1f")
      .child(text);
  return v_flex()
    .id("st-sample")
    .w_full()
    .gap(10)
    .p(14)
    .rounded(10)
    .border_1()
    .border_color(t.border)
    .bg(t.background)
    .child(
      h_flex()
        .justify_end()
        .child(
          div()
            .px(12)
            .py(7)
            .rounded(12)
            .bg(t.raised)
            .text_size(13)
            .text_color(t.foreground)
            .font_family(ui)
            .child("Fix the login bug, please"),
        ),
    )
    .child(div().text_size(13).text_color(t.foreground).font_family(ui).child("Found it: the token check ran before the session loaded."))
    .child(
      v_flex()
        .rounded(6)
        .overflow_hidden()
        .border_1()
        .border_color(t.border)
        .child(diffLine("- if (!token) return deny();", t.ansi1))
        .child(diffLine("+ if (!session || !token) return deny();", t.ansi2)),
    )
    .child(
      v_flex()
        .p(10)
        .rounded(6)
        .bg(t.canvas ?? t.background)
        .text_size(size)
        .font_family(mono)
        .child(div().text_color(t.foreground).child("$ cargo test login"))
        .child(div().text_color(t.ansi2).child("test result: ok. 12 passed")),
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
  const items = [{ value: "", label: def, detail: "Kompitum's default", font: def }];
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
    .child(
      v_flex()
        .w_full()
        .gap(10)
        .py(12)
        .border_b(1)
        .border_color(t.border)
        .when(view.lit("theme"), (el) => el.bg(t.selected).rounded(7))
        .child(div().text_size(14).text_color(t.foreground).child("Theme"))
        .child(appearanceTiles(view, t)),
    )
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
    .child(view.heading(t, "Sample"))
    .child(sample(t, s))
    .child(
      view.advanced(
        t,
        "appearance",
        [
          view.row(t, "Window", "Puts the window back as Kompitum ships it: its panes, sizes and drawers.", layoutReset(view, t), "layout"),
          fontRow("ui_font", null, "Interface font", s.ui_font_family, DEFAULT_UI_FONT),
        ],
        ["layout", "ui_font"],
      ),
    );
}
