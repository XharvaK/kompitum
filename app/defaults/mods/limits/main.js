// The Limits sheet, from the top bar's context ring: the lead's context, Claude's two windows, and each worker's
// state, its own meters (when its CLI reports any) and what it cost this week. Reading is
// kom status --json, and each worker CLI's own usage report, run as the Settings > Agents page runs it.

import { View, div, PathBuilder, Background } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { kom, exec, knownAgents, window, theme, run, props, closeOverlay, settings, lead, terminals, text } from "kompitum";

const USAGE_EVERY_MS = 60000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAY_WORDS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const pad = (n) => String(n).padStart(2, "0");
const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";
const dollars = (n) => `$${Number(n).toFixed(2)}`;

// Tokens as thousands: 271000 is "271K".
const kTokens = (tokens) => `${Math.round(tokens / 1000)}K`;

// Piped, kom says what went wrong as JSON: {"error": {"message", "hint"}}; show its first line.
function errorText(result) {
  for (const out of [result?.stderr, result?.stdout]) {
    try {
      const message = JSON.parse(String(out ?? "")).error?.message;
      if (message) return firstLine(message);
    } catch {}
  }
  return firstLine(result?.stderr) || firstLine(result?.stdout) || text("limits.kom-exited", { code: result?.code ?? "?" });
}

const runWords = (runs) => `${runs} ${runs === 1 ? "run" : "runs"}`;

// The clock time when a reset is today, the weekday within a week, else "23 Nov" (the rule of Settings > Agents).
function resetWords(seconds) {
  if (typeof seconds !== "number" || !Number.isFinite(seconds) || seconds <= 0) return null;
  const when = new Date(seconds * 1000);
  const now = new Date();
  const midnight = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((midnight(when) - midnight(now)) / 86400000);
  if (days === 0) return `${pad(when.getHours())}:${pad(when.getMinutes())}`;
  if (Math.abs(days) < 7) return WEEKDAY_WORDS[when.getDay()];
  return `${when.getDate()} ${MONTHS[when.getMonth()]}`;
}

// When an out worker is back, in the same words; its until is milliseconds (or seconds from an older core).
function backWords(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return null;
  return resetWords(n > 1e12 ? n / 1000 : n);
}

// npm installs agent CLIs as `.cmd` shims on Windows; CreateProcess won't
// launch those, so the CLI's own report goes through cmd.exe there.
function execCli(entry, args) {
  const argvOf = entry.run ?? [];
  const command = argvOf[0] ?? entry.key;
  const argv = [...argvOf.slice(1), ...args];
  return window()?.platform === "windows"
    ? exec("cmd.exe", ["/d", "/c", command, ...argv])
    : exec(command, argv);
}

function parseUsage(text) {
  let data = null;
  const raw = String(text ?? "").trim();
  try {
    data = JSON.parse(raw);
  } catch {
    const from = raw.indexOf("{");
    const to = raw.lastIndexOf("}");
    if (from >= 0 && to > from) {
      try { data = JSON.parse(raw.slice(from, to + 1)); } catch {}
    }
  }
  if (!data || typeof data !== "object") return null;
  const report = data.data;
  const candidates = [
    data, data.go, data.usage, data.go?.usage, data.usage?.go,
    report, report?.go, report?.usage, report?.go?.usage, report?.usage?.go,
  ];
  // A meter counts only when it says something: a percent, or that it is limited.
  const says = (m) => m && typeof m === "object" && ((m.percent != null && Number.isFinite(Number(m.percent))) || m.limited === true);
  const meters = candidates.find((c) => c && typeof c === "object" && [c.rolling, c.weekly, c.monthly].some(says));
  if (!meters) return null;
  const one = (m) => (m && typeof m === "object" ? { percent: m.percent == null ? null : Number(m.percent), limited: m.limited === true } : null);
  return { rolling: one(meters.rolling), weekly: one(meters.weekly), monthly: one(meters.monthly) };
}

// The chain's workers in the core's order; a key with no worker is skipped.
function chainWorkers(status) {
  const list = Array.isArray(status?.workers?.workers) ? status.workers.workers : [];
  const chain = Array.isArray(status?.workers?.chain) ? status.workers.chain : [];
  return chain.map((key) => list.find((worker) => worker?.key === key)).filter(Boolean);
}

// Claude's two windows that count: a percentage, and a reset not yet past (as the composer counts them).
function claudeWindows(usage) {
  const now = Date.now() / 1000;
  return [
    { key: "5h", label: text("limits.window-5-hours"), window: usage?.five_hour },
    { key: "week", label: text("limits.window-week"), window: usage?.seven_day },
  ]
    .map((w) => ({ ...w, pct: w.window?.used_percentage, resets: w.window?.resets_at }))
    .filter((w) => typeof w.pct === "number" && Number.isFinite(w.pct))
    .filter((w) => !(typeof w.resets === "number" && w.resets < now));
}

// 100% and over is blocked, from 75% working, else the calm colour.
const limitColor = (t, share) => (share >= 1 ? t.blocked : share >= 0.75 ? t.working : t.done);

// A dial's path: `size` px across, a `stroke` on a circle inside it, drawn from the top, clockwise.
function dialPath(fraction, size, stroke) {
  const c = size / 2;
  const r = c - stroke / 2;
  const builder = PathBuilder.stroke(stroke).move_to(c, c - r);
  if (fraction >= 1) return builder.arc_to(r, r, 0, false, true, c, c + r).arc_to(r, r, 0, false, true, c, c - r).build();
  const angle = 2 * Math.PI * Math.min(fraction, 0.999);
  const x = c + r * Math.sin(angle);
  const y = c - r * Math.cos(angle);
  return builder.arc_to(r, r, 0, fraction > 0.5, true, x, y).build();
}

// One bar: its track, and a fill of `share` across it.
function shareBar(t, id, share, color, height, radius) {
  const fraction = Math.min(1, Math.max(0, share));
  return h_flex().id(id).w_full().h(height).rounded(radius).bg(t.hover).overflow_hidden()
    .when(fraction > 0, (el) => el.child(div().h_full().rounded(radius).bg(color).flex_grow(fraction)))
    .when(fraction < 1, (el) => el.child(div().h_full().flex_grow(1 - fraction)));
}

export default class Limits extends View {
  init(_props, cx) {
    this.status = null;
    this.statusError = null;
    this.catalog = [];
    this.usage = {};
    this.usageAt = {};
    this.reading = false;
    this.read(cx);
    cx.timer.every(30000, (cx) => this.read(cx));
  }

  // The status, then the agents' names (read once) and each due CLI's usage report. A failed status read keeps
  // the last data and remembers its first line; a failed CLI report draws no meters for its worker.
  read(cx) {
    if (this.reading) return;
    this.reading = true;
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["status", "--json"]);
        if (r?.code === 0) {
          this.status = JSON.parse(r.stdout || "{}");
          this.statusError = null;
        } else {
          this.statusError = errorText(r);
        }
      } catch (e) {
        this.statusError = firstLine(e?.message ?? e) || text("limits.status-failed");
      }
      if (this.catalog.length === 0) {
        try {
          const list = await knownAgents();
          this.catalog = Array.isArray(list) ? list : [];
        } catch {}
      }
      await this.loadUsage();
      this.reading = false;
      cx.notify();
    });
  }

  // A worker CLI's own report, asked at most once a minute per CLI, for each chain worker whose catalog entry has one.
  async loadUsage() {
    const now = Date.now();
    for (const worker of chainWorkers(this.status)) {
      const entry = this.catalog.find((item) => item.worker === worker.cli);
      if (!entry?.usage?.length) continue;
      if (now - (this.usageAt[entry.key] ?? 0) < USAGE_EVERY_MS) continue;
      this.usageAt[entry.key] = now;
      try {
        const r = await execCli(entry, entry.usage);
        if (r.code !== 0) throw new Error("its usage report couldn't be read");
        const meters = parseUsage(r.stdout);
        if (!meters) throw new Error("its usage report has no meters");
        this.usage[entry.key] = meters;
      } catch {
        // Only that it failed is kept: a CLI's raw report never reaches the screen.
        delete this.usage[entry.key];
      }
    }
  }

  // A plain card: the raised surface, a hairline border, rounded corners and 12 px inside.
  card(t, id, children) {
    return v_flex().id(id).w_full().bg(t.raised).border_1().border_color(t.border).rounded(10).p(12).gap(10)
      .children(children);
  }

  // A card's title: the larger card type, in the foreground.
  cardTitle(t, id, label) {
    return div().id(id).text_size(15).font_medium().text_color(t.foreground).child(label);
  }

  // This conversation: the title and its tokens, one full-width bar, then Compact now centred under it.
  conversationCard(t, usage, terminal) {
    const tokens = usage.context_tokens;
    const max = usage.compact_at;
    const fraction = Math.min(1, Math.max(0, tokens / max));
    const color = limitColor(t, fraction);
    const parts = [
      h_flex().w_full().items_center().justify_between().gap(8)
        .child(this.cardTitle(t, "limits-this-conversation", text("limits.this-conversation")))
        .child(div().id("limits-context-tokens").text_size(14).text_color(t.muted)
          .child(text("limits.context-tokens", { used: kTokens(tokens), total: kTokens(max) }))),
      shareBar(t, "limits-context", fraction, color, 10, 5),
    ];
    if (terminal) parts.push(h_flex().id("limits-compact-row").w_full().justify_center().child(this.compactButton(t, terminal)));
    return this.card(t, "limits-conversation", parts);
  }

  // Live only while the lead waits: no card open, not compacting.
  compactButton(t, terminal) {
    const current = lead();
    const live = current?.state === "waiting" && !current?.ask && current?.compacting !== true;
    return h_flex().id("limits-compact").h(30).px(16).rounded(8).border_1().border_color(t.border)
      .items_center().justify_center()
      .when(live, (el) => el.cursor_pointer().hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected)).tab_stop(true).focus_ring(t.focus, 8).on_click(() => {
        run("lead.compact", { terminal });
        closeOverlay(props().overlay);
      }))
      .child(div().text_size(13).text_color(live ? t.foreground : t.dim).child(text("limits.compact-now")));
  }

  // One 64 px dial: the track, the arc, and the percent (or "Used up") in the middle.
  dial(t, w) {
    const share = Math.min(1, Math.max(0, w.pct / 100));
    const used = w.pct >= 100;
    return div().id(`limits-claude-${w.key}`).relative().w(64).h(64).flex_none()
      .child(globalThis.window.paint_path(dialPath(1, 64, 7), Background.solid(t.hover)).absolute().top(0).left(0).size_full())
      .when(share > 0, (el) => el.child(globalThis.window.paint_path(dialPath(share, 64, 7), Background.solid(limitColor(t, share)))
        .absolute().top(0).left(0).size_full()))
      .child(div().absolute().top(0).left(0).size_full().flex().items_center().justify_center()
        .child(div().text_size(15).font_medium().text_color(used ? t.blocked : t.foreground)
          .child(used ? text("limits.used-up") : `${Math.round(w.pct)}%`)));
  }

  // A dial and its name, with when it resets (or, used up, when it is back).
  dialColumn(t, w) {
    const used = w.pct >= 100;
    const when = resetWords(w.resets);
    return v_flex().flex_1().min_w(0).items_center().gap(3)
      .child(this.dial(t, w))
      .child(div().text_size(14).text_color(used ? t.muted : t.foreground).child(w.label))
      .when(used && Boolean(when), (el) => el.child(div().text_size(13).text_color(t.blocked).child(text("limits.back-at", { when }))))
      .when(!used && Boolean(when), (el) => el.child(div().text_size(13).text_color(t.dim).child(text("limits.resets-at", { when }))));
  }

  claudeCard(t, usage) {
    const windows = claudeWindows(usage);
    const body = windows.length
      ? h_flex().id("limits-claude-dials").w_full().gap(10).children(windows.map((w) => this.dialColumn(t, w)))
      : div().id("limits-claude-none").py(4).text_size(13).text_color(t.dim).child(text("limits.claude-none"));
    return this.card(t, "limits-claude", [this.cardTitle(t, "limits-claude-plan", text("limits.claude-plan")), body]);
  }

  // The week's spend on the Workers title: its dollars and runs, or that nothing ran.
  weekWords(rows) {
    const sum = (pick) => rows.reduce((total, worker) => total + (Number(pick(worker.spend?.week)) || 0), 0);
    const cost = sum((week) => week?.cost_usd);
    const runs = sum((week) => week?.runs);
    return runs > 0 ? `${dollars(cost)} · ${runWords(runs)}` : text("limits.no-runs");
  }

  // A worker's highest reported window: one 40 × 6 bar, with every reported window in its tooltip.
  workerBar(t, worker) {
    const entry = this.catalog.find((item) => item.worker === worker.cli);
    const meters = entry ? this.usage[entry.key] : null;
    const reports = [
      ["5h", text("limits.window-5-hours"), meters?.rolling],
      ["week", text("limits.window-week"), meters?.weekly],
      ["month", text("limits.window-month"), meters?.monthly],
    ].filter(([, , reported]) => reported && (reported.percent != null || reported.limited));
    if (!reports.length) return null;
    const shareOf = (reported) => (reported.limited ? 1 : Math.min(1, Math.max(0, Number(reported.percent) / 100)));
    const share = Math.max(...reports.map(([, , reported]) => shareOf(reported)));
    const tip = reports.map(([, label, reported]) => text("limits.window-share", {
      window: label,
      percent: reported.limited ? 100 : Math.round(Number(reported.percent)),
    })).join(" · ");
    return h_flex().id(`limits-worker-${worker.key}-bar`).w(40).h(6).flex_none().rounded(3).bg(t.hover).overflow_hidden().tooltip(tip)
      .when(share > 0, (el) => el.child(div().h_full().rounded(3).bg(limitColor(t, share)).flex_grow(share)))
      .when(share < 1, (el) => el.child(div().h_full().flex_grow(1 - share)));
  }

  // Next up is a calm badge; ready is quiet words; out is an alarm badge with when it is back.
  workerState(t, worker) {
    if (worker.state === "out") {
      const when = backWords(worker.until_ms);
      return div().id(`limits-worker-${worker.key}-state`).px(8).h(20).rounded(11).bg(t.blocked + "26")
        .flex().items_center().text_size(13).font_medium().text_color(t.blocked)
        .child(when ? text("limits.out-until", { when }) : text("limits.out"));
    }
    if (worker.key === this.status?.workers?.next) {
      return div().id(`limits-worker-${worker.key}-state`).px(8).h(20).rounded(11).bg(t.done + "26")
        .flex().items_center().text_size(13).font_medium().text_color(t.done).child(text("limits.next-up"));
    }
    return div().id(`limits-worker-${worker.key}-state`).text_size(13).text_color(t.dim).child(text("limits.ready"));
  }

  // One worker in the chain's order: its name, its highest window, then its state.
  workerRow(t, worker) {
    const out = worker.state === "out";
    const bar = this.workerBar(t, worker);
    return h_flex().id(`limits-worker-${worker.key}`).w_full().py(4).gap(8).items_center()
      .child(div().flex_1().min_w(0).truncate().text_size(14).text_color(out ? t.muted : t.foreground)
        .child(worker.label ?? worker.key))
      .when(Boolean(bar), (el) => el.child(bar))
      .child(this.workerState(t, worker));
  }

  workersCard(t) {
    const rows = chainWorkers(this.status);
    const parts = [
      h_flex().w_full().items_center().justify_between().gap(8)
        .child(this.cardTitle(t, "limits-workers-title", text("limits.workers")))
        .child(div().id("limits-spend-total").text_size(14).text_color(rows.length ? t.foreground : t.dim).child(this.weekWords(rows))),
    ];
    if (!this.status && !this.statusError) {
      parts.push(div().id("limits-reading").py(4).text_size(13).text_color(t.dim).child(text("limits.reading-workers")));
    }
    if (this.statusError) {
      parts.push(div().id("limits-error").py(4).text_size(13).text_color(t.dim).child(this.statusError));
    }
    parts.push(...rows.map((worker) => this.workerRow(t, worker)));
    return this.card(t, "limits-workers", parts);
  }

  render() {
    const t = theme();
    const usage = lead()?.usage;
    // Compact now runs in the terminal the popup was opened for, else the live lead's.
    const terminal = typeof props()?.terminal === "string"
      ? props().terminal
      : terminals().find((each) => each.state === "live" && each.labels?.lead)?.address ?? null;
    const cards = [];
    if (typeof usage?.context_tokens === "number" && typeof usage?.compact_at === "number" && usage.compact_at > 0) {
      cards.push(this.conversationCard(t, usage, terminal));
    }
    if ((settings()?.lead?.program ?? "claude") === "claude") cards.push(this.claudeCard(t, usage));
    cards.push(this.workersCard(t));
    return v_flex().id("limits").size_full().min_h(0).bg(t.chrome).text_color(t.foreground).text_size(14)
      .px(12).pt(4).pb(12).gap(10).overflow_y_scroll()
      .children(cards);
  }
}
