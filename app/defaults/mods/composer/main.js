// The composer under the lead's terminal: one box that sends only what the
// Owner typed, when they press Enter or click Send. Files and folders dropped
// on it, and images pasted into it, wait as chips above the text.
//
// Part 5b turns `state` and `card` into Claude's prompts; both are empty here.

import { View, div, svg, image } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState } from "gpui-base";
import {
  kom,
  exec,
  run,
  theme,
  props,
  shared,
  setShared,
  terminals,
  knownAgents,
  appLink,
  openLink,
  openOverlay,
  appHome,
  readTextFile,
  window,
  pasteImage,
} from "kompitum";

const HISTORY_KEY = "composer.history";
const HISTORY_LIMIT = 100;
const PLANS_EVERY_MS = 60000;
const SYSTEMS = [{ id: "grokbot", name: "Grok Bot", scheme: "grokbot" }];
const WINDOWS = [
  { label: "5H", spend: "five_hours", report: "rolling", block: "5-hour", clock: true },
  { label: "Week", spend: "week", report: "weekly", block: "weekly", clock: false },
  { label: "Month", spend: "month", report: "monthly", block: "monthly", clock: false },
];

function errorText(error) {
  if (error == null) return "The command failed.";
  if (typeof error === "string") return error;
  if (typeof error.message === "string" && error.message.trim()) return error.message.trim();
  for (const raw of [error.stderr, error.stdout]) {
    if (typeof raw !== "string" || !raw.trim()) continue;
    try {
      const said = JSON.parse(raw.trim());
      const message = said?.error?.message ?? said?.message ?? said?.error;
      if (typeof message === "string" && message.trim()) return message.trim();
    } catch {}
    return raw.trim();
  }
  if (typeof error.code === "number") return `The command failed with exit code ${error.code}.`;
  return String(error);
}

function firstLine(text) {
  return String(text ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";
}

function baseName(path) {
  return String(path ?? "").split(/[\\/]/).filter(Boolean).pop() ?? String(path ?? "");
}

function quotePath(path) {
  const text = String(path ?? "");
  return /[ \t]/.test(text) ? `"${text}"` : text;
}

function loadHistory() {
  try {
    const list = JSON.parse(localStorage.getItem(HISTORY_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter((text) => typeof text === "string").slice(0, HISTORY_LIMIT) : [];
  } catch {
    return [];
  }
}

function saveHistory(list) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(list.slice(0, HISTORY_LIMIT)));
  } catch {}
}

function modelLabel(entry, id) {
  return entry?.models?.find((model) => model.id === id)?.label ?? id;
}

function leadName(entry, key) {
  return key === "claude" ? "Claude" : entry?.name ?? key;
}

function two(n) {
  return String(n).padStart(2, "0");
}

function resetWords(ms, clock) {
  if (ms == null || ms === 0) return null;
  let when;
  if (typeof ms === "string" && Number.isNaN(Number(ms))) {
    when = new Date(ms);
  } else {
    const value = Number(ms);
    if (!Number.isFinite(value)) return null;
    when = new Date(value > 1e12 ? value : value * 1000);
  }
  if (Number.isNaN(when.getTime())) return null;
  const now = new Date();
  const sameDay = when.getFullYear() === now.getFullYear() && when.getMonth() === now.getMonth() && when.getDate() === now.getDate();
  if (clock || sameDay) return `${two(when.getHours())}:${two(when.getMinutes())}`;
  const midnight = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((midnight(when) - midnight(now)) / 86400000);
  if (Math.abs(days) < 7) return ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][when.getDay()];
  return `${["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][when.getMonth()]} ${when.getDate()}`;
}

function routeModel(route) {
  let model = String(route ?? "").split("#")[0];
  if (model.startsWith("commandcode/")) model = model.slice("commandcode/".length);
  return model;
}

function planLimit(plan, model, label) {
  if (plan?.models) return plan.models?.[model]?.[label];
  return plan?.windows?.[label];
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
  const candidates = [data, data.go, data.usage, data.go?.usage, data.usage?.go];
  const meters = candidates.find((c) => c && typeof c === "object" && (c.rolling || c.weekly || c.monthly));
  if (!meters) return null;
  const one = (m) => (m && typeof m === "object" ? { percent: Number(m.percent), resetsAt: m.resetsAt } : null);
  return { rolling: one(meters.rolling), weekly: one(meters.weekly), monthly: one(meters.monthly) };
}

// npm installs CLIs as `.cmd` shims on Windows; CreateProcess won't launch those.
function execCli(entry, args) {
  const run = entry.run ?? [];
  const command = run[0] ?? entry.key;
  const argv = [...run.slice(1), ...args];
  return window()?.platform === "windows"
    ? exec("cmd.exe", ["/d", "/c", command, ...argv])
    : exec(command, argv);
}

function meter(t, label, percent, reset, { approx = false, exhausted = false } = {}) {
  const used = Number(percent);
  const has = Number.isFinite(used);
  const spent = has && used >= 100;
  const text = has
    ? `${label} ${spent ? "exhausted" : `${approx ? "≈" : ""}${Math.min(99, Math.round(used))}%`}${reset ? ` · resets ${reset}` : ""}`
    : `${label}${reset ? ` · resets ${reset}` : ""}`;
  const row = v_flex().gap(4).child(div().text_size(12.5).text_color(spent || exhausted ? t.working : t.muted).child(text));
  if (has) {
    const width = 170 * Math.max(2, Math.min(100, used)) / 100;
    row.child(div().w(170).h(3).rounded(2).bg(t.hover).child(div().w(width).h(3).rounded(2).bg(spent || exhausted ? t.working : t.accent)));
  }
  return row;
}

export default class Composer extends View {
  init(_props, cx) {
    const p = props() ?? {};
    this.mode = p.mode === "plans" ? "plans" : "compose";
    this.terminal = typeof p.terminal === "string" ? p.terminal : null;
    this.person = {};
    this.agents = [];
    this.status = null;
    this.workers = null;
    this.usage = {};
    this.usageAt = {};
    this.usageError = {};
    this.apps = [];
    this.loading = false;
    this.sending = false;
    this.error = null;
    this.chips = [];
    this.history = loadHistory();
    this.historyAt = -1;
    this.draft = "";
    this.askAt = null;
    this.hoverPlan = false;
    // Part 5b fills these: working, asking, waiting, or unknown, and its card.
    this.state = null;
    this.card = null;
    this.handle = cx.focus_handle();
    if (this.mode === "compose") {
      this.text = TextareaState.new({ placeholder: "Message the lead…" });
      this.text.set_auto_grow(1, 10);
      this.text.set_submit_on_enter(true);
      this.text.on("change", (_event, cx) => {
        if (this.text.value().length === 0) this.historyAt = -1;
        this.historyMode = this.text.value().length === 0 || this.historyAt >= 0;
        cx.notify();
      });
      this.text.on("submit", (event, cx) => this.submit(event, cx));
      this.text.on("paste_other", (_event, cx) => this.pastedImage(cx));
      cx.bind_keys([
        { keystroke: "escape", action: "composer-escape", context: "Composer" },
        { keystroke: "escape", action: "composer-escape", context: "ComposerHistory" },
        { keystroke: "up", action: "composer-history-up", context: "ComposerHistory" },
        { keystroke: "down", action: "composer-history-down", context: "ComposerHistory" },
      ]);
      this.historyMode = true;
      this.takeAsk(cx);
    } else {
      this.handle.focus();
    }
    cx.spawn(async (cx) => {
      await this.load(cx);
      cx.notify();
    });
    cx.timer.every(5000, (cx) => {
      if (this.mode === "compose") this.load(cx);
    });
    cx.timer.every(500, (cx) => {
      if (this.mode === "compose") this.takeAsk(cx);
    });
  }

  async load(cx) {
    if (this.loading) return;
    this.loading = true;
    try {
      let person = {};
      try {
        const raw = await readTextFile(`${appHome()}/settings.json`);
        const parsed = raw ? JSON.parse(raw) : {};
        person = parsed && typeof parsed === "object" ? parsed : {};
      } catch {}
      this.person = person;
      try {
        this.agents = await knownAgents();
      } catch {
        this.agents = [];
      }
      try {
        const r = await kom(["status", "--json"]);
        if (r?.code === 0) this.status = JSON.parse(r.stdout || "null");
      } catch {}
      try {
        const r = await kom(["workers", "--json"]);
        if (r?.code === 0) this.workers = JSON.parse(r.stdout || "null");
      } catch {}
      await this.loadUsage();
      const apps = [];
      for (const system of SYSTEMS) {
        if (!this.person?.connect?.[system.id]?.agent) continue;
        try {
          const app = appLink(system.scheme);
          if (app) apps.push({ ...system, app });
        } catch {}
      }
      this.apps = apps;
    } finally {
      this.loading = false;
    }
  }

  // The CLI's own report wins over Kompitum's estimate, at most once a minute.
  async loadUsage() {
    const now = Date.now();
    for (const entry of this.agents ?? []) {
      if (!entry.usage?.length) continue;
      if (now - (this.usageAt[entry.key] ?? 0) < PLANS_EVERY_MS) continue;
      this.usageAt[entry.key] = now;
      try {
        const r = await execCli(entry, entry.usage);
        if (r.code !== 0) throw new Error(firstLine(r.stderr || r.stdout) || `exit ${r.code}`);
        const meters = parseUsage(r.stdout);
        if (!meters) throw new Error(firstLine(r.stdout) || "its report has no usage meters");
        this.usage[entry.key] = meters;
        delete this.usageError[entry.key];
      } catch (error) {
        delete this.usage[entry.key];
        this.usageError[entry.key] = firstLine(errorText(error));
      }
    }
  }

  leadProgram() {
    const key = this.person?.lead?.program ?? "claude";
    return this.agents?.find((entry) => entry.key === key) ?? { key, name: key, models: [] };
  }

  leadLive() {
    return Boolean(this.terminal && terminals().some((terminal) => terminal.address === this.terminal));
  }

  leadLine() {
    const lead = this.person?.lead ?? {};
    const entry = this.leadProgram();
    const parts = [leadName(entry, entry.key)];
    if (lead.model) parts.push(modelLabel(entry, lead.model));
    if (lead.effort) parts.push(lead.effort);
    return parts.join(" · ");
  }

  takeAsk(cx) {
    if (this.mode !== "compose" || !this.leadLive()) return;
    const ask = shared("composer.ask");
    if (!ask || typeof ask.text !== "string" || !ask.text.trim()) return;
    if (this.askAt === ask.at) return;
    this.askAt = ask.at;
    setShared("composer.ask", null);
    this.text.set_value(ask.text);
    this.text.focus();
    cx.notify();
  }

  addChip(path) {
    const text = String(path ?? "");
    if (!text || this.chips.some((chip) => chip.path === text)) return;
    this.chips.push({ path: text });
  }

  removeChip(index, cx) {
    this.chips.splice(index, 1);
    cx.notify();
  }

  pastedImage(cx) {
    let path = null;
    try {
      path = pasteImage();
    } catch (error) {
      this.error = errorText(error);
    }
    if (path) {
      this.addChip(path);
      this.text.focus();
    }
    cx.notify();
  }

  escape(cx) {
    if (this.text.value().length === 0 && this.leadLive()) this.stopTurn(cx);
  }

  historyKey(direction, cx) {
    if (direction > 0) {
      if (this.historyAt < 0 && this.text.value().length > 0) return;
      if (this.historyAt >= this.history.length - 1) return;
    } else if (this.historyAt < 0) {
      return;
    }
    this.moveHistory(direction, cx);
  }

  moveHistory(delta, cx) {
    if (this.historyAt < 0) this.draft = this.text.value();
    const next = this.historyAt + delta;
    if (next < 0) {
      this.historyAt = -1;
      this.text.set_value(this.draft);
    } else if (next < this.history.length) {
      this.historyAt = next;
      this.text.set_value(this.history[next]);
    }
    this.historyMode = this.text.value().length === 0 || this.historyAt >= 0;
    this.text.focus();
    cx.notify();
  }

  stopTurn(cx) {
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["keys", this.terminal, "esc"]);
        if (!r || r.code !== 0) throw new Error(errorText(r));
      } catch (error) {
        this.error = errorText(error);
      }
      cx.notify();
    });
  }

  submit(event, cx) {
    if (event?.shift) return;
    const value = this.text.value();
    if (value.endsWith("\\")) {
      this.text.set_value(`${value.slice(0, -1)}\n`);
      this.text.focus();
      cx.notify();
      return;
    }
    this.send(cx);
  }

  send(cx) {
    if (this.sending) return;
    const text = this.text.value();
    const paths = this.chips.map((chip) => chip.path);
    if (!text.trim() && paths.length === 0) return;
    if (!this.leadLive()) {
      this.error = "The lead isn't running";
      cx.notify();
      return;
    }
    const body = [text.trim() ? text : "", ...paths.map(quotePath)].filter(Boolean).join(" ");
    this.sending = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["send", this.terminal, body]);
        if (!r || r.code !== 0) throw new Error(errorText(r));
        this.text.set_value("");
        this.chips = [];
        this.error = null;
        this.historyAt = -1;
        this.draft = "";
        this.historyMode = true;
        if (text.trim()) {
          this.history = [text, ...this.history.filter((message) => message !== text)].slice(0, HISTORY_LIMIT);
          saveHistory(this.history);
        }
      } catch (error) {
        // The words stay in the box; the refusal shows under it.
        this.error = errorText(error);
      } finally {
        this.sending = false;
        cx.notify();
      }
    });
  }

  iconButton(id, icon, t, action) {
    return h_flex()
      .id(id)
      .h(24)
      .px(6)
      .items_center()
      .rounded(6)
      .cursor_pointer()
      .hover((style) => style.bg(t.hover))
      .on_click(action)
      .child(icon);
  }

  filledButton(id, label, action, t) {
    return h_flex()
      .id(id)
      .h(28)
      .px(12)
      .items_center()
      .rounded(7)
      .bg(t.accent_fill)
      .text_color(t.accent_foreground)
      .text_size(12.5)
      .font_medium()
      .cursor_pointer()
      .hover((style) => style.opacity(0.88))
      .on_click(action)
      .child(label);
  }

  dot(t) {
    const colors = { working: t.working, asking: t.blocked, waiting: t.done };
    const color = this.state ? colors[this.state] : null;
    return div().w(7).h(7).rounded(4).when(Boolean(color), (el) => el.bg(color));
  }

  ring(t, percent, color, size = 18) {
    const segments = 12;
    const filled = Math.round(Math.max(0, Math.min(100, Number(percent) || 0)) / 100 * segments);
    const dot = 2.1;
    const radius = size / 2 - dot;
    let element = div().w(size).h(size).relative().flex_none();
    for (let i = 0; i < segments; i++) {
      const angle = (i / segments) * Math.PI * 2 - Math.PI / 2;
      element = element.child(
        div()
          .absolute()
          .left(size / 2 + Math.cos(angle) * radius - dot)
          .top(size / 2 + Math.sin(angle) * radius - dot)
          .w(dot * 2)
          .h(dot * 2)
          .rounded(dot)
          .bg(i < filled ? color : t.border),
      );
    }
    return element;
  }

  summary() {
    const entry = this.leadProgram();
    const percents = [];
    const claude = entry.key === "claude" ? this.status?.claude : null;
    if (claude) {
      for (const [label, value, at, clock] of [
        ["5H", claude.five_hour_used_percentage, claude.five_hour_resets_at, true],
        ["Week", claude.seven_day_used_percentage, claude.seven_day_resets_at, false],
      ]) {
        if (Number.isFinite(Number(value))) {
          percents.push({ percent: Number(value), label, reset: resetWords(at, clock) });
        }
      }
    }
    const plan = (entry.plans ?? []).find((item) => item.id === this.person?.plans?.[entry.key]);
    if (plan) {
      const workers = (this.workers?.workers ?? []).filter((worker) => worker.cli === entry.worker);
      const first = (this.workers?.order ?? [])
        .map((key) => workers.find((worker) => worker.key === key))
        .find(Boolean) ?? workers[0];
      const model = first ? routeModel(first.route) : null;
      for (const window of WINDOWS) {
        const blocked = workers.find((worker) => worker.state === "out" && worker.window === window.block);
        if (blocked) {
          percents.push({ percent: 100, label: window.label, reset: resetWords(blocked.until_ms, window.clock), exhausted: true });
          continue;
        }
        const report = this.usage[entry.key]?.[window.report];
        if (Number.isFinite(Number(report?.percent))) {
          percents.push({ percent: Number(report.percent), label: window.label, reset: resetWords(report.resetsAt, window.clock) });
          continue;
        }
        const limit = planLimit(plan, model, window.label);
        if (Number.isFinite(Number(limit)) && Number(limit) > 0) {
          const spent = workers.reduce((sum, worker) => sum + Number(worker.spend?.[window.spend]?.cost_usd ?? 0), 0);
          percents.push({ percent: (spent / Number(limit)) * 100, label: window.label, approx: true });
        }
      }
    }
    if (!percents.length) return null;
    const fullest = percents.reduce((best, item) => (item.percent > best.percent ? item : best));
    const percent = Math.max(0, Math.min(100, fullest.percent));
    const name = leadName(entry, entry.key);
    const out = fullest.exhausted || percent >= 100;
    return {
      percent,
      exhausted: out,
      color: out ? t.blocked : percent >= 80 ? t.working : t.accent,
      text: `${name} ${out ? "out" : `${Math.round(percent)}%`}${fullest.reset ? ` · resets ${fullest.reset}` : ""}`,
    };
  }

  planRing(t) {
    const summary = this.summary();
    return h_flex()
      .id("composer-plan")
      .h(24)
      .px(6)
      .gap(6)
      .items_center()
      .rounded(6)
      .cursor_pointer()
      .hover((style) => style.bg(t.hover))
      .on_hover((hovered, cx) => {
        this.hoverPlan = hovered;
        cx.notify();
      })
      .on_click(() => openOverlay({ mod: "composer", kind: "popup", width: 560, height: 0.8, props: { mode: "plans" } }))
      .when(Boolean(this.hoverPlan && summary), (el) => el.child(div().text_size(11).text_color(t.muted).child(summary.text)))
      .child(this.ring(t, summary?.percent ?? 0, summary?.color ?? t.accent));
  }

  header(t, cx) {
    return h_flex()
      .w_full()
      .h(30)
      .px(10)
      .items_center()
      .gap(8)
      .border_b(1)
      .border_color(t.border)
      .child(div().text_size(12).font_medium().text_color(t.foreground).child(this.leadLine()))
      .child(this.dot(t))
      .child(div().flex_1())
      .child(this.iconButton("composer-mail", svg("frame/settings/mail.svg").size(14).text_color(t.muted), t, () => run("app.mailbox")))
      .child(this.planRing(t))
      .children(this.apps.map((app) =>
        this.iconButton(
          `composer-app-${app.id}`,
          app.app.icon
            ? image(app.app.icon).w(16).h(16).flex_none()
            : svg("frame/settings/link.svg").size(14).text_color(t.muted),
          t,
          () => openLink(`${app.scheme}://`),
        )));
  }

  chipsRow(t, cx) {
    return h_flex()
      .w_full()
      .px(10)
      .pt(7)
      .gap(6)
      .flex_wrap()
      .children(this.chips.map((chip, index) =>
        h_flex()
          .id(`composer-chip-${index}`)
          .h(22)
          .px(7)
          .gap(5)
          .items_center()
          .rounded(6)
          .border_1()
          .border_color(t.border)
          .bg(t.raised)
          .child(div().text_size(11.5).text_color(t.muted).child(baseName(chip.path)))
          .child(
            h_flex()
              .id(`composer-chip-remove-${index}`)
              .w(14)
              .h(14)
              .items_center()
              .justify_center()
              .rounded(4)
              .text_size(11)
              .text_color(t.dim)
              .cursor_pointer()
              .hover((style) => style.bg(t.hover).text_color(t.foreground))
              .on_click(() => this.removeChip(index, cx))
              .child("×"),
          )));
  }

  empty(t) {
    return v_flex()
      .id("composer-empty")
      .size_full()
      .px(12)
      .py(10)
      .gap(8)
      .bg(t.background)
      .child(div().text_size(12.5).text_color(t.muted).child("The lead isn't running"))
      .child(h_flex().child(this.filledButton("composer-open", "Open", () => run("lead.open"), t)));
  }

  compose(t, cx) {
    return v_flex()
      .id("composer")
      .size_full()
      .min_h(0)
      .bg(t.background)
      .track_focus(this.handle)
      .key_context(this.historyMode ? "ComposerHistory" : "Composer")
      .on_action("composer-escape", (_event, cx) => this.escape(cx))
      .on_action("composer-history-up", (_event, cx) => this.historyKey(1, cx))
      .on_action("composer-history-down", (_event, cx) => this.historyKey(-1, cx))
      .on_drop((event, cx) => {
        for (const path of event.paths ?? []) this.addChip(path);
        cx.notify();
      })
      .child(this.header(t, cx))
      .when(this.chips.length > 0, (el) => el.child(this.chipsRow(t, cx)))
      .when(Boolean(this.state), (el) => el.child(div().px(10).pt(4).text_size(11).text_color(t.dim).child(String(this.state))))
      .when(Boolean(this.card), (el) => el.child(div().px(10).pt(4).text_size(12).text_color(t.muted).child(String(this.card))))
      .child(
        v_flex()
          .flex_1()
          .min_h(0)
          .px(10)
          .py(8)
          .child(
            div()
              .w_full()
              .rounded(7)
              .border_1()
              .border_color(t.input)
              .bg(t.surface)
              .px(9)
              .py(6)
              .child(Textarea.new(this.text).w_full()),
          ),
      )
      .when(Boolean(this.error), (el) =>
        el.child(div().id("composer-error").px(12).pb(6).text_size(11.5).text_color(t.blocked).child(this.error)))
      .child(
        h_flex()
          .w_full()
          .px(10)
          .pb(8)
          .items_center()
          .gap(8)
          .child(div().flex_1())
          .child(this.filledButton("composer-send", this.sending ? "Sending…" : "Send", () => this.send(cx), t)),
      );
  }

  planRow(t, cx, entry) {
    const plan = (entry.plans ?? []).find((item) => item.id === this.person?.plans?.[entry.key]);
    const head = h_flex()
      .gap(8)
      .items_center()
      .child(entry.icon ? (entry.tint ? svg(entry.icon).size(16).text_color(t.dim) : image(entry.icon).w(16).h(16)) : div().w(16).h(16))
      .child(div().text_size(13.5).font_medium().text_color(t.foreground).child(entry.name ?? entry.key));
    if (!plan) {
      return v_flex().w_full().py(10).gap(4).border_b(1).border_color(t.border)
        .child(head)
        .child(div().text_size(12.5).text_color(t.dim).child((entry.plans ?? []).length ? `Pick this CLI's plan in Settings › Agents (${(entry.plans ?? []).map((item) => item.label).join(", ")}).` : "No plans for this CLI."));
    }
    const workers = (this.workers?.workers ?? []).filter((worker) => worker.cli === entry.worker);
    const first = (this.workers?.order ?? []).map((key) => workers.find((worker) => worker.key === key)).find(Boolean) ?? workers[0];
    const model = first ? routeModel(first.route) : null;
    const rows = [];
    for (const window of WINDOWS) {
      const blocked = workers.find((worker) => worker.state === "out" && worker.window === window.block);
      if (blocked) {
        rows.push(meter(t, window.label, 100, resetWords(blocked.until_ms, window.clock), { exhausted: true }));
        continue;
      }
      const report = this.usage[entry.key]?.[window.report];
      if (Number.isFinite(Number(report?.percent))) {
        rows.push(meter(t, window.label, report.percent, resetWords(report.resetsAt, window.clock)));
        continue;
      }
      const limit = planLimit(plan, model, window.label);
      if (!Number.isFinite(Number(limit)) || Number(limit) <= 0) continue;
      const spent = workers.reduce((sum, worker) => sum + Number(worker.spend?.[window.spend]?.cost_usd ?? 0), 0);
      rows.push(meter(t, window.label, (spent / Number(limit)) * 100, null, { approx: true }));
    }
    return v_flex().w_full().py(10).gap(6).border_b(1).border_color(t.border)
      .child(head)
      .children(rows)
      .when(Boolean(this.usageError[entry.key]), (el) =>
        el.child(div().text_size(12).text_color(t.dim).child(`${entry.name ?? entry.key}: ${this.usageError[entry.key]}; these are Kompitum's own estimates.`)));
  }

  plans(t, cx) {
    const claude = this.status?.claude;
    const entries = [];
    const seen = new Set();
    for (const key of this.workers?.order ?? []) {
      const worker = (this.workers?.workers ?? []).find((item) => item.key === key);
      const entry = worker ? (this.agents ?? []).find((item) => item.worker === worker.cli) : null;
      if (entry?.plans?.length && !seen.has(entry.key)) {
        seen.add(entry.key);
        entries.push(entry);
      }
    }
    const claudeRows = [];
    if (claude) {
      const five = cableValue(claude.five_hour_used_percentage, claude.five_hour_resets_at, true);
      const week = cableValue(claude.seven_day_used_percentage, claude.seven_day_resets_at, false);
      if (five) claudeRows.push(five);
      if (week) claudeRows.push(week);
    }
    return v_flex()
      .id("composer-plans")
      .size_full()
      .bg(t.popover)
      .text_color(t.foreground)
      .child(
        h_flex()
          .h(38)
          .px(14)
          .items_center()
          .justify_between()
          .bg(t.chrome)
          .border_b(1)
          .border_color(t.border)
          .child(div().text_size(13).font_semibold().child("Plans"))
          .child(
            h_flex()
              .id("composer-plans-close")
              .w(22)
              .h(22)
              .items_center()
              .justify_center()
              .rounded(6)
              .text_color(t.dim)
              .cursor_pointer()
              .hover((style) => style.bg(t.hover))
              .on_click(() => run("overlay.close"))
              .child("×"),
          ),
      )
      .child(
        v_flex()
          .flex_1()
          .min_h(0)
          .px(14)
          .py(10)
          .gap(8)
          .overflow_y_scrollbar()
          .child(div().text_size(12).text_color(t.dim).child("What's left on each subscription; no money, just the share used."))
          .child(
            v_flex()
              .w_full()
              .py(10)
              .gap(6)
              .border_b(1)
              .border_color(t.border)
              .child(
                h_flex()
                  .gap(8)
                  .items_center()
                  .child(svg("icons/claude.svg").size(16).text_color(t.dim))
                  .child(div().text_size(13.5).font_medium().child("Claude Code")),
              )
              .when(claudeRows.length > 0, (el) => el.children(claudeRows))
              .when(claudeRows.length === 0, (el) => el.child(div().text_size(12.5).text_color(t.dim).child("Claude Code hasn't reported its plan yet."))),
          )
          .children(entries.map((entry) => this.planRow(t, cx, entry)))
          .when(entries.length === 0, (el) => el.child(div().text_size(12.5).text_color(t.dim).child("No worker plans yet."))),
      );
  }

  render(cx) {
    const t = theme();
    if (this.mode === "plans") return this.plans(t, cx);
    if (!this.leadLive()) return this.empty(t);
    return this.compose(t, cx);
  }
}

// A Claude status window as a meter, or null when it has said nothing.
function cableValue(percent, resetAt, clock) {
  if (percent == null && resetAt == null) return null;
  return meter(theme(), clock ? "5H" : "Week", percent, resetWords(resetAt, clock));
}
