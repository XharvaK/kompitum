// The room's fixtures: wall pieces, the door, weather in the windows, seasonal touches.
// Each function returns elements positioned in the scene's px.
import { div } from "gpui-kit";
import { run } from "limen";
import * as A from "./art.js";

const WOOD = "#7a5c3e";
const INK = "#3b3b44";
const PAPER = "#fbf8f1";

export function windowPane(x, y, w, h, sky, weather) {
  const dayish = !sky.stars && (weather === "clear" || weather === "clouds" || !weather);
  return div().absolute().left(x).top(y).w(w).h(h).border_3().border_color(WOOD).bg(sky.bottom).overflow_hidden()
    .child(div().absolute().left(0).top(0).w_full().h(h * 0.55).bg(weather === "rain" || weather === "storm" ? "#6f8296" : sky.top))
    .when(Boolean(sky.stars) && weather !== "fog", (el) => el.children([[14, 10], [70, 18], [96, 6], [40, 26]].map(([sx, sy]) =>
      div().absolute().left(sx).top(sy).size(2).bg("#fff7d6").loop("opacity", { from: 0.3, to: 1, period: 1800 + sx * 20 }))))
    .when(Boolean(sky.stars) || dayish, (el) => el.child(div().absolute().left(w - 34).top(8).size(14).rounded_full().bg(sky.stars ? "#f1ecd2" : "#ffe28a")))
    .children(weatherIn(weather, w, h))
    .child(div().absolute().left(w / 2 - 4).top(0).w(3).h_full().bg(WOOD))
    .child(div().absolute().left(0).top(h / 2 - 3).w_full().h(3).bg(WOOD))
    .child(div().absolute().left(-6).top(h - 6).w(w + 6).h(5).bg("#d8c3a2"));
}

function weatherIn(kind, w, h) {
  const n = (k, a, b) => a + ((k * 7919) % (b - a));
  if (kind === "rain" || kind === "storm") {
    const drops = Array.from({ length: 9 }, (_, k) => div().absolute().left(n(k, 4, w - 6)).w(1).h(7).bg("#d6e6ff")
      .loop("top", { from: -10 - (k % 3) * 9, to: h, period: 520 + (k % 4) * 90, easing: "linear" }));
    if (kind === "storm") drops.push(div().absolute().left(0).top(0).w(w).h(h).bg("#ffffff")
      .loop("opacity", { from: 0, to: 0.45, period: 3800, easing: "ease-in" }));
    return drops;
  }
  if (kind === "snow") {
    return Array.from({ length: 11 }, (_, k) => {
      const x = n(k, 4, w - 6);
      return div().absolute().left(x).size(3).rounded_full().bg("#ffffff")
        .loop("top", { from: -6 - (k % 4) * 10, to: h, period: 3000 + (k % 5) * 400, easing: "linear" })
        .loop("left", { from: x - 3, to: x + 3, period: 1400 + (k % 3) * 300 });
    });
  }
  if (kind === "clouds") {
    return [0, 1].map((k) => div().absolute().top(6 + k * 14).w(34 - k * 8).h(10).rounded(5).bg(k ? "#e9eef3" : "#ffffff")
      .loop("left", { from: -36, to: w, period: 26000 + k * 9000, easing: "linear" }));
  }
  if (kind === "fog") return [div().absolute().left(0).top(0).w(w).h(h).bg("#e8eef2").opacity(0.6)];
  return [];
}

export function poster(x, y, h, [text, bg]) {
  return div().absolute().left(x).top(y).w(70).h(Math.min(h, 92)).p(3).bg(PAPER).border_1().border_color(WOOD)
    .child(div().size_full().bg(bg).flex().flex_col().items_center().justify_center().text_size(10).font_bold()
      .text_color("#fffaf0").children(text.split("\n").map((l) => div().child(l))));
}

export function clock(x, y, text) {
  return div().absolute().left(x).top(y).size(42).rounded_full().bg(PAPER).border_3().border_color(INK)
    .flex().items_center().justify_center().text_size(10).font_bold().text_color(INK).child(text);
}

export function whiteboard(x, y, rows) {
  return div().absolute().left(x).top(y).w(150).p(6).bg("#fbfbf8").border_3().border_color("#a6adb8").rounded(2)
    .flex().flex_col().text_size(10).font_bold()
    .children(rows.filter(([, , show]) => show !== false).map(([color, text]) => div().text_color(color).child(text)));
}

// "DAYS SINCE LAST INCIDENT", flipped back to a blinking 0 while someone is blocked.
export function incidentSign(x, y, h, days, now) {
  return div().absolute().left(x).top(y).w(96).h(Math.min(h, 96)).p(5).bg("#f6d44c").border_3().border_color(INK)
    .flex().flex_col().items_center().justify_between().text_color(INK)
    .child(div().text_size(8).font_bold().child("DAYS SINCE LAST"))
    .child(div().px(8).rounded(3).bg("#1d1d24").text_size(24).font_bold().text_color(now ? "#ff5a5a" : "#7dff9a")
      .child(String(now ? 0 : days)).when(now, (el) => el.loop("opacity", { from: 0.35, to: 1, period: 600 })))
    .child(div().text_size(8).font_bold().child("INCIDENT"));
}

// A framed head-and-shoulders of the busiest agent.
export function portrait(x, y, h, who) {
  const p = 3;
  const face = who
    ? div().absolute().left(16).top(-(A.BOX - 21) * p + 4).w(A.PERSON * p).h(A.BOX * p)
      .children(A.rects([...A.STAND.body, ...(who.remote ? A.HEADSET.stand : [])], who.pal, p))
    : div().size_full().flex().items_center().justify_center().text_size(18).text_color("#fffaf0").child("?");
  return div().absolute().left(x).top(y).w(84).flex().flex_col().items_center().gap(2)
    .child(div().w(70).h(Math.min(h - 26, 62)).p(3).bg("#c99640").border_1().border_color("#8a6420")
      .child(div().relative().size_full().overflow_hidden().bg("#3d4a6b").child(face)))
    .child(div().w(84).py(1).rounded(2).bg("#e0b43d").flex().flex_col().items_center().text_color("#3b2a1a")
      .child(div().text_size(7).font_bold().child("EMPLOYEE OF THE MONTH"))
      .child(div().max_w(80).truncate().text_size(8).child(who?.name ?? "vacant")));
}

// A call on the wall with everyone working from another machine: each tile shows how
// they're doing, and clicking one goes to their terminal.
export function videoWall(x, y, h, remotes) {
  const p = 3;
  const ring = { working: "#3d8be0", blocked: "#e05252", done: "#58c26b", idle: "#5b6270" };
  const tile = (r) => {
    const st = r.status;
    const face = div().absolute().left(-1).top(-(A.BOX - 21) * p + 4).w(A.PERSON * p).h(A.BOX * p)
      .children(A.rects([...A.STAND.body, ...A.HEADSET.stand], r.pal, p))
      .when(st === "working", (el) => el.loop("top", { from: -(A.BOX - 21) * p + 4, to: -(A.BOX - 21) * p + 2, period: 320 }));
    return div().id(`call-${r.address}`).w(48).h(48).relative().overflow_hidden().rounded(3).bg("#24425a")
      .border_2().border_color(ring[st] ?? ring.idle).cursor_pointer().tooltip(r.title)
      .on_click(() => run("pane.focus", { pane: r.pane }))
      .child(face)
      .when(st === "working", (el) => el.child(div().absolute().left(0).top(0).size_full().bg("#7fb6ff").opacity(0.12)))
      .when(st === "idle", (el) => el.child(div().absolute().left(0).top(0).size_full().bg("#0b1030").opacity(0.5))
        .child(div().absolute().right(3).top(1).text_size(8).font_bold().text_color("#c9c2ff").frames(["z", "zZ", "zZz"], 900)))
      .when(st === "blocked", (el) => el.child(div().absolute().right(2).top(1).px(3).rounded(3).bg("#e05252")
        .text_size(9).font_bold().text_color("#ffffff").child("!").loop("opacity", { from: 0.4, to: 1, period: 600 })))
      .when(st === "done", (el) => el.child(div().absolute().right(2).top(1).px(3).rounded(3).bg("#2f9a50")
        .text_size(9).font_bold().text_color("#ffffff").child("✓")))
      .child(div().absolute().left(0).bottom(0).w_full().h(10).bg("#000000aa").px(2).flex().items_center()
        .text_size(7).text_color("#ffffff").child(div().truncate().child(r.name)));
  };
  const machines = [...new Set(remotes.map((r) => r.machine))].join(", ");
  return div().absolute().left(x).top(y).p(4).bg("#1d1d24").rounded(3).border_2().border_color("#3b3b44").flex().flex_col().gap(3)
    .child(div().flex().items_center().gap(4).max_w(48 * 3 + 6).text_size(8).font_bold()
      .child(div().size(5).flex_none().rounded_full().bg("#e05252").loop("opacity", { from: 0.3, to: 1, period: 1200 }))
      .child(div().truncate().text_color("#cfd6e0").child(`REMOTE · ${machines}`)))
    .child(div().flex().flex_wrap().gap(3).max_w(48 * 3 + 6).max_h(h - 22).overflow_hidden().children(remotes.slice(0, 6).map(tile)));
}

// The way in and out, on the back wall. Open while someone passes through.
export function door(x, wallBottom, P, open) {
  const w = 16 * P, h = Math.min(28 * P, wallBottom - 30);
  return div().absolute().left(x).top(wallBottom - h).w(w).h(h)
    .child(div().absolute().left(w / 2 - 18).top(-20).w(36).h(14).rounded(2).bg("#2f9a50").flex().items_center().justify_center()
      .text_size(8).font_bold().text_color("#eaffef").child("EXIT"))
    .child(div().absolute().left(0).top(0).w(w).h(h).bg("#5c3a22"))
    .child(div().absolute().left(P).top(P).w(w - 2 * P).h(h - P).bg(open ? "#1f1a16" : "#8a5a35"))
    .when(open, (el) => el.child(div().absolute().left(P).top(h - 4 * P).w(w - 2 * P).h(3 * P).bg("#3a302a")))
    .when(!open, (el) => el
      .child(div().absolute().left(3 * P).top(4 * P).w(w - 6 * P).h(h / 2 - 6 * P).border_2().border_color("#7d4f2e"))
      .child(div().absolute().left(3 * P).top(h / 2 + P).w(w - 6 * P).h(h / 2 - 5 * P).border_2().border_color("#7d4f2e"))
      .child(div().absolute().left(w - 4 * P).top(h / 2).size(2 * P).rounded_full().bg("#e0b43d")));
}

export function stringLights(W, y) {
  const colors = ["#e05252", "#e0b43d", "#58b368", "#3d8be0"];
  const els = [div().absolute().left(0).top(y).w(W).h(1).bg("#2f4a2f")];
  for (let x = 14, k = 0; x < W; x += 28, k++)
    els.push(div().absolute().left(x).top(y + 1).w(5).h(7).rounded(2).bg(colors[k % 4])
      .loop("opacity", { from: k % 2 ? 0.35 : 1, to: k % 2 ? 1 : 0.35, period: 1600 }));
  return els;
}

export function banner(W, y, text, bg) {
  return div().absolute().left(W / 2 - 110).top(y).w(220).h(22).rounded(3).bg(bg).flex().items_center().justify_center()
    .text_size(12).font_bold().text_color("#fffaf0").child(text);
}

// A spinning red light over the desk, and the whole room washed red in time.
export function alarm(W, H, x) {
  return [
    div().absolute().left(0).top(0).w(W).h(H).bg("#ff2a2a").loop("opacity", { from: 0, to: 0.13, period: 900 }),
    div().absolute().left(x - 10).top(0).w(20).h(6).bg("#3b3b44"),
    div().absolute().left(x - 8).top(6).w(16).h(10).rounded(5).bg("#ff3b3b").loop("opacity", { from: 0.4, to: 1, period: 450 }),
    div().absolute().left(x - 40).top(4).w(80).h(26).rounded_full().bg("#ff5a5a").loop("opacity", { from: 0, to: 0.35, period: 450 }),
  ];
}
