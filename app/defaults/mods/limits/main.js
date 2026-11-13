// The Limits sheet, from the composer's ring and the sidebar's foot: the lead's context, Claude's two windows,
// and each worker's state, its own meters (when its CLI reports any) and what it cost this week. Reading is
// kom status --json, and each worker CLI's own usage report, run as the Settings > Agents page runs it.

import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { kom, exec, knownAgents, window, theme, run, props, closeOverlay, settings, lead } from "kompitum";

const BAR = 316;
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
  return firstLine(result?.stderr) || firstLine(result?.stdout) || `kom exited ${result?.code ?? "?"}`;
}

// A worker's out-until time, as the board says it: today "09:10", another day "Nov 4 18:26".
function untilText(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "";
  const d = new Date(n > 1e12 ? n : n * 1000);
  const clock = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  return d.toDateString() === new Date().toDateString() ? clock : `${MONTHS[d.getMonth()]} ${d.getDate()} ${clock}`;
}

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
    { key: "5h", label: "5-hour limit", window: usage?.five_hour, seconds: 5 * 3600 },
    { key: "week", label: "Week", window: usage?.seven_day, seconds: 7 * 86400 },
  ]
    .map((w) => ({ ...w, pct: w.window?.used_percentage, resets: w.window?.resets_at }))
    .filter((w) => typeof w.pct === "number" && Number.isFinite(w.pct))
    .filter((w) => !(typeof w.resets === "number" && w.resets < now));
}

// How far through its window the clock is, from the reset: 0 at its start, 1 at its reset; null when it can't say.
function paceOf(resets, seconds) {
  if (typeof resets !== "number" || !Number.isFinite(resets)) return null;
  const pace = 1 - (resets - Date.now() / 1000) / seconds;
  return pace >= 0 && pace <= 1 ? pace : null;
}

function claudeWords(w) {
  const when = resetWords(w.resets);
  if (w.pct >= 100) return when ? `Used up · back ${when}` : "Used up";
  const pct = Math.round(w.pct);
  return when ? `${pct}% · resets ${when}` : `${pct}%`;
}

// 100% and over is blocked, from 75% working, else muted.
const limitColor = (t, share) => (share >= 1 ? t.blocked : share >= 0.75 ? t.working : t.muted);

// One bar: its label and words over a track of BAR px, a fill, and a pace mark when the window has a clock.
function meter(t, id, label, words, fraction, pace, color, wordsColor) {
  const share = Math.min(1, Math.max(0, fraction));
  const fill = Math.round(BAR * share);
  const track = div().id(`${id}-bar`).relative().w(BAR).h(6).rounded(3).bg(t.hover)
    .when(fill > 0, (el) => el.child(div().absolute().top(0).left(0).h(6).w(fill).rounded(3).bg(color)))
    .when(pace != null, (el) => el
      .child(div().id(`${id}-pace`).absolute().top(-2).left(Math.round(BAR * pace) - 1).w(2).h(10).rounded(1).bg(t.foreground).opacity(0.55))
      .tooltip(`${Math.round(pace * 100)}% of this window has passed`));
  return v_flex().id(id).gap(5).py(4)
    .child(h_flex().justify_between().items_center()
      .child(div().text_size(12.5).text_color(t.muted).child(label))
      .child(div().text_size(12.5).text_color(wordsColor).child(words)))
    .child(track);
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
        this.statusError = firstLine(e?.message ?? e) || "status failed";
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

  sectionHeader(t, label, right) {
    if (!right) return div().pt(6).pb(2).text_size(11.5).font_medium().text_color(t.dim).child(label);
    return h_flex().pt(6).pb(2).items_center()
      .child(div().text_size(11.5).font_medium().text_color(t.dim).child(label))
      .child(div().flex_1())
      .child(right);
  }

  contextParts(t, usage, terminal) {
    const tokens = usage.context_tokens;
    const max = usage.compact_at;
    const fraction = tokens / max;
    const color = fraction >= 0.95 ? t.blocked : fraction >= 0.75 ? t.working : t.muted;
    const parts = [
      this.sectionHeader(t, "Context", null),
      meter(t, "limits-context", "Context", `${kTokens(tokens)} of ${kTokens(max)}`, fraction, null, color,
        fraction >= 1 ? t.blocked : t.foreground),
      div().text_size(12).text_color(t.dim).child("Kompitum tidies up by itself while you're away."),
    ];
    if (terminal) parts.push(this.compactButton(t, terminal));
    return parts;
  }

  // Live only while the lead waits: no card open, not compacting.
  compactButton(t, terminal) {
    const current = lead();
    const live = current?.state === "waiting" && !current?.ask && current?.compacting !== true;
    return h_flex().id("limits-compact").w_full().h(30).rounded(8).border_1().border_color(t.border)
      .items_center().justify_center()
      .when(live, (el) => el.cursor_pointer().hover((s) => s.bg(t.hover)).on_click(() => {
        run("lead.compact", { terminal });
        closeOverlay(props().overlay);
      }))
      .child(div().text_size(12.5).text_color(live ? t.foreground : t.dim).child("Compact now"));
  }

  claudeParts(t, usage) {
    const windows = claudeWindows(usage);
    const parts = [this.sectionHeader(t, "Claude", null)];
    for (const w of windows) {
      const share = w.pct / 100;
      parts.push(meter(t, `limits-claude-${w.key}`, w.label, claudeWords(w), share, paceOf(w.resets, w.seconds),
        limitColor(t, share), share >= 1 ? t.blocked : t.foreground));
    }
    if (windows.length === 0) {
      parts.push(div().id("limits-claude-none").py(4).text_size(12).text_color(t.dim).child("Claude's limits show after its next reply."));
    }
    return parts;
  }

  workerParts(t) {
    const rows = chainWorkers(this.status);
    const total = rows.reduce((sum, worker) => sum + (Number(worker.spend?.week?.cost_usd) || 0), 0);
    const right = this.status
      ? div().id("limits-spend-total").text_size(11.5).text_color(t.muted).child(`${dollars(total)} this week`)
      : null;
    const parts = [this.sectionHeader(t, "Workers", right)];
    if (!this.status && !this.statusError) {
      parts.push(div().id("limits-reading").py(4).text_size(12).text_color(t.dim).child("Reading the workers…"));
    }
    if (this.statusError) {
      parts.push(div().id("limits-error").py(4).text_size(11.5).text_color(t.dim).child(this.statusError));
    }
    parts.push(...rows.map((worker) => this.workerRow(t, worker)));
    return parts;
  }

  workerRow(t, worker) {
    const next = this.status?.workers?.next;
    const until = untilText(worker.until_ms);
    const state = worker.state === "out"
      ? div().text_size(12).text_color(t.blocked).child(until ? `Out until ${until}` : "Out")
      : worker.key === next
        ? div().text_size(12).text_color(t.accent).child("Next up")
        : div().text_size(12).text_color(t.dim).child("Ready");
    const cost = Number(worker.spend?.week?.cost_usd) || 0;
    const runs = Number(worker.spend?.week?.runs) || 0;
    const detail = runs > 0 ? `${dollars(cost)} this week · ${runs} ${runs === 1 ? "run" : "runs"}` : "Nothing this week";
    const entry = this.catalog.find((item) => item.worker === worker.cli);
    const meters = entry ? this.usage[entry.key] : null;
    const bars = [];
    for (const [key, label, reported] of [["5h", "5 hours", meters?.rolling], ["week", "Week", meters?.weekly], ["month", "Month", meters?.monthly]]) {
      if (!reported || (reported.percent == null && !reported.limited)) continue;
      const pct = reported.percent == null ? null : Math.round(reported.percent);
      const share = pct == null ? 1 : pct / 100;
      const used = reported.limited || share >= 1;
      const words = used ? "Used up" : `${pct}%`;
      bars.push(meter(t, `limits-worker-${worker.key}-${key}`, label, words, share, null,
        used ? t.blocked : limitColor(t, share), used ? t.blocked : t.foreground));
    }
    return v_flex().id(`limits-worker-${worker.key}`).py(6).gap(2)
      .child(h_flex().justify_between().items_center()
        .child(div().text_size(13).text_color(t.foreground).child(worker.label ?? worker.key))
        .child(state))
      .child(div().text_size(11.5).text_color(t.dim).child(detail))
      .children(bars);
  }

  render() {
    const t = theme();
    const usage = lead()?.usage;
    const terminal = typeof props()?.terminal === "string" ? props().terminal : null;
    const parts = [];
    if (typeof usage?.context_tokens === "number" && typeof usage?.compact_at === "number") {
      parts.push(...this.contextParts(t, usage, terminal));
    }
    if ((settings()?.lead?.program ?? "claude") === "claude") parts.push(...this.claudeParts(t, usage));
    parts.push(...this.workerParts(t));
    return v_flex().id("limits").size_full().min_h(0).bg(t.chrome).text_color(t.foreground).text_size(13)
      .p(12).gap(4).overflow_y_scroll()
      .children(parts);
  }
}
