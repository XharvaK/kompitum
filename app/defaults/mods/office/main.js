// The office: every agent is a coworker with a desk of their own. Working: typing.
// Idle: they walk to the water cooler and chat; idle long enough, they nap on the couch.
// Blocked: hand up at their desk (five minutes and the fire alarm goes). Done: cheering in
// confetti until you look. New agents come in the door; closed ones leave with a box.
// Sub-agents sit by their parent's desk as interns and graduate when they finish.
// Three finishes in five minutes and pizza arrives. A cat lives here. Lights go down after
// dark, it snows in the windows in winter, and the office decorates for the season.
// From DustyPolk/herdr-office (MIT, see LICENSE), renamed for Kompitum; the real-weather lookup
// (ipapi.co and Open-Meteo) is removed, so this mod reaches no network.
// Hover anyone to see what they're working on; click them to go to their terminal.
import { View, div, svg } from "gpui-kit";
import { agents, terminals, spaces, panes, shared, run, setAgentOrder } from "kompitum";
import * as A from "./art.js";
import * as D from "./decor.js";

const WALK_SPEED = 0.16; // px per ms
const CAT_SPEED = 0.12;
const NAP_AFTER = 10 * 60 * 1000;
const ALARM_AFTER = 5 * 60 * 1000;
const SWEAT_AFTER = 30 * 60 * 1000;
const MUGS_AFTER = 60 * 60 * 1000;
const PARTY_FINISHES = 3, PARTY_WINDOW = 5 * 60 * 1000, PARTY_LENGTH = 8 * 60 * 1000;
const GRAD_BOW = 2500;
const COUCH_SEATS = 3;
const MIN_DESKS = 3;
const FRONT_ROW = 6;
const DAY = 86400000;
const CHAT = ["…", "☕", "lgtm", "standup?", "🍩", "ship it", "nice", "brb", "tabs > spaces", "🎉", "coffee?", "hmm"];
const PIZZA_CHAT = ["🍕", "nom", "pizza!", "🍕", "best day", "nom nom"];
const CODE = ["#7fd1b9", "#f2b866", "#9aa8ff", "#e88fb0"];
const CONFETTI = ["#e05252", "#e0b43d", "#58b368", "#3d8be0", "#9b6bdc", "#e05c8c"];
const POSTERS = [["SHIP\nIT", "#e07a5f"], ["LGTM", "#3d8be0"], ["BE KIND\nTO YOUR\nAGENTS", "#58a368"], ["push\n--force?\nno.", "#9b6bdc"]];
const STATUS = { working: "#3d8be0", blocked: "#e05252", done: "#58c26b", idle: "#9aa0aa" };
// The sidebar's space colors (mods/my-spaces), by stored name.
const MARK = { ansi6: "#22a99a", ansi3: "#e3814f", ansi4: "#5b8def", ansi5: "#8b7cf6", ansi2: "#3fae6a", ansi1: "#d9649a" };

function sky(hour) {
  if (hour < 5 || hour >= 21) return { top: "#141c38", bottom: "#2a3766", stars: true };
  if (hour < 7) return { top: "#9cc3e6", bottom: "#f4b98a" };
  if (hour < 17) return { top: "#7fc4f0", bottom: "#c4e6f8" };
  if (hour < 19) return { top: "#f7c98b", bottom: "#f29a6b" };
  return { top: "#3a3a6e", bottom: "#e07a5f" };
}
const lightsDown = (hour) => hour >= 20 || hour < 6;

function season(d) {
  const m = d.getMonth(), day = d.getDate();
  if ((m === 11 && day === 31) || (m === 0 && day <= 2)) return "newyear";
  if (m === 11) return "winter";
  if (m === 9 || (m === 10 && day <= 7)) return "halloween";
  if (m === 1 && day === 14) return "valentine";
  return null;
}

const rotate = (list, n) => list.map((_, i) => list[(i + n) % list.length]);
const clock = (d) => `${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
const remote = (address) => address.includes("/");
const mugOf = (a) => {
  const names = `${a.program?.key ?? ""} ${a.app ?? ""}`;
  return Object.entries(A.MUGS).find(([k]) => names.includes(k))?.[1] ?? A.PALETTE.mug;
};
// Live sub-agents under `addr`, however deep.
const descendants = (all, addr) =>
  all.filter((k) => k.parent === addr && k.state === "live").flatMap((k) => [k, ...descendants(all, k.address)]);

function loadNumber(key) {
  try { return Number(localStorage.getItem(key)) || null; } catch { return null; }
}
function saveNumber(key, n) {
  try { localStorage.setItem(key, String(n)); } catch {}
}

export default class Office extends View {
  init(_props, cx) {
    this.size = { width: 1400, height: 250 };
    this.desk = {}; // address -> desk index, kept while the agent lives
    this.at = {}; // walker -> { spot, x, y, fromX, until, dur, face }
    this.seen = {}; // address -> what we knew last render: { status, pal }
    this.idleSince = {};
    this.blockedSince = {};
    this.finishes = []; // when agents finished, for the pizza
    this.party = null; // { start }
    this.leaving = {}; // address -> pal: closed agents on their way out
    this.kids = {}; // intern address -> { x, y, p, pal } where it sat last render
    this.grads = {}; // intern address -> { x, y, p, pal, start }
    this.away = [];
    this.hovered = null;
    this.weather = null;
    this.started = false;
    this.cat = { goal: "sit", desk: null, since: Date.now() };
    this.now = new Date();
    this.incident = loadNumber("office.incident") ?? Date.now();
    saveNumber("office.incident", this.incident);
    cx.timer.every(30000, (cx) => { this.now = new Date(); cx.notify(); });
    cx.timer.every(9000, (cx) => { if (this.wanderCat()) cx.notify(); });
  }

  // Desks are handed out once, lowest free first, so nobody swaps seats. Agents that
  // closed head for the door with a box.
  seat(list, door, cx) {
    const live = new Set(list.map((a) => a.address));
    for (const k of Object.keys(this.desk)) {
      if (live.has(k)) continue;
      const at = this.at[k];
      if (this.started && at && this.seen[k]) {
        this.leaving[k] = this.seen[k].pal;
        this.walk(`leave:${k}`, "door", door.x, door.y, WALK_SPEED, cx, at);
      }
      delete this.desk[k]; delete this.at[k]; delete this.idleSince[k]; delete this.blockedSince[k]; delete this.seen[k];
    }
    const taken = new Set(Object.values(this.desk));
    for (const a of list) {
      if (a.address in this.desk) continue;
      let i = 0;
      while (taken.has(i)) i++;
      this.desk[a.address] = i;
      taken.add(i);
    }
  }

  // Since-when for idle and blocked, finishes for the pizza, and the incident sign.
  track(list, now, cx) {
    const changed = {};
    for (const s of spaces()) for (const t of s.tabs ?? []) changed[t.id] = t.changed_at;
    for (const a of list) {
      const was = this.seen[a.address]?.status;
      if (a.status !== "idle") delete this.idleSince[a.address];
      else this.idleSince[a.address] ??= changed[a.tab] ?? now;
      if (a.status !== "blocked") delete this.blockedSince[a.address];
      else this.blockedSince[a.address] ??= was === "blocked" || !was ? (changed[a.tab] ?? now) : now;
      if (this.started && a.status === "done" && was && was !== "done") this.finishes.push(now);
      this.seen[a.address] = { status: a.status, pal: A.look(a.address) };
    }
    this.finishes = this.finishes.filter((t) => now - t < PARTY_WINDOW);
    if (!this.party && this.finishes.length >= PARTY_FINISHES) {
      this.party = { start: now };
      this.finishes = [];
      cx.timer.after(PARTY_LENGTH + 50, (cx) => { this.party = null; cx.notify(); });
    }
    if (list.some((a) => a.status === "blocked") && now - this.incident > 60000) {
      this.incident = now;
      saveNumber("office.incident", now);
    }
  }

  // Starts a walk when a walker's spot changes; returns where it is (or is heading).
  // `start`: where a new walker appears (the door), instead of at its spot.
  walk(key, spot, x, y, speed, cx, start) {
    const now = Date.now();
    const fresh = !this.at[key] && Boolean(start);
    const at = this.at[key] ?? (start ? { ...start, spot: null } : null);
    if (!at) return (this.at[key] = { spot, x, y, fromX: x, until: 0, dur: 1 });
    if (at.spot === spot && at.x === x && at.y === y) return at;
    const dur = Math.max(400, Math.hypot(x - at.x, y - at.y) / speed);
    this.at[key] = { spot, x, y, fromX: at.x, until: now + dur, dur, face: at.face };
    if (spot === "door" || fresh) this.doorUntil = Math.max(this.doorUntil ?? 0, now + dur);
    cx.timer.after(dur + 30, (cx) => cx.notify());
    return this.at[key];
  }

  geometry(W, H, deskCount) {
    const P = A.P;
    const floorY = H - 30;
    const wallBottom = floorY - 14 * P;
    const slot = A.SLOT * P;
    const front = Math.min(deskCount, FRONT_ROW);
    const back = Math.max(0, deskCount - FRONT_ROW);
    const desksW = Math.max(front, back ? back + 0.5 : 0) * slot;
    const gap = 18 * P;
    const doorW = 16 * P;
    const kitchen = A.COUNTER.width * P + gap + A.COOLER.width * P + 22 * P + A.COUCH.width * P + 6 * P + A.PLANT.width * P;
    const room = doorW + 20 + A.PLANT.width * P + 10 + desksW + 50 + kitchen;
    const doorX = Math.max(16, Math.round((W - room) / 2));
    const plantL = doorX + doorW + 20;
    const x0 = plantL + A.PLANT.width * P + 10;
    const zoneX = x0 + desksW + 50;
    const coolerX = zoneX + A.COUNTER.width * P + gap;
    const couchX = coolerX + A.COOLER.width * P + 22 * P;
    const deskPos = (i) => i < FRONT_ROW
      ? { x: x0 + i * slot, y: floorY, back: false }
      : { x: x0 + (i - FRONT_ROW) * slot + slot / 2, y: floorY - 10 * P, back: true };
    return {
      P, floorY, wallBottom, slot, x0, deskPos, zoneX, coolerX, couchX, doorX, plantL,
      plantR: couchX + A.COUCH.width * P + 6 * P,
      door: { x: doorX + 2 * P, y: wallBottom + 2 * P },
    };
  }

  // Picks the cat's next stop: a nap on an empty keyboard, or a sit somewhere on the floor.
  wanderCat() {
    const g = this.g;
    if (!g) return false;
    const c = this.cat;
    if (c.goal === "sleep" && this.away.includes(c.desk) && Date.now() - c.since < 60000) return false;
    if (this.away.length && Math.random() < 0.45) {
      this.cat = { goal: "sleep", desk: this.away[Math.floor(Math.random() * this.away.length)], since: Date.now() };
    } else {
      const span = g.couchX + A.COUCH.width * g.P - g.x0;
      this.cat = { goal: "sit", desk: null, x: Math.round(g.x0 + Math.random() * span), since: Date.now() };
    }
    return true;
  }

  render(cx) {
    const { width: W, height: H } = this.size;
    const list = agents();
    // Agents on other machines are working from home: they're on the video call, not in a chair.
    const here = list.filter((a) => !remote(a.address));
    const all = terminals();
    const now = Date.now();
    setAgentOrder(list.map((a) => a.address));

    A.setScale(Math.max(3, Math.min(6, Math.floor(H / 60))));
    const deskCount = Math.max(MIN_DESKS, here.length, ...Object.values(this.desk).map((i) => i + 1));
    const g = (this.g = this.geometry(W, H, deskCount));
    this.seat(here, g.door, cx);
    this.track(list, now, cx);
    const P = g.P;
    const byDesk = {};
    for (const a of here) byDesk[this.desk[a.address]] = a;
    const tabs = {};
    for (const s of spaces()) for (const t of s.tabs ?? []) tabs[t.id] = t;
    const branches = {};
    for (const p of panes()) branches[p.pane] = p.branch;

    // The pizza: the courier walks in, leaves it on the counter, and walks out.
    const party = this.party;
    const counter = { x: g.zoneX + 2 * P, y: g.floorY };
    const courierTrip = party ? Math.max(400, Math.hypot(counter.x - g.door.x, counter.y - g.door.y) / WALK_SPEED) : 0;
    const pizzaHere = party && now >= party.start + courierTrip;

    // Where everyone goes. Long idlers take the couch while there's room; the rest
    // gather around the cooler (or the pizza), alternating sides, further out each pair.
    const idle = here.filter((a) => a.status === "idle").sort((a, b) => this.idleSince[a.address] - this.idleSince[b.address]);
    const nappers = pizzaHere ? [] : idle.filter((a) => now - this.idleSince[a.address] > NAP_AFTER).slice(0, COUCH_SEATS);
    const chatters = idle.filter((a) => !nappers.includes(a));
    const place = {};
    nappers.forEach((a, k) => { place[a.address] = { spot: "couch", x: g.couchX + (k * 10 - 1) * P, y: g.floorY, face: 1 }; });
    const hub = pizzaHere ? { left: g.zoneX - 12 * P, right: g.zoneX + A.COUNTER.width * P - 4 * P }
      : { left: g.coolerX - (A.PERSON - 3) * P, right: g.coolerX + A.COOLER.width * P - 2 * P };
    chatters.forEach((a, k) => {
      const left = k % 2 === 0;
      const ring = Math.floor(k / 2) * 9 * P;
      place[a.address] = { spot: pizzaHere ? "pizza" : "cooler", x: left ? hub.left - ring : hub.right + ring, y: g.floorY, face: left ? 1 : -1 };
    });
    for (const a of here) {
      const d = g.deskPos(this.desk[a.address]);
      place[a.address] ??= { spot: "desk", x: d.x, y: d.y, face: 1 };
    }
    this.away = here.filter((a) => place[a.address].spot !== "desk").map((a) => this.desk[a.address]);
    if (this.cat.goal === "sleep" && !this.away.includes(this.cat.desk)) this.wanderCat();

    const alarmed = here.filter((a) => a.status === "blocked" && now - (this.blockedSince[a.address] ?? now) > ALARM_AFTER);
    const people = here.map((a) => {
      const p = place[a.address];
      const start = this.started ? g.door : null;
      const at = this.walk(a.address, p.spot, p.x, p.y, WALK_SPEED, cx, start);
      return { y: p.y, el: this.person(a, at, g, p.face, tabs[a.tab], alarmed.includes(a)) };
    });
    const backPeople = people.filter((p) => p.y < g.floorY).map((p) => p.el);
    const frontPeople = people.filter((p) => p.y >= g.floorY).map((p) => p.el);
    const desks = Array.from({ length: deskCount }, (_, i) => {
      const pos = g.deskPos(i);
      const a = byDesk[i];
      return { back: pos.back, el: this.deskAt(i, pos, g, a, a && tabs[a.tab], a && branches[a.pane]) };
    });

    const hour = this.now.getHours();
    const dark = lightsDown(hour);
    const when = season(this.now);
    const hovered = here.find((a) => a.address === this.hovered);
    this.started = true;

    return div().id("office").size_full().relative().overflow_hidden().bg("#efe3cf")
      .on_size_change((size, cx) => { this.size = size; cx.notify(); })
      .children([
        ...this.room(W, H, g, sky(hour), list, all, now, when),
        ...desks.filter((d) => d.back).map((d) => d.el),
        ...backPeople,
        ...desks.filter((d) => !d.back).map((d) => d.el),
        this.prop(A.COUNTER, g.zoneX, g),
        ...(pizzaHere ? [this.prop({ width: A.COUNTER.width, rects: A.PIZZA_BOX }, g.zoneX, g)] : []),
        this.prop(A.COOLER, g.coolerX, g),
        this.prop(A.COUCH, g.couchX, g),
        this.prop(A.PLANT, g.plantR, g),
        when === "winter" ? this.prop(A.TREE, g.plantL - 3 * P, g) : this.prop(A.PLANT, g.plantL, g),
        ...(when === "halloween" ? [this.prop(A.PUMPKIN, g.plantL + 10 * P, g), this.prop(A.PUMPKIN, g.plantR - 9 * P, g),
          ...(pizzaHere ? [] : [this.prop(A.PUMPKIN, g.zoneX + 13 * P, g, 11 * P)])] : []),
        ...here.flatMap((a) => this.interns(a, descendants(all, a.address), g)),
        ...this.graduates(g, now, cx),
        ...frontPeople,
        ...this.leavers(g, now),
        ...(party ? [this.courier(party, counter, courierTrip, g, now, cx)] : []),
        this.kitty(g, cx),
        ...(dark ? this.night(W, H, g, byDesk, deskCount, when) : []),
        ...(alarmed.length ? D.alarm(W, H, g.deskPos(this.desk[alarmed[0].address]).x + 27 * P) : []),
        ...(hovered && this.at[hovered.address] ? [this.thought(hovered, this.at[hovered.address], g)] : []),
        ...(list.length ? [] : [this.sign(W, g)]),
      ]);
  }

  room(W, H, g, s, list, all, now, when) {
    const { P, wallBottom, zoneX } = g;
    const els = [
      div().absolute().left(0).top(wallBottom - 9 * P).w(W).h(9 * P).bg("#d8c3a2"),
      div().absolute().left(0).top(wallBottom - 9 * P - 2).w(W).h(2).bg("#c4ab86"),
      div().absolute().left(0).top(wallBottom - 4).w(W).h(4).bg("#7a5c3e"),
      div().absolute().left(0).top(wallBottom).w(W).h(H - wallBottom).bg("#b9895a"),
    ];
    for (let y = wallBottom + 12; y < H; y += 14) els.push(div().absolute().left(0).top(y).w(W).h(1).bg("#a5784d"));
    for (let i = 0, y = wallBottom; y < H; i++, y += 14)
      for (let x = (i % 2) * 60 + 20; x < W; x += 120) els.push(div().absolute().left(x).top(y).w(1).h(13).bg("#a5784d"));

    // Along the wall: the incident sign, Employee of the Month and the remote call take
    // the first spots; windows and posters fill the rest, clear of the door and kitchen.
    const winH = Math.max(36, Math.min(110, wallBottom - 9 * P - 40));
    const weather = this.weather ?? (when === "winter" ? "snow" : null);
    const turns = new Map(all.map((t) => [t.address, t.turn ?? 0]));
    const star = [...list].sort((a, b) => (turns.get(b.address) ?? 0) - (turns.get(a.address) ?? 0))[0];
    const remotes = list.filter((a) => remote(a.address)).map((a) => ({
      address: a.address, pane: a.pane, pal: A.look(a.address), machine: a.address.split("/")[0],
      name: a.space_name, status: a.status, title: a.title || a.tab_name || a.space_name,
    }));
    const blockedNow = list.some((a) => a.status === "blocked");
    const specials = [
      (x) => D.incidentSign(x + 12, 16, winH, Math.floor((now - this.incident) / DAY), blockedNow),
      (x) => D.portrait(x + 18, 10, winH + 6, star && (turns.get(star.address) ?? 0) > 0
        ? { pal: A.look(star.address), name: star.space_name, remote: remote(star.address) } : null),
      ...(remotes.length ? [(x) => D.videoWall(x + 4, 12, winH + 4, remotes)] : []),
    ];
    const busy = [[g.doorX - 40, g.doorX + 16 * P + 30], [zoneX - 90, zoneX + 170]];
    for (let x = 40, k = 0; x + 120 < W - 20; x += 220) {
      if (busy.some(([a, b]) => x + 120 > a && x < b)) continue;
      if (k % 2 === 1 && specials.length) els.push(specials.shift()(x));
      else els.push(k % 5 === 4 ? D.poster(x + 25, 18, winH, POSTERS[Math.floor(k / 5) % POSTERS.length]) : D.windowPane(x, 16, 120, winH, s, weather));
      k++;
    }
    els.push(D.door(g.doorX, wallBottom, P, now < (this.doorUntil ?? 0)));
    els.push(D.clock(zoneX - 70, 20, clock(this.now)));
    const count = (st) => list.filter((a) => a.status === st).length;
    const napping = list.filter((a) => this.at[a.address]?.spot === "couch").length;
    const interns = list.reduce((n, a) => n + descendants(all, a.address).length, 0);
    els.push(D.whiteboard(zoneX, 14, [
      ["#3d8be0", `WORKING  ${count("working")}`],
      ["#58a368", `DONE     ${count("done")}`],
      ["#e05252", `BLOCKED  ${count("blocked")}`],
      ["#8a8f99", `ON BREAK ${count("idle") - napping}`],
      ["#6b5fb5", `NAPPING  ${napping}`, napping > 0],
      ["#c0643f", `INTERNS  ${interns}`, interns > 0],
      ["#d94a3a", "PIZZA!!", Boolean(this.party)],
    ]));
    if (when === "winter" || when === "newyear") els.push(...D.stringLights(W, 2));
    if (when === "newyear") els.push(D.banner(W, wallBottom - 9 * P - 30, "HAPPY NEW YEAR", "#9b6bdc"));
    if (when === "valentine") els.push(D.banner(W, wallBottom - 9 * P - 30, "♥  HAPPY VALENTINE'S  ♥", "#e05c8c"));
    return els;
  }

  sign(W, g) {
    return div().absolute().left(W / 2 - 140).top(g.wallBottom - 34).w(280).py(4).rounded(4).bg("#00000033")
      .text_size(11).text_color("#fffaf0").flex().justify_center().child("Nobody's in yet. Start an agent and they'll show up.");
  }

  prop(art, x, g, lift = 0) {
    return div().absolute().left(x).top(g.floorY - A.BOX * g.P - lift).w(art.width * g.P).h(A.BOX * g.P).children(A.rects(art.rects, A.PALETTE));
  }

  // A desk with its chair, monitor, bell and lamp, the branch on a sticky note, and the
  // nameplate of whoever sits there.
  deskAt(i, pos, g, a, tab, branch) {
    const P = g.P;
    const st = a?.status;
    const away = Boolean(a) && this.at[a.address]?.spot !== "desk";
    const pal = a ? { ...A.PALETTE, mug: mugOf(a) } : A.PALETTE;
    const done = st === "done" && !away;
    const overtime = st === "working" && tab?.working_since && Date.now() - tab.working_since > MUGS_AFTER;
    const plate = a ? this.plate(a) : null;
    return div().id(`desk-${i}`).absolute().left(pos.x).top(pos.y - A.BOX * P).w(A.SLOT * P).h(A.BOX * P + 28)
      .when(Boolean(a), (el) => el.cursor_pointer().on_click(() => run("pane.focus", { pane: a.pane })))
      .children([
        ...A.rects(A.DESK.chair, pal), ...A.rects(A.DESK.desk, pal),
        ...(overtime ? A.rects(A.MUG_PILE, pal) : []),
        this.screen(away ? "idle" : st, away, P),
        ...(branch ? [this.sticky(branch, P)] : []),
        A.layer(A.DESK.bell, pal, A.SLOT).when(done, (el) => el.loop("left", { from: -1.5, to: 1.5, period: 160 })),
        ...(done ? [this.ding(P), ...this.confetti(P, a.address)] : []),
        ...(plate ? [plate.absolute().left(6 * P).top(pos.back ? (A.BOX - 29) * P : A.BOX * P + 6)] : []),
      ]);
  }

  screen(st, screensaver, P) {
    const [sx, sy, sw, sh] = A.DESK.screen;
    const bg = { blocked: "#4a1a1e", done: "#143a24", working: "#13263f" }[st] ?? "#0b1220";
    return div().absolute().left(sx * P).top((A.BOX + sy) * P).w(sw * P).h(sh * P).bg(bg).overflow_hidden()
      .when(st === "working", (el) => el.children(CODE.map((c, k) =>
        div().absolute().left(P * (k % 2 === 0 ? 1 : 2)).top(P * (1 + 2 * k) - 1).h(P - 1).bg(c)
          .loop("width", { from: P, to: P * (8 - (k % 2)), period: 700 + k * 260, easing: "linear" }))))
      .when(st === "blocked", (el) => el.flex().items_center().justify_center().text_size(14).font_bold().text_color("#ff6b6b")
        .child("!").loop("opacity", { from: 0.4, to: 1, period: 700 }))
      .when(st === "done", (el) => el.flex().items_center().justify_center().text_size(13).font_bold().text_color("#6be38b").child("✓"))
      .when(screensaver, (el) => el.child(div().absolute().top(P * 3).h(P).w(P).bg("#3d8be0")
        .loop("left", { from: P, to: P * 8, period: 5000 })));
  }

  sticky(branch, P) {
    return div().absolute().left(18 * P).top((A.BOX - 26) * P).w(10 * P).h(5 * P).px(2).bg("#f6e27a")
      .border_1().border_color("#d8bf4a").flex().items_center().overflow_hidden()
      .text_size(7).text_color("#4a3a10").child(div().truncate().child(branch));
  }

  ding(P) {
    return div().absolute().left(15 * P).top((A.BOX - 21) * P).text_size(10).font_bold().text_color("#b8860b")
      .child("ding!").loop("opacity", { from: 0, to: 1, period: 640 });
  }

  // Paper falling over the desk, for as long as the news is unread.
  confetti(P, seed) {
    let h = 0;
    for (const ch of seed) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return Array.from({ length: 16 }, (_, k) => {
      const r = ((h >>> (k % 24)) ^ Math.imul(k + 1, 2654435761)) >>> 0;
      const x = (4 + (r % 34)) * P;
      return div().absolute().left(x).w(P).h(k % 3 === 0 ? P * 2 : P).bg(CONFETTI[k % CONFETTI.length])
        .loop("top", { from: (A.BOX - 30) * P - (k % 5) * 8, to: (A.BOX - 1) * P, period: 1100 + ((r >>> 3) % 900), easing: "linear" })
        .loop("left", { from: x - 4, to: x + 4, period: 500 + ((r >>> 5) % 400) });
    });
  }

  spaceTile(id) {
    const icon = (shared("space.icons") ?? {})[id];
    const c = (shared("space.colors") ?? {})[id] ?? "ansi6";
    const fill = c.startsWith("#") ? c : MARK[c] ?? "#22a99a";
    return div().size(12).flex_none().rounded(3).bg(fill).flex().items_center().justify_center().text_size(8).text_color("#ffffff")
      .when(Boolean(icon?.svg), (el) => el.child(svg(icon.svg).size(8)))
      .when(Boolean(icon?.text), (el) => el.child(icon.text));
  }

  plate(a) {
    const focused = a.focused;
    return div().max_w((A.SLOT - 8) * A.P).h(16).px(5).gap(4).rounded(4).flex().items_center()
      .bg(focused ? "#fff6dd" : "#00000040").text_size(10).text_color(focused ? "#3b2a1a" : "#fffaf0")
      .child(div().size(6).flex_none().rounded_full().bg(STATUS[a.status] ?? STATUS.idle))
      .child(this.spaceTile(a.space))
      .when(remote(a.address), (el) => el.child(div().flex_none().text_size(9).text_color("#fffaf0").child("🎧")))
      .child(div().truncate().child(a.space_name + (a.tab_name ? ` · ${a.tab_name}` : "")));
  }

  // Sub-agents: small people on stools in front of their parent's desk, laptops out.
  interns(a, kids, g) {
    if (!kids.length) return [];
    const P = g.P;
    const p = Math.max(2, Math.round(P * 0.7));
    const pos = g.deskPos(this.desk[a.address]);
    const x0 = pos.x + 20 * P;
    const shown = kids.slice(0, 3);
    const els = shown.map((k, i) => {
      const pal = A.look(k.address);
      const x = x0 + i * 8 * P;
      this.kids[k.address] = { x, y: pos.y, p, pal };
      const busy = k.status === "working";
      const stuck = k.status === "blocked" || k.status === "error";
      return div().id(`intern-${k.address}`).absolute().left(x).top(pos.y - A.BOX * p).w(A.PERSON * p).h(A.BOX * p)
        .cursor_pointer().on_click(() => run("terminal.peek", { terminal: k.address }))
        .children([
          this.part(A.SIT.stool, pal, -1, p), this.part(A.SIT.body, pal, -1, p),
          ...(busy ? [this.part(A.SIT.laptop, pal, -1, p), this.part(A.SIT.lapArm, pal, -1, p).loop("top", { from: 0, to: -1.5, period: 260 + i * 40 })]
            : [this.part(A.SIT.rest, pal, -1, p)]),
          ...(stuck ? [this.bubble((A.BOX - 24) * p - 18, "#e05252", p).child("!").loop("opacity", { from: 0.5, to: 1, period: 700 })] : []),
        ]);
    });
    if (kids.length > shown.length)
      els.push(div().absolute().left(x0 + 3 * 8 * P).top(pos.y - 14 * P).px(4).rounded(4).bg("#00000055")
        .text_size(10).text_color("#fffaf0").child(`+${kids.length - shown.length}`));
    return els;
  }

  // Interns that finished: a bow where they sat, then out the door.
  graduates(g, now, cx) {
    const live = new Set(terminals().filter((t) => t.state === "live").map((t) => t.address));
    for (const [k, info] of Object.entries(this.kids)) {
      if (live.has(k)) continue;
      delete this.kids[k];
      if (!this.started) continue;
      this.grads[k] = { ...info, start: now };
      cx.timer.after(GRAD_BOW + 20, (cx) => cx.notify());
    }
    return Object.entries(this.grads).flatMap(([k, gr]) => {
      const box = (el, x, y) => div().id(`grad-${k}`).absolute().left(x).top(y - A.BOX * gr.p).w(A.PERSON * gr.p).h(A.BOX * gr.p).child(el);
      if (now < gr.start + GRAD_BOW) {
        const body = this.pose(gr.pal, -1, [A.STAND.backLeg, A.STAND.frontLeg, A.STAND.arm], gr.p)
          .child(this.part(A.STAND.body, gr.pal, -1, gr.p).left(-gr.p).loop("top", { from: 0, to: 2 * gr.p, period: 900 }));
        return [box(body, gr.x, gr.y).child(this.bubble((A.BOX - 20) * gr.p - 18, "#3b3b44", gr.p).child("🎓 thx!"))];
      }
      const at = this.walk(`grad:${k}`, "door", g.door.x, g.door.y, WALK_SPEED, cx, { x: gr.x, y: gr.y });
      if (now >= at.until && at.until > 0) { delete this.grads[k]; delete this.at[`grad:${k}`]; return []; }
      return [box(this.walker(gr.pal, -1, [], gr.p), at.x, at.y)
        .transition("left", { duration: at.dur, easing: "ease-in-out" }).transition("top", { duration: at.dur, easing: "ease-in-out" })];
    });
  }

  // Closed agents carrying their box out.
  leavers(g, now) {
    return Object.entries(this.leaving).flatMap(([k, pal]) => {
      const at = this.at[`leave:${k}`];
      if (!at || now >= at.until) { delete this.leaving[k]; delete this.at[`leave:${k}`]; return []; }
      return [div().id(`leave-${k}`).absolute().left(at.x).top(at.y - A.BOX * g.P).w(A.PERSON * g.P).h(A.BOX * g.P)
        .transition("left", { duration: at.dur, easing: "ease-in-out" }).transition("top", { duration: at.dur, easing: "ease-in-out" })
        .child(this.walker(pal, Math.sign(at.x - at.fromX) || -1, [A.STAND.box], g.P, false))];
    });
  }

  courier(party, counter, trip, g, now, cx) {
    const stay = 2500;
    const leaving = now >= party.start + trip + stay;
    if (now >= party.start + 2 * trip + stay + 100) return div();
    if (!leaving) cx.timer.after(party.start + trip + stay - now + 20, (cx) => cx.notify());
    const goal = leaving ? g.door : counter;
    const at = this.walk("courier", leaving ? "door" : "counter", goal.x, goal.y, WALK_SPEED, cx, g.door);
    const walking = now < at.until;
    const face = walking ? Math.sign(at.x - at.fromX) || 1 : 1;
    const body = walking ? this.walker(A.COURIER, face, leaving ? [] : [A.STAND.pizza], g.P, leaving)
      : this.pose(A.COURIER, 1, [A.STAND.backLeg, A.STAND.frontLeg, A.STAND.body, A.STAND.armUp]);
    return div().id("courier").absolute().left(at.x).top(at.y - A.BOX * g.P).w(A.PERSON * g.P).h(A.BOX * g.P)
      .transition("left", { duration: at.dur, easing: "ease-in-out" }).transition("top", { duration: at.dur, easing: "ease-in-out" })
      .child(body).when(!walking, (el) => el.child(this.bubble((A.BOX - 20) * g.P - 20, "#d94a3a").child("pizza! 🍕")));
  }

  person(a, at, g, placeFace, tab, alarmed) {
    const P = g.P;
    const napping = at.spot === "couch";
    const base = A.look(a.address);
    const pal = napping ? { ...base, eye: "#00000000" } : base;
    const away = remote(a.address);
    const walking = Date.now() < at.until;
    const face = walking ? Math.sign(at.x - at.fromX) || 1 : placeFace;
    at.face = face;
    const st = a.status;
    const hs = away ? [A.HEADSET.stand] : [];
    let body, headTop;
    if (walking) [body, headTop] = [this.walker(pal, face, hs), 20];
    else if (at.spot === "cooler" || at.spot === "pizza") {
      body = this.pose(pal, face, [A.STAND.backLeg, A.STAND.frontLeg, A.STAND.body, ...hs, A.STAND.cup])
        .loop("top", { from: 0, to: -1, period: 2600 + (pal.salt % 900) });
      headTop = 20;
    } else if (napping) {
      body = this.pose(pal, 1, [A.SIT.body, ...(away ? [A.HEADSET.sit] : []), A.SIT.rest]).loop("top", { from: 0, to: 1.5, period: 3400 + (pal.salt % 700) });
      headTop = 24;
    } else if (st === "working") {
      const sweaty = tab?.working_since && Date.now() - tab.working_since > SWEAT_AFTER;
      body = div().absolute().size_full().children([
        this.part(A.SIT.body, pal, 1),
        ...(away ? [this.part(A.HEADSET.sit, pal, 1)] : []),
        this.part(A.SIT.arm, pal, 1).loop("top", { from: 0, to: -2, period: 240 + (pal.salt % 120) }),
        ...(sweaty ? [0, 1].map((k) => div().absolute().left((k ? 1 : 9) * P).w(P).h(P * 1.5).rounded(P).bg("#7fc4f0")
          .loop("top", { from: (A.BOX - 23 + k) * P, to: (A.BOX - 18 + k) * P, period: 900 + k * 300, easing: "ease-in" })) : []),
      ]);
      headTop = 24;
    } else if (alarmed) {
      body = this.pose(pal, 1, [A.STAND.backLeg, A.STAND.frontLeg, A.STAND.body, ...hs])
        .child(this.part(A.STAND.armUp, pal, 1).loop("top", { from: 0, to: -P, period: 260 }))
        .child(this.part(A.STAND.armUpBack, pal, 1).loop("top", { from: -P, to: 0, period: 260 }))
        .loop("top", { from: 0, to: -4, period: 420 });
      headTop = 21;
    } else {
      // blocked: one hand up, bouncing; done: both hands up, hopping
      const cheer = st === "done";
      body = this.pose(pal, 1, [A.STAND.backLeg, A.STAND.frontLeg, A.STAND.body, ...hs, A.STAND.armUp, cheer ? A.STAND.armUpBack : A.STAND.arm])
        .loop("top", { from: 0, to: cheer ? -6 : -3, period: cheer ? 560 : 900 });
      headTop = 21;
    }

    const top = (A.BOX - headTop) * P - 20;
    let bubble = null;
    if (walking) bubble = null;
    else if (at.spot === "pizza") bubble = this.bubble(top, "#d94a3a").frames(rotate(PIZZA_CHAT, pal.salt % PIZZA_CHAT.length), 1800 + (pal.salt % 900));
    else if (at.spot === "cooler") bubble = this.bubble(top, "#3b3b44").frames(rotate(CHAT, pal.salt % CHAT.length), 2200 + (pal.salt % 1500));
    else if (napping) bubble = this.bubble(top, "#6b5fb5").frames(["z", "zZ", "zZz", "zZ"], 900);
    else if (alarmed) bubble = this.bubble(top, "#e05252").child("HELP!!").loop("opacity", { from: 0.5, to: 1, period: 400 });
    else if (st === "blocked") bubble = this.bubble(top, "#e05252").child("!").loop("opacity", { from: 0.5, to: 1, period: 700 });
    else if (st === "done") bubble = this.bubble(top, "#2f9a50").child("✓");
    at.headTop = headTop;

    return div().id(`person-${a.address}`).absolute().left(at.x).top(at.y - A.BOX * P).w(A.PERSON * P).h(A.BOX * P)
      .transition("left", { duration: at.dur, easing: "ease-in-out" })
      .transition("top", { duration: at.dur, easing: "ease-in-out" })
      .cursor_pointer().on_click(() => run("pane.focus", { pane: a.pane }))
      .on_hover((h, cx) => {
        if (h) this.hovered = a.address;
        else if (this.hovered === a.address) this.hovered = null;
        cx.notify();
      })
      .children([body, ...(bubble ? [bubble] : [])]);
  }

  // What they're working on, in a thought cloud over their head.
  thought(a, at, g) {
    const P = g.P;
    const text = (a.title || a.tab_name || a.program?.name || "thinking…").replace(/\s+/g, " ").trim();
    const x = at.x + 8 * P;
    const y = at.y - A.BOX * P + (A.BOX - (at.headTop ?? 22)) * P;
    return div().absolute().left(Math.max(4, x - 20)).top(Math.max(2, y - 54))
      .child(div().max_w(260).px(8).py(4).rounded(10).bg("#ffffff").border_1().border_color("#3b3b44")
        .text_size(10).text_color("#3b3b44").child(div().truncate().child(`💭 ${text}`)))
      .child(div().absolute().left(16).top(26).size(7).rounded_full().bg("#ffffff").border_1().border_color("#3b3b44"))
      .child(div().absolute().left(22).top(35).size(4).rounded_full().bg("#ffffff").border_1().border_color("#3b3b44"));
  }

  part(list, pal, face, p = A.P) {
    return A.layer(face < 0 ? A.mirror(list, A.PERSON) : list, pal, A.PERSON, p);
  }

  pose(pal, face, parts, p = A.P) {
    return div().absolute().size_full().children(parts.map((l) => this.part(l, pal, face, p)));
  }

  // Walking: legs swinging, a bob. `extra` parts ride along (a headset, a box); `swing`
  // false keeps the arms on whatever they're carrying.
  walker(pal, face, extra = [], p = A.P, swing = true) {
    const carry = extra.some((l) => l === A.STAND.box || l === A.STAND.pizza);
    return div().absolute().size_full().loop("top", { from: 0, to: -2, period: 320 }).children([
      this.part(A.STAND.backLeg, pal, face, p).loop("left", { from: -p, to: p, period: 520, easing: "linear" }),
      this.part(A.STAND.body, pal, face, p),
      ...extra.map((l) => this.part(l, pal, face, p)),
      this.part(A.STAND.frontLeg, pal, face, p).loop("left", { from: p, to: -p, period: 520, easing: "linear" }),
      ...(carry ? [] : [this.part(A.STAND.arm, pal, face, p).when(swing, (el) => el.loop("left", { from: -2, to: 2, period: 520 }))]),
    ]);
  }

  kitty(g, cx) {
    const P = g.P;
    const c = this.cat;
    const onDesk = c.goal === "sleep";
    const pos = onDesk ? g.deskPos(c.desk) : null;
    const x = onDesk ? pos.x + 8 * P : (c.x ?? g.couchX - 12 * P);
    const y = onDesk ? pos.y - 12 * P : g.floorY;
    const at = this.walk("cat", `${c.goal}-${c.desk}-${c.since}`, x, y, CAT_SPEED, cx);
    const walking = Date.now() < at.until;
    if (walking) at.face = Math.sign(at.x - at.fromX) || 1;
    const face = at.face ?? 1;
    const part = (list) => A.layer(face < 0 ? A.mirror(list, A.CAT) : list, A.PALETTE, A.CAT);
    let body;
    if (walking) body = div().absolute().size_full().children([
      part(A.KITTY.legsA).loop("left", { from: -1, to: 1.5, period: 360, easing: "linear" }),
      part(A.KITTY.legsB).loop("left", { from: 1.5, to: -1, period: 360, easing: "linear" }),
      part(A.KITTY.body).loop("top", { from: 0, to: -1, period: 360 }),
    ]);
    else if (onDesk) body = div().absolute().size_full().children([
      part(A.KITTY.sleep).loop("top", { from: 0, to: 0.8, period: 2400 }),
      div().absolute().left(9 * P).top((A.BOX - 10) * P).text_size(9).font_bold().text_color("#6b5fb5").frames(["z", "zz", "zzz", ""], 800),
    ]);
    else body = part(A.KITTY.sit);
    return div().id("cat").absolute().left(at.x).top(at.y - A.BOX * P).w(A.CAT * P).h(A.BOX * P)
      .transition("left", { duration: at.dur, easing: "ease-in-out" })
      .transition("top", { duration: at.dur, easing: "ease-in-out" })
      .child(body);
  }

  // After dark: the room dims, lamps light the occupied desks, and busy screens glow.
  night(W, H, g, byDesk, deskCount, when) {
    const P = g.P;
    const [bx, by, bw, bh] = A.DESK.bulb;
    const [sx, sy] = A.DESK.screen;
    const els = [div().absolute().left(0).top(0).w(W).h(H).bg("#0b1030").opacity(0.5)];
    if (when === "winter" || when === "newyear") els.push(...D.stringLights(W, 2));
    for (let i = 0; i < deskCount; i++) {
      const a = byDesk[i];
      if (!a || this.at[a.address]?.spot !== "desk") continue;
      const pos = g.deskPos(i);
      const top = pos.y - A.BOX * P;
      els.push(div().absolute().left(pos.x + (bx - 5) * P).top(top + (A.BOX + by) * P).size(14 * P).rounded_full().bg("#ffd27a").opacity(0.18));
      els.push(div().absolute().left(pos.x + bx * P).top(top + (A.BOX + by) * P).w(bw * P).h(bh * P).bg("#fff2b0"));
      if (a.status !== "idle") {
        els.push(div().absolute().left(pos.x + (sx - 3) * P).top(top + (A.BOX + sy - 3) * P).w(16 * P).h(14 * P).rounded(6 * P)
          .bg({ working: "#7fb6ff", blocked: "#ff6b6b", done: "#6be38b" }[a.status]).opacity(0.16));
        els.push(div().absolute().left(pos.x).top(top).w(A.SLOT * P).h(A.BOX * P).child(this.screen(a.status, false, P)));
      }
    }
    return els;
  }

  bubble(top, color, p = A.P) {
    return div().absolute().left((A.PERSON * p) / 2 - 4).top(top).px(5).h(16).flex().items_center().rounded(6)
      .bg("#fffdf7").border_1().border_color("#3b3b44").text_size(10).font_bold().text_color(color).whitespace_nowrap();
  }
}
