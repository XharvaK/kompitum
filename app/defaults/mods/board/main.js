// Every worker from both orchestrators: what waits, the queue, and the rows'
// actions. Reading is kom status --json; actions are kom task ....

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { kom, theme, notify, playSound, openOverlay, closeOverlay, props, nextCommand, settings } from "kompitum";

// The two orchestrators keep these colors in the Line panel too.
const OPUS = "accent";
const MANAGER = "ansi5";
const OWNER = "done";
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n) => String(n).padStart(2, "0");

const cap = (s) => (s ? String(s).replace(/^./, (c) => c.toUpperCase()) : "");
const text = (v) => (typeof v === "string" ? v : "");
const base = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";
const count = (v, fallback) => (v == null || v === "" || !Number.isFinite(Number(v)) ? fallback : Number(v));

function clock(d) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

// Text runs in a div do not break on newlines; one child per line does.
function lines(value) {
  return String(value ?? "").replace(/\r\n/g, "\n").split("\n").map((line) => div().w_full().child(line || " "));
}

// Today says 09:10; another day says Nov 4 18:26.
function stamp(value) {
  const d = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(d.getTime())) return String(value ?? "");
  const now = new Date();
  const today = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  return today ? clock(d) : `${MONTHS[d.getMonth()]} ${d.getDate()} ${clock(d)}`;
}

function duration(seconds) {
  if (seconds == null || seconds === "" || !Number.isFinite(Number(seconds))) return "";
  const m = Math.floor(Math.max(0, Number(seconds)) / 60);
  if (m < 1) return "<1m";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60), rest = m % 60;
  if (h < 24) return rest ? `${h}h ${rest}m` : `${h}h`;
  const d = Math.floor(h / 24), hr = h % 24;
  return hr ? `${d}d ${hr}h` : `${d}d`;
}

function money(value) {
  if (value == null || value === "") return "";
  const n = Number(value);
  if (!Number.isFinite(n)) return "";
  // A run of a tenth of a cent is not free: say it is under a cent.
  return n > 0 && n < 0.005 ? "<$0.01" : `$${n.toFixed(2)}`;
}

// Piped, kom says what went wrong as JSON: {"error": {"message", "hint"}}; show its words.
function errorText(result) {
  for (const out of [result?.stderr, result?.stdout]) {
    try {
      const message = JSON.parse(String(out ?? "")).error?.message;
      if (message) return firstLine(message);
    } catch {}
  }
  return firstLine(result?.stderr) || firstLine(result?.stdout) || `kom exited ${result?.code ?? "?"}`;
}

function tasksOf(tasks, key) {
  const list = tasks && typeof tasks === "object" ? tasks[key] : null;
  return Array.isArray(list)
    ? list.filter((t) => t && typeof t === "object" && typeof t.name === "string")
    : [];
}

// Whatever kom workers --json gives: an array, or an object holding one.
function workersOf(workers) {
  const list = Array.isArray(workers) ? workers : Array.isArray(workers?.workers) ? workers.workers : [];
  return list.filter((w) => w && typeof w === "object"
    && typeof (w.label ?? w.name) === "string"
    && typeof (w.status ?? w.state) === "string")
    .map((w) => ({ label: w.label ?? w.name, status: w.status ?? w.state, until: w.until ?? w.until_ms }));
}

function untilText(value) {
  if (value == null || value === "") return "";
  if (typeof value === "string") {
    const d = new Date(value);
    return Number.isNaN(d.getTime()) ? value : stamp(d);
  }
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "";
  return stamp(new Date(n > 1e12 ? n : n * 1000));
}

export default class Board extends View {
  init(_props, cx) {
    const p = props() ?? {};
    this.mode = p.review ? "review" : p.titlebar ? "titlebar" : "panel";
    this.data = null;
    this.error = null;
    this.loading = false;
    this.loaded = false;
    this.seenReview = new Set();
    this.hovered = null;
    this.confirm = null;
    this.accepting = null;
    this.handle = cx.focus_handle();
    if (this.mode !== "panel") return;
    this.refresh(cx);
    cx.timer.every(3000, (cx) => this.refresh(cx));
    // A toast can open a review: notify's action names this mod command.
    cx.spawn(async (cx) => {
      for (;;) {
        let command;
        try {
          command = await nextCommand();
        } catch {
          return;
        }
        if (typeof command !== "string" || !command.startsWith("openReview:")) continue;
        let name = command.slice("openReview:".length);
        try {
          name = decodeURIComponent(name);
        } catch {}
        if (name) this.review(name, cx);
      }
    });
  }

  refresh(cx) {
    if (this.loading) return;
    this.loading = true;
    cx.spawn(async (cx) => {
      try {
        const result = await kom(["status", "--json"]);
        if (result.code !== 0) {
          this.error = errorText(result);
        } else {
          let data;
          try {
            data = JSON.parse(result.stdout);
          } catch (e) {
            throw new Error(firstLine(e?.message ?? e) || "status did not return JSON");
          }
          if (!data || typeof data !== "object") throw new Error("status did not return an object");
          this.data = data;
          this.error = null;
          this.noteReviews(data);
        }
      } catch (e) {
        this.error = firstLine(e?.message ?? e) || "status failed";
      } finally {
        this.loading = false;
        cx.notify();
      }
    });
  }

  noteReviews(data) {
    const tasks = data.tasks && typeof data.tasks === "object" ? data.tasks : {};
    const names = tasksOf(tasks, "review").map((t) => t.name);
    const now = new Set(names);
    if (this.loaded) {
      let fresh = false;
      for (const name of names) {
        if (this.seenReview.has(name)) continue;
        fresh = true;
        notify({
          title: `${name} is ready for review`,
          action: { name: `board.openReview:${encodeURIComponent(name)}` },
        });
      }
      if (fresh) playSound("done");
    }
    this.seenReview = now;
    this.loaded = true;
  }

  fail(what, e) {
    notify({ title: `${what} failed`, body: firstLine(e?.message ?? e), level: "error" });
  }

  review(name, cx) {
    cx.spawn(async () => {
      try {
        const result = await kom(["task", "review", name]);
        if (result.code !== 0) throw new Error(errorText(result));
        const body = text(result.stdout).trim() || "(no review text)";
        openOverlay({
          mod: "board",
          kind: "window",
          key: `board-review:${name}`,
          width: 760,
          height: 560,
          props: { review: { title: `Review: ${name}`, body } },
        });
      } catch (e) {
        this.fail(`Review ${name}`, e);
      }
    });
  }

  openAccept(name, cx) {
    const input = InputState.new({ placeholder: "Review note, Enter to accept" });
    input.on("submit", (_e, cx) => this.accept(name, cx));
    this.accepting = { name, input };
    input.focus();
    cx.timer.after(50, () => input.focus());
    cx.notify();
  }

  accept(name, cx) {
    const input = this.accepting?.name === name ? this.accepting.input : null;
    if (!input) return;
    const line = input.value();
    this.accepting = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const result = await kom(["task", "accept", name, "--review", line]);
        if (result.code !== 0) throw new Error(errorText(result));
        this.refresh(cx);
      } catch (e) {
        this.fail(`Accept ${name}`, e);
      }
    });
  }

  ask(key, action, cx) {
    const asked = { key, action };
    this.confirm = asked;
    cx.notify();
    cx.timer.after(4000, (cx) => {
      if (this.confirm !== asked) return;
      this.confirm = null;
      cx.notify();
    });
  }

  runTask(action, name, cx) {
    this.confirm = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const result = await kom(["task", action, name]);
        if (result.code !== 0) throw new Error(errorText(result));
        this.refresh(cx);
      } catch (e) {
        this.fail(`${cap(action)} ${name}`, e);
      }
    });
  }

  watch(name) {
    openOverlay({
      kind: "window",
      key: `board-watch:${name}`,
      width: 900,
      height: 600,
      layout: {
        type: "column",
        children: [
          { type: "block", size: { px: 30 }, content: { mod: "board", props: { titlebar: `Watch: ${name}` } } },
          { type: "block", content: { terminal: name } },
        ],
      },
    });
  }

  titlebar(t, label) {
    return h_flex()
      .h(30)
      .w_full()
      .flex_none()
      .px(10)
      .gap(8)
      .items_center()
      .bg(t.surface)
      .border_b(1)
      .border_color(t.border)
      .text_size(12)
      .child(div().flex_1().min_w(0).truncate().font_semibold().text_color(t.foreground).child(label || ""))
      .child(div()
        .id("board-close")
        .px(6)
        .py(2)
        .rounded(5)
        .text_color(t.muted)
        .cursor_pointer()
        .tooltip("Close")
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .on_click(() => closeOverlay(props().overlay))
        .child(svg("mods/workspace-chrome/x.svg").size(12)));
  }

  chip(t, label, color) {
    return div()
      .px(5)
      .h(16)
      .flex_none()
      .flex()
      .items_center()
      .rounded(4)
      .bg(color + "1f")
      .text_size(10)
      .font_medium()
      .text_color(color)
      .child(label);
  }

  byLabel(by) {
    if (by === "opus") return "Opus";
    if (by === "manager") return "Manager";
    if (by === "owner") return "You";
    return typeof by === "string" ? cap(by) : "";
  }

  byColor(t, by) {
    if (by === "opus") return t[OPUS];
    if (by === "manager") return t[MANAGER];
    if (by === "owner") return t[OWNER];
    return t.muted;
  }

  action(t, id, label, primary, onClick) {
    return div()
      .id(id)
      .h(20)
      .px(6)
      .flex_none()
      .flex()
      .items_center()
      .rounded(5)
      .bg(primary ? t.solid : t.raised)
      .text_color(primary ? t.solid_foreground : t.foreground)
      .text_size(10.5)
      .font_medium()
      .cursor_pointer()
      .hover((s) => s.opacity(0.85))
      .on_click(onClick)
      .child(label);
  }

  row(t, task, kind, cx) {
    const key = `${kind}:${task.name}`;
    const hovered = this.hovered === key;
    const accepting = this.accepting?.name === task.name;
    const show = kind === "review" || hovered || accepting;
    const asked = this.confirm?.key === key ? this.confirm.action : null;
    const repo = base(task.repo);
    const worker = text(task.worker);
    const time = duration(task.seconds);
    const cost = money(task.cost_usd);
    const checks = (kind === "review" || kind === "recent") && typeof task.checks === "string" ? task.checks : "";
    const by = this.byLabel(task.by);

    const buttons = [];
    buttons.push(this.action(t, `review-${key}`, "Review", false, () => this.review(task.name, cx)));
    if (kind === "review") {
      buttons.push(this.action(t, `accept-${key}`, "Accept", true,
        () => accepting ? this.accept(task.name, cx) : this.openAccept(task.name, cx)));
      buttons.push(this.action(t, `discard-${key}`, asked === "discard" ? "Discard?" : "Discard", false,
        () => asked === "discard" ? this.runTask("discard", task.name, cx) : this.ask(key, "discard", cx)));
    }
    if (kind === "running" || kind === "queued") {
      buttons.push(this.action(t, `cancel-${key}`, asked === "cancel" ? "Cancel?" : "Cancel", false,
        () => asked === "cancel" ? this.runTask("cancel", task.name, cx) : this.ask(key, "cancel", cx)));
    }
    if (kind === "running") {
      buttons.push(this.action(t, `watch-${key}`, "Watch", false, () => this.watch(task.name)));
    }

    const meta = [];
    if (repo) meta.push(div().child(repo));
    if (worker) meta.push(div().child(worker));
    if (time) meta.push(div().child(time));
    if (cost) meta.push(div().child(cost));
    if (checks) meta.push(div().text_color(t.muted).child(checks));

    return v_flex()
      .id(key)
      .px(10)
      .py(6)
      .gap(4)
      .rounded(7)
      .when(hovered, (el) => el.bg(t.hover))
      .on_hover((on, cx) => {
        if (on) this.hovered = key;
        else if (this.hovered === key) this.hovered = null;
        cx.notify();
      })
      .child(h_flex().gap(6).items_center()
        .child(div().flex_1().min_w(0).truncate().font_medium().text_color(t.foreground).child(task.name))
        .when(Boolean(by), (el) => el.child(this.chip(t, by, this.byColor(t, task.by))))
        .when(show, (el) => el.child(h_flex().gap(4).children(buttons))))
      .when(meta.length > 0, (el) => el.child(h_flex().flex_wrap().gap(8).text_size(11).text_color(t.dim).children(meta)))
      .when(accepting, (el) => el.child(h_flex().pt(2).gap(6).items_center()
        .child(div().flex_1().min_w(0).h(24).px(6).rounded(5).border_1().border_color(t.accent).bg(t.background)
          .child(Input.new(this.accepting.input).h_full().text_size(11).text_color(t.foreground)))
        .child(div().text_size(10).text_color(t.dim).child("Enter accepts · Esc closes"))));
  }

  section(t, title, list, kind, cx) {
    return v_flex().gap(1).pt(8)
      .child(h_flex().px(10).pb(3).gap(6).items_center().text_size(11).font_medium().text_color(t.dim)
        .child(title)
        .child(div().text_color(t.dim).child(String(list.length))))
      .children(list.map((task) => this.row(t, task, kind, cx)));
  }

  renderPanel(cx) {
    const t = theme();
    const data = this.data && typeof this.data === "object" ? this.data : {};
    const tasks = data.tasks && typeof data.tasks === "object" ? data.tasks : {};
    const review = tasksOf(tasks, "review");
    const running = tasksOf(tasks, "running");
    const queued = tasksOf(tasks, "queued");
    const recent = tasksOf(tasks, "recent");
    const waiting = data.waiting && typeof data.waiting === "object" ? data.waiting : {};
    const blocked = Array.isArray(data.agents) ? data.agents.filter((a) => a && a.state === "blocked").length : 0;
    const reviewCount = count(waiting.review, review.length);
    const blockedCount = count(waiting.blocked, blocked);
    const waits = [];
    if (reviewCount > 0) waits.push(`${reviewCount} to review`);
    if (blockedCount > 0) waits.push(`${blockedCount} ${blockedCount === 1 ? "agent" : "agents"} blocked`);
    const headline = waits.length ? waits.join(" · ") : "Nothing waits for you";
    const workers = workersOf(data.workers);
    const workerLine = workers.map((w) => {
      const status = w.status.replace(/_/g, " ");
      const blockedWorker = /block|out/i.test(status);
      const until = blockedWorker ? untilText(w.until) : "";
      return `${w.label} ${status}${until ? ` until ${until}` : ""}`;
    }).join(" · ");
    const claude = data.claude && typeof data.claude === "object" ? data.claude : null;
    const reset = claude ? Number(claude.five_hour_resets_at) : NaN;
    const resetAt = Number.isFinite(reset) && reset > 0 ? new Date(reset * 1000) : null;
    const sections = [
      ["To review", review, "review"],
      ["Running", running, "running"],
      ["Queued", queued, "queued"],
      ["Recent", recent, "recent"],
    ].filter(([, list]) => list.length > 0);
    const empty = sections.length === 0;

    return v_flex()
      .id("board")
      .size_full()
      .min_h(0)
      .bg(t.chrome)
      .text_size(13)
      .text_color(t.foreground)
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key === "escape" && this.accepting) {
          this.accepting = null;
          cx.stop_propagation();
          cx.notify();
        }
      })
      .child(v_flex().flex_none().px(12).pt(10).pb(8).gap(3)
        .child(div().text_size(13).font_semibold().text_color(waits.length ? t.foreground : t.muted).child(headline))
        .when(Boolean(workerLine), (el) => el.child(div().text_size(11).text_color(t.muted).child(workerLine)))
        .when(Boolean(resetAt), (el) => el.child(div().text_size(11).text_color(t.muted)
          .child(`Claude's 5h limit resets at ${clock(resetAt)}`)))
        .when(Boolean(this.error), (el) => el.child(div().text_size(11).text_color(t.muted).child(this.error))))
      .child(v_flex().id("board-list").flex_1().min_h(0).overflow_y_scrollbar().mode("always").p(8)
        .when(empty && !this.data && !this.error, (el) => el.child(div().p(6).text_size(12).text_color(t.muted).child("Loading board…")))
        .when(empty && Boolean(this.data), (el) => el.child(div().p(6).text_size(12).text_color(t.muted).child("No tasks.")))
        .children(sections.map(([title, list, kind]) => this.section(t, title, list, kind, cx))));
  }

  renderReview() {
    const t = theme();
    const review = props()?.review ?? {};
    return v_flex()
      .id("board-review")
      .size_full()
      .min_h(0)
      .bg(t.popover)
      .text_color(t.foreground)
      .child(this.titlebar(t, text(review.title) || "Review"))
      .child(div()
        .id("board-review-body")
        .flex_1()
        .min_h(0)
        .overflow_y_scrollbar()
        .p(12)
        .font_family(settings()?.font_family ?? "JetBrains Mono")
        .text_size(11.5)
        .child(v_flex().w_full().children(lines(review.body))));
  }

  renderTitlebar() {
    const t = theme();
    return v_flex().size_full().bg(t.surface).child(this.titlebar(t, text(props()?.titlebar)));
  }

  render(cx) {
    if (this.mode === "review") return this.renderReview();
    if (this.mode === "titlebar") return this.renderTitlebar();
    return this.renderPanel(cx);
  }
}
