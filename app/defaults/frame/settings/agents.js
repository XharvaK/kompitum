// The Agents page: the lead, the usage each CLI reports, and the workers the
// core tries in order. Reads the CLI catalog from programs.json (through
// knownAgents) and the core's own `kom workers --json` and `kom status --json`.

import { div, svg, image } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { kom, exec, knownAgents, pickFolder, window } from "kompitum";
import { dropdown } from "./ui.js";

const BUILTIN = new Set(["cc", "codex", "go", "free"]);
const USAGE_EVERY_MS = 60000;
const DETAIL_LIMIT = 32;

function errorText(error) {
  return String(error?.message ?? error ?? "It didn't work.").trim();
}

function firstLine(text) {
  return String(text ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";
}

// npm installs agent CLIs as `.cmd` shims on Windows; CreateProcess won't
// launch those, so the CLI's own report goes through cmd.exe there.
function execCli(entry, args) {
  const run = entry.run ?? [];
  const command = run[0] ?? entry.key;
  const argv = [...run.slice(1), ...args];
  return window()?.platform === "windows"
    ? exec("cmd.exe", ["/d", "/c", command, ...argv])
    : exec(command, argv);
}

export function ensureAgents(view) {
  const s = (view.agentPage ??= {
    known: null,
    workers: null,
    status: null,
    errors: [],
    busy: false,
    usage: {},
    usageError: {},
    usageAt: {},
    listModels: {},
    leadCli: {},
    leadModel: {},
    leadEffort: {},
    addOpen: false,
    add: {},
    drag: null,
  });
  s.leadCli ??= {};
  s.leadModel ??= {};
  s.leadEffort ??= {};
  return s;
}

export function loadAgents(view, cx, forceUsage = false) {
  if (view.section !== "agents") return;
  const s = ensureAgents(view);
  if (s.busy) return;
  s.busy = true;
  cx.spawn(async (cx) => {
    s.errors = [];
    try {
      s.known = await knownAgents();
    } catch (error) {
      s.known = null;
      s.errors.push(errorText(error));
    }
    for (const [name, args, label] of [
      ["workers", ["workers", "--json"], "kom workers"],
      ["status", ["status", "--json"], "kom status"],
    ]) {
      try {
        const r = await kom(args);
        if (r.code !== 0) throw new Error(firstLine(r.stderr || r.stdout) || `${label} failed`);
        s[name] = JSON.parse(r.stdout || "null");
      } catch (error) {
        s[name] = null;
        s.errors.push(`${label}: ${errorText(error)}`);
      }
    }
    await loadUsage(view, cx, s, forceUsage);
    s.busy = false;
    cx.notify();
  });
}

// The CLI's own usage report, asked when the page opens, then at most once a
// minute. Only a worker CLI's report is drawn, on its worker row.
async function loadUsage(view, cx, s, force) {
  const now = Date.now();
  for (const entry of s.known ?? []) {
    if (!entry.usage?.length) continue;
    if (!(s.workers?.workers ?? []).some((worker) => worker.cli === entry.worker)) continue;
    if (!force && now - (s.usageAt[entry.key] ?? 0) < USAGE_EVERY_MS) continue;
    s.usageAt[entry.key] = now;
    try {
      const r = await execCli(entry, entry.usage);
      if (r.code !== 0) throw new Error("its usage report couldn't be read");
      const meters = parseUsage(r.stdout);
      if (!meters) throw new Error("its usage report has no meters");
      s.usage[entry.key] = meters;
      delete s.usageError[entry.key];
    } catch {
      delete s.usage[entry.key];
      // Only that it failed is kept: a CLI's raw report never reaches the screen.
      s.usageError[entry.key] = true;
    }
  }
}

// One quiet line of the meters a CLI reports, or null when it reports none.
// 80% and over is working; 100% (or the report's own limited flag) is blocked.
function usageMeters(t, meters) {
  const parts = [];
  for (const [label, meter] of [["5H", meters?.rolling], ["Week", meters?.weekly], ["Month", meters?.monthly]]) {
    const percent = meter?.percent;
    if (percent == null || !Number.isFinite(Number(percent))) continue;
    const used = Math.round(Number(percent));
    const color = meter.limited || used >= 100 ? t.blocked : used >= 80 ? t.working : t.dim;
    if (parts.length) parts.push(div().text_color(t.dim).child(" \u00b7 "));
    parts.push(div().text_color(color).child(`${label} ${used}%`));
  }
  return parts.length ? h_flex().text_size(12).text_color(t.dim).children(parts) : null;
}

// Claude's own status report, drawn under the Lead section.
function claudeMeters(t, claude) {
  if (!claude) return null;
  return usageMeters(t, {
    rolling: { percent: claude.five_hour_used_percentage },
    weekly: { percent: claude.seven_day_used_percentage },
  });
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

function routeModel(route) {
  let model = String(route ?? "").split("#")[0];
  if (model.startsWith("commandcode/")) model = model.slice("commandcode/".length);
  return model;
}

function modelLabel(entry, id) {
  return entry?.models?.find((m) => m.id === id)?.label ?? id;
}

function programFor(s, cli) {
  return (s.known ?? []).find((entry) => entry.worker === cli) ?? null;
}

const WINDOWS = [
  { label: "5H", block: "5-hour", clock: true },
  { label: "Week", block: "weekly", clock: false },
  { label: "Month", block: "monthly", clock: false },
];

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

function programIcon(entry, size, t) {
  if (!entry?.icon) return div().w(size).h(size).flex_none();
  return entry.tint
    ? svg(entry.icon).size(size).flex_none().text_color(t.dim)
    : image(entry.icon).w(size).h(size).flex_none();
}

// Nothing is saved until the Owner picks a model; every CLI starts without one.
function defaultModel(entry) {
  return null;
}

function saveLead(view, patch, cx) {
  view.change((person) => {
    const lead = { ...(person.lead ?? {}) };
    for (const [key, value] of Object.entries(patch)) {
      if (value == null || value === "") delete lead[key];
      else lead[key] = value;
    }
    person.lead = lead;
  }, cx);
}

function chooseFolder(view, cx) {
  cx.spawn(async (cx) => {
    const folder = await pickFolder({ prompt: "Choose" });
    if (!folder) return; // a cancelled pick keeps the old choice
    view.change((person) => {
      person.lead = { ...(person.lead ?? {}), folder };
    }, cx);
  });
}

function leadPrograms(s) {
  return (s.known ?? []).filter((entry) => entry.installed && entry.flags?.model?.length);
}

function leadProgram(view, s) {
  const list = leadPrograms(s);
  const chosen = view.person?.lead?.program;
  return list.find((entry) => entry.key === chosen) ?? list.find((entry) => entry.key === "claude") ?? list[0] ?? null;
}

function loadModelList(view, cx, entry) {
  const s = ensureAgents(view);
  if (!entry?.list_models?.length || s.listModels[entry.key]) return;
  s.listModels[entry.key] = { loading: true, lines: [], error: null };
  cx.spawn(async (cx) => {
    try {
      const r = await execCli(entry, entry.list_models);
      if (r.code !== 0) throw new Error(firstLine(r.stderr || r.stdout) || `exit ${r.code}`);
      s.listModels[entry.key] = { loading: false, lines: modelLines(r.stdout), error: null };
    } catch (error) {
      s.listModels[entry.key] = { loading: false, lines: [], error: firstLine(errorText(error)) };
    }
    cx.notify();
  });
}

function modelLines(text) {
  const lines = [];
  for (const raw of String(text ?? "").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.endsWith(":") || /^available models/i.test(line) || /^default model/i.test(line) || /^you are not authenticated/i.test(line)) continue;
    const words = line.replace(/^[-*]\s*/, "");
    const token = words.split(/\s+/)[0];
    if (!/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(token)) continue;
    if (!/[./-]/.test(token)) continue;
    const rest = words.slice(token.length).trim();
    lines.push(rest ? { id: token, label: rest, detail: token } : { id: token, label: token });
  }
  return lines;
}

function cap(word) {
  return String(word ?? "").replace(/^./, (c) => c.toUpperCase());
}

function leadSection(view, t, cx, s) {
  const list = leadPrograms(s);
  if (!list.length) {
    return v_flex().w_full()
      .child(view.heading(t, "Lead"))
      .child(div().py(12).text_size(12.5).text_color(t.dim).child(s.known ? "No installed CLI can lead yet." : "Reading agents…"));
  }
  const entry = leadProgram(view, s);
  if (!entry) return v_flex().w_full().child(view.heading(t, "Lead"));
  const lead = view.person?.lead ?? {};
  const effort = lead.effort;
  const sState = ensureAgents(view);
  const pick = (item, cx) => {
    const next = defaultModel(item);
    const keep = (item.efforts ?? []).includes(effort) ? effort : null;
    saveLead(view, { program: item.key, model: next, effort: keep }, cx);
    cx.notify();
  };
  // The CLIs that can lead, as one dropdown: closed, only the chosen one shows.
  sState.leadCli.accent = view.lit("lead");
  const cliMenu = dropdown(t, "st-lead-cli", {
    items: list.map((item) => ({ value: item.key, label: item.name ?? item.key, icon: item.icon, tint: item.tint })),
    value: entry.key,
    onPick: (key, cx) => {
      const item = list.find((candidate) => candidate.key === key);
      if (item) pick(item, cx);
    },
    state: sState.leadCli,
    width: 300,
  });
  const live = s.listModels[entry.key];
  const modelItems = [];
  const seen = new Set();
  for (const model of entry.models ?? []) {
    if (seen.has(model.id)) continue;
    seen.add(model.id);
    modelItems.push({ value: model.id, label: model.label });
  }
  if (lead.model && !seen.has(lead.model)) {
    seen.add(lead.model);
    modelItems.push({ value: lead.model, label: modelLabel(entry, lead.model) });
  }
  for (const model of live?.lines ?? []) {
    if (seen.has(model.id)) continue;
    seen.add(model.id);
    modelItems.push({ value: model.id, label: model.label, detail: model.detail });
  }
  // A model the lists don't name yet: what's typed in the search can be used as its id.
  const typed = (sState.leadModel.query ?? "").trim();
  if (typed && !seen.has(typed)) modelItems.push({ value: typed, label: typed, detail: "Use this model id" });
  const modelNote = live?.loading
    ? div().pt(4).text_size(12).text_color(t.dim).child(`Reading ${entry.name ?? entry.key}'s models…`)
    : live?.error
      ? div().pt(4).text_size(12).line_height("17px").text_color(t.blocked)
        .child(`Couldn't read ${entry.name ?? entry.key}'s models: ${live.error}`)
      : null;
  const modelMenu = dropdown(t, "st-lead-model", {
    items: modelItems,
    value: lead.model ?? "",
    onPick: (value, cx) => saveLead(view, { model: value }, cx),
    state: sState.leadModel,
    search: true,
    width: 300,
    onOpen: (cx) => loadModelList(view, cx, entry),
  });
  const effortMenu = entry.efforts?.length
    ? dropdown(t, "st-lead-effort", {
        items: [
          { value: "", label: "Its own default" },
          ...(entry.efforts ?? []).map((level) => ({ value: level, label: cap(level) })),
          // A saved level this CLI no longer lists still shows as the pick.
          ...(effort && !entry.efforts.includes(effort) ? [{ value: effort, label: cap(effort) }] : []),
        ],
        value: effort ?? "",
        onPick: (value, cx) => saveLead(view, { effort: value }, cx),
        state: sState.leadEffort,
        width: 200,
      })
    : null;
  // Claude reports its own limits; they belong under the lead only while Claude is the lead.
  const usage = entry.key === "claude" ? claudeMeters(t, s.status?.claude) : null;
  return v_flex().w_full()
    .child(view.heading(t, "Lead"))
    .child(div().pb(8).text_size(12.5).line_height("18px").text_color(t.dim)
      .child("The agent you talk to. It plans, asks you, and hands work to the workers below."))
    .child(cliMenu)
    .when(Boolean(usage), (el) => el.child(div().pt(6).child(usage)))
    .child(view.row(t, "Model", null, modelMenu, "lead_model"))
    .when(Boolean(modelNote), (el) => el.child(modelNote))
    .when(Boolean(effortMenu), (el) => el.child(view.row(t, "Reasoning", null, effortMenu, "lead_model")))
    .child(view.row(
      t,
      "Folder",
      "Where the lead works. A new folder starts a new conversation.",
      h_flex()
        .flex_none()
        .gap(8)
        .items_center()
        .child(div().max_w(340).truncate().text_size(13).text_color(t.muted).child(lead.folder || "Not set"))
        .child(view.button(t, "st-lead-folder", "Choose\u2026", (_e, cx) => chooseFolder(view, cx))),
      "folder",
    ))
    .child(div().pt(8).text_size(12).text_color(t.dim).child("Changes apply to the lead's next conversation."));
}

function workerState(t, worker) {
  if (worker.state === "not_installed") return div().text_size(12).text_color(t.dim).child("Not installed");
  if (worker.state !== "out") return null;
  const window = WINDOWS.find((w) => w.block === worker.window);
  const text = window
    ? `${window.label} exhausted · resets ${resetWords(worker.until_ms, window.clock)}`
    : worker.sentence;
  return div().text_size(12).text_color(t.working).truncate().child(text);
}

function keyButton(t, id, label, onClick) {
  return div().id(id).w(22).h(22).flex_none().flex().items_center().justify_center().rounded(5)
    .text_size(12).text_color(t.dim)
    .hover((s) => s.bg(t.hover).text_color(t.foreground))
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
    .on_click(onClick)
    .child(label);
}

function workerRow(view, t, cx, s, worker, index) {
  const entry = programFor(s, worker.cli) ?? { name: worker.label, icon: null };
  const model = routeModel(worker.route);
  const state = workerState(t, worker);
  const reported = entry.key ? usageMeters(t, s.usage[entry.key]) : null;
  const usage = reported ?? (entry.key && s.usageError[entry.key]
    ? div().text_size(12).text_color(t.dim).child(`${entry.name ?? entry.key}'s usage report couldn't be read.`)
    : null);
  const from = s.drag?.from;
  const row = v_flex()
    .id(`st-worker-${worker.key}`)
    .relative()
    .w_full()
    .py(10)
    .gap(4)
    .border_b(1)
    .border_color(t.border)
    .when(view.lit("workers"), (el) => el.bg(t.selected).rounded(7))
    .when(from === index && s.drag?.over === index, (el) => el.opacity(0.55))
    .on_mouse_down("left", () => { s.drag = { from: index, over: index }; })
    .on_mouse_move((_e, cx) => {
      if (s.drag && s.drag.over !== index) {
        s.drag.over = index;
        cx.notify();
      }
    })
    .on_mouse_up("left", (_e, cx) => {
      const drag = s.drag;
      s.drag = null;
      if (drag && drag.from !== index) moveWorker(view, drag.from, index, cx);
      cx.notify();
    })
    .when(from != null && from !== index && s.drag?.over === index, (el) => el.child(
      div().absolute().left(0).right(0).h(2).bg(t.accent)
        .when(from < index, (line) => line.bottom(0))
        .when(from > index, (line) => line.top(0)),
    ))
    .child(h_flex().gap(8).items_center()
      .child(programIcon(entry, 16, t))
      .child(v_flex().flex_1().min_w(0).gap(2)
        .child(div().text_size(13.5).text_color(t.foreground).truncate().child(worker.label))
        .child(div().text_size(12).text_color(t.dim).truncate().child(modelLabel(entry, model))))
      .when(Boolean(state), (el) => el.child(state.flex_none()))
      .child(keyButton(t, `st-worker-up-${worker.key}`, "↑", (_e, cx) => workerStep(view, index, -1, cx)))
      .child(keyButton(t, `st-worker-down-${worker.key}`, "↓", (_e, cx) => workerStep(view, index, 1, cx)))
      .when(!BUILTIN.has(worker.key), (el) => el.child(keyButton(t, `st-worker-remove-${worker.key}`, "×", (_e, cx) => removeWorker(view, worker, cx)))));
  if (usage) row.child(usage);
  return row;
}

function subagentRow(view, t, cx, s, name, index) {
  const claude = (s.known ?? []).find((entry) => entry.key === "claude") ?? { icon: "icons/claude.svg", tint: false };
  const row = v_flex()
    .id(`st-subagent-${name}`)
    .relative()
    .w_full()
    .py(10)
    .gap(4)
    .border_b(1)
    .border_color(t.border)
    .when(view.lit("workers"), (el) => el.bg(t.selected).rounded(7))
    .on_mouse_down("left", () => { s.drag = { from: `sub:${index}`, over: `sub:${index}` }; })
    .on_mouse_move((_e, cx) => {
      if (s.drag && s.drag.over !== `sub:${index}`) {
        s.drag.over = `sub:${index}`;
        cx.notify();
      }
    })
    .on_mouse_up("left", (_e, cx) => {
      const drag = s.drag;
      s.drag = null;
      if (drag && drag.from !== `sub:${index}`) moveWorker(view, drag.from, `sub:${index}`, cx);
      cx.notify();
    })
    .child(h_flex().gap(8).items_center()
      .child(programIcon(claude, 16, t))
      .child(v_flex().flex_1().min_w(0).gap(2)
        .child(div().text_size(13.5).text_color(t.foreground).child("Claude Code"))
        .child(div().text_size(12).text_color(t.dim).child(name)))
      .child(keyButton(t, `st-subagent-up-${name}`, "↑", (_e, cx) => workerStep(view, `sub:${index}`, -1, cx)))
      .child(keyButton(t, `st-subagent-down-${name}`, "↓", (_e, cx) => workerStep(view, `sub:${index}`, 1, cx)))
      .child(keyButton(t, `st-subagent-remove-${name}`, "×", (_e, cx) => removeSubagent(view, name, cx))));
  return row;
}

function orderedList(s) {
  return (s.workers?.order ?? []).map((key) => (s.workers?.workers ?? []).find((w) => w.key === key)).filter(Boolean);
}

function subagents(view) {
  const program = view.person?.lead?.program ?? "claude";
  return program === "claude" ? (view.person?.lead?.subagents ?? []) : [];
}

function moveWorker(view, from, to, cx) {
  const s = ensureAgents(view);
  if (typeof from === "string" && typeof to === "string" && from.startsWith("sub:") && to.startsWith("sub:")) {
    const subs = [...subagents(view)];
    const a = Number(from.slice(4));
    const b = Number(to.slice(4));
    if (a < 0 || b < 0 || a >= subs.length || b >= subs.length) return;
    subs.splice(b, 0, subs.splice(a, 1)[0]);
    view.change((person) => {
      person.lead = { ...(person.lead ?? {}), subagents: subs };
    }, cx);
    return;
  }
  if (typeof from !== "number" || typeof to !== "number") return;
  const order = [...(s.workers?.order ?? [])];
  if (from < 0 || to < 0 || from >= order.length || to >= order.length) return;
  order.splice(to, 0, order.splice(from, 1)[0]);
  komCall(view, ["workers", "--order", order.join(",")], cx);
}

function workerStep(view, index, delta, cx) {
  const s = ensureAgents(view);
  if (typeof index === "string") {
    const subs = subagents(view);
    const at = Number(index.slice(4));
    const to = at + delta;
    if (to < 0 || to >= subs.length) return;
    moveWorker(view, index, `sub:${to}`, cx);
    return;
  }
  const order = s.workers?.order ?? [];
  const to = index + delta;
  if (to < 0 || to >= order.length) return;
  moveWorker(view, index, to, cx);
}

function removeWorker(view, worker, cx) {
  komCall(view, ["workers", "--remove", worker.key], cx);
}

function removeSubagent(view, name, cx) {
  view.change((person) => {
    person.lead = { ...(person.lead ?? {}), subagents: (person.lead?.subagents ?? []).filter((n) => n !== name) };
  }, cx);
}

function komCall(view, args, cx) {
  cx.spawn(async (cx) => {
    const s = ensureAgents(view);
    try {
      const r = await kom(args);
      if (r.code !== 0) {
        let message = firstLine(r.stderr || r.stdout);
        try { message = JSON.parse(r.stdout || r.stderr).error?.message ?? message; } catch {}
        throw new Error(message || `kom ${args[0]} failed`);
      }
      s.busy = false;
      loadAgents(view, cx);
    } catch (error) {
      s.errors = [errorText(error)];
    }
    cx.notify();
  });
}

function slug(text) {
  return String(text).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, DETAIL_LIMIT) || "worker";
}

function uniqueKey(s, base) {
  const taken = new Set((s.workers?.workers ?? []).map((w) => w.key));
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) {
    const tail = `-${n}`;
    const key = base.slice(0, DETAIL_LIMIT - tail.length) + tail;
    if (!taken.has(key)) return key;
  }
}

function addRoute(entry, model, effort) {
  return String(entry.route ?? "{model}#{effort}").replace("{model}", model).replace("{effort}", effort ?? "");
}

function effortWord(id) {
  return ({ low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max" })[id]
    ?? (id ? id.charAt(0).toUpperCase() + id.slice(1) : "");
}

function workerClis(s) {
  return (s.known ?? []).filter((entry) => entry.installed && entry.worker);
}

function firstAddPicks(entry) {
  return {
    cli: entry?.key ?? null,
    model: entry?.models?.[0]?.id ?? null,
    effort: entry?.efforts?.includes("max") ? "max" : entry?.efforts?.[0] ?? null,
  };
}

function readyDetail(key) {
  if (key === "free") return "Free models through OpenCode.";
  if (key === "codex") return "Codex with its own default model.";
  if (key === "sonnet") return "A subagent inside the lead's Claude Code, on Sonnet.";
  if (key === "haiku") return "A subagent inside the lead's Claude Code, on Haiku.";
  return null;
}

function failLine(t, text) {
  const message = firstLine(text);
  if (!message) return null;
  return div()
    .text_size(12)
    .text_color(t.blocked)
    .line_height("16px")
    .max_h(32)
    .overflow_hidden()
    .tooltip(message)
    .child(message);
}

function parseKomError(r, args) {
  let message = firstLine(r.stderr || r.stdout);
  try { message = JSON.parse(r.stdout || r.stderr).error?.message ?? message; } catch {}
  return firstLine(message) || `kom ${args[0]} failed`;
}

function addReadyRow(t, s, { key, name, iconEntry, onAdd }) {
  const detail = readyDetail(key);
  const busy = s.addBusy === key;
  return h_flex()
    .id(`st-add-ready-${key}`)
    .w_full()
    .h(44)
    .px(10)
    .gap(10)
    .items_center()
    .rounded(8)
    .hover((el) => el.bg(t.hover))
    .child(programIcon(iconEntry, 16, t))
    .child(
      v_flex()
        .flex_1()
        .min_w(0)
        .child(div().text_size(13.5).text_color(t.foreground).truncate().child(name))
        .when(Boolean(detail), (el) => el.child(div().text_size(12).text_color(t.dim).truncate().child(detail))),
    )
    .child(
      h_flex()
        .h(26)
        .px(8)
        .flex_none()
        .items_center()
        .rounded(6)
        .cursor_pointer()
        .text_size(12.5)
        .text_color(t.foreground)
        .hover((el) => el.bg(t.hover))
        .on_click((_e, cx) => {
          if (s.addBusy) return;
          onAdd(cx);
        })
        .child(busy ? "Adding…" : "Add"),
    );
}

function addPanel(view, t, cx, s) {
  const order = s.workers?.order ?? [];
  const builtin = (s.workers?.workers ?? []).filter((w) => BUILTIN.has(w.key) && !order.includes(w.key));
  const entries = workerClis(s);
  const subs = new Set(subagents(view));
  const subChoices = leadProgram(view, s)?.key === "claude"
    ? [["sonnet", "Claude Code with Sonnet"], ["haiku", "Claude Code with Haiku"]].filter(([name]) => !subs.has(name))
    : [];
  const ready = [
    ...builtin.map((worker) => ({
      key: worker.key,
      name: worker.label ?? worker.key,
      iconEntry: programFor(s, worker.cli),
      onAdd: (cx) => {
        s.addBusy = worker.key;
        s.addReadyError = null;
        cx.notify();
        cx.spawn(async (cx) => {
          const args = ["workers", "--order", [...(s.workers?.order ?? []), worker.key].join(",")];
          try {
            const r = await kom(args);
            if (r.code !== 0) throw new Error(parseKomError(r, args));
            s.busy = false;
            loadAgents(view, cx);
          } catch (error) {
            s.addReadyError = firstLine(errorText(error));
          }
          s.addBusy = null;
          cx.notify();
        });
      },
    })),
    ...subChoices.map(([name, label]) => ({
      key: name,
      name: label,
      iconEntry: leadProgram(view, s),
      onAdd: (cx) => {
        view.change((person) => {
          const lead = { ...(person.lead ?? {}) };
          lead.subagents = [...(lead.subagents ?? []), name];
          person.lead = lead;
        }, cx);
      },
    })),
  ];
  if (!s.addForm || !entries.some((entry) => entry.key === s.addForm.cli)) {
    s.addForm = firstAddPicks(entries[0]);
  }
  s.addCli ??= {};
  s.addModel ??= {};
  s.addEffort ??= {};
  const form = s.addForm;
  const entry = entries.find((item) => item.key === form?.cli) ?? entries[0] ?? null;

  const close = h_flex()
    .id("st-add-close")
    .w(24)
    .h(24)
    .flex_none()
    .items_center()
    .justify_center()
    .rounded(6)
    .cursor_pointer()
    .text_color(t.muted)
    .hover((el) => el.bg(t.hover).text_color(t.foreground))
    .tooltip("Close")
    .on_click((_e, cx) => { s.addOpen = false; cx.notify(); })
    .child(svg("frame/settings/close.svg").size(14));

  const header = (title) => div().text_size(11.5).font_semibold().text_color(t.dim).child(title);

  let card = v_flex()
    .id("st-add")
    .w_full()
    .rounded(12)
    .border_1()
    .border_color(t.border)
    .bg(t.surface)
    .p(14)
    .gap(12)
    .child(
      h_flex()
        .w_full()
        .items_center()
        .gap(10)
        .child(div().flex_1().min_w(0).child(header(ready.length ? "Ready to add" : "Choose your own")))
        .child(close),
    );
  if (ready.length) {
    card = card
      .child(v_flex().w_full().gap(4).children(ready.map((item) => addReadyRow(t, s, item))))
      .when(Boolean(s.addReadyError), (el) => el.child(failLine(t, s.addReadyError)))
      .child(header("Choose your own"));
  }
  if (!entries.length) {
    return card.child(
      div().text_size(12).text_color(t.dim).child("No other worker CLI is installed. Install Codex or OpenCode to add one."),
    );
  }

  const pickCli = (key, cx) => {
    s.addForm = firstAddPicks(entries.find((item) => item.key === key) ?? entries[0]);
    s.addNote = null;
    cx.notify();
  };
  const addOwn = (_e, cx) => {
    if (s.addBusy || !entry) return;
    const model = form.model ?? "";
    const effort = form.effort;
    const label = `${entry.name ?? entry.key} · ${modelLabel(entry, model)}`;
    const key = uniqueKey(s, slug(label));
    const args = ["workers", "--add", key, "--cli", entry.worker, "--route", addRoute(entry, model, effort), "--label", label];
    s.addBusy = "own";
    s.addOwnError = null;
    s.addNote = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await kom(args);
        if (r.code !== 0) throw new Error(parseKomError(r, args));
        s.addNote = `Added ${label}.`;
        s.busy = false;
        loadAgents(view, cx);
      } catch (error) {
        s.addOwnError = firstLine(errorText(error));
      }
      s.addBusy = null;
      cx.notify();
    });
  };

  card = card.child(
    h_flex()
      .w_full()
      .gap(8)
      .items_center()
      .flex_wrap()
      .child(dropdown(t, "st-add-cli", {
        items: entries.map((item) => ({ value: item.key, label: item.name ?? item.key, icon: item.icon, tint: item.tint })),
        value: form.cli,
        onPick: pickCli,
        state: s.addCli,
        width: 200,
      }))
      .child(dropdown(t, "st-add-model", {
        items: (entry?.models ?? []).map((m) => ({
          value: m.id,
          label: m.label ?? m.id,
          detail: m.label && m.label !== m.id ? m.id : undefined,
        })),
        value: form.model,
        onPick: (id, cx) => { form.model = id; s.addNote = null; cx.notify(); },
        state: s.addModel,
        search: true,
        width: 240,
      }))
      .when((entry?.efforts ?? []).length > 0, (el) => el.child(dropdown(t, "st-add-effort", {
        items: (entry?.efforts ?? []).map((level) => ({ value: level, label: effortWord(level) })),
        value: form.effort,
        onPick: (id, cx) => { form.effort = id; cx.notify(); },
        state: s.addEffort,
        width: 160,
      })))
      .child(
        h_flex()
          .id("st-add-go")
          .h(30)
          .px(14)
          .flex_none()
          .items_center()
          .justify_center()
          .rounded(8)
          .bg(t.solid)
          .text_color(t.solid_foreground)
          .cursor_pointer()
          .hover((el) => el.opacity(0.88))
          .on_click(addOwn)
          .child(s.addBusy === "own" ? "Adding…" : "Add"),
      ),
  );
  card = card.when(Boolean(s.addNote), (el) => el.child(div().text_size(12).text_color(t.dim).child(s.addNote)));
  card = card.when(Boolean(s.addOwnError), (el) => el.child(failLine(t, s.addOwnError)));
  return card;
}

// A day as `kom workers` words it: today, a weekday within a week, else "12 Oct" (and the year when not this one).
function dayWords(ms) {
  const at = new Date(Number(ms));
  const now = new Date();
  const midnight = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((midnight(at) - midnight(now)) / 86400000);
  if (days === 0) return "today";
  if (Math.abs(days) < 7) return ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][at.getDay()];
  const month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][at.getMonth()];
  return `${at.getDate()} ${month}${at.getFullYear() !== now.getFullYear() ? ` ${at.getFullYear()}` : ""}`;
}

function cliLine(cli, now) {
  const name = cli.version ? `${cli.label} ${cli.version}` : cli.label;
  if (cli.waiting) return `${name} · waiting: ${cli.waiting}`;
  const when = dayWords(cli.updated_ms > 0 ? cli.updated_ms : cli.tried_ms);
  if (cli.updated_ms > 0) {
    const changed = cli.before && cli.before !== cli.version
      ? `${name} · updated ${when} (was ${cli.before})`
      : `${name} · updated ${when}`;
    return cli.error ? `${changed}: ${cli.error}` : changed;
  }
  if (cli.error) return `${name} · update failed: ${cli.error}`;
  if (cli.tried_ms > 0) return `${name} · up to date, checked ${when}`;
  return `${name} · not checked yet`;
}

function workersSection(view, t, cx, s) {
  const order = orderedList(s);
  const subs = subagents(view);
  const rows = [];
  for (let i = 0; i < order.length; i++) rows.push(workerRow(view, t, cx, s, order[i], i));
  for (let i = 0; i < subs.length; i++) rows.push(subagentRow(view, t, cx, s, subs[i], i));
  const clis = Array.isArray(s.workers?.clis) ? s.workers.clis : [];
  return v_flex().w_full()
    .child(view.heading(t, "Workers"))
    .child(div().pb(6).text_size(12.5).text_color(t.dim).child("Tried top to bottom; drag or use the arrows."))
    .when(rows.length > 0, (el) => el.children(rows))
    .when(rows.length === 0, (el) => el.child(div().py(10).text_size(12.5).text_color(t.dim).child(s.workers ? "No workers in the order." : "Reading workers…")))
    .child(h_flex().pt(10).child(view.button(t, "st-add-worker", "Add a worker", (_e, cx) => { s.addOpen = !s.addOpen; cx.notify(); })))
    .when(s.addOpen, (el) => el.child(addPanel(view, t, cx, s)))
    .when(clis.length > 0, (el) => el.child(v_flex().pt(12).gap(3).when(view.lit("clis"), (line) => line.bg(t.selected).rounded(7))
      .children(clis.map((cli) => {
        const line = cliLine(cli, Date.now());
        return div().text_size(12.5).text_color(t.dim).tooltip(line).child(firstLine(line) || line);
      }))));
}

export function agentsPage(view, t, cx) {
  const s = ensureAgents(view);
  return v_flex().w_full()
    .when(s.errors.length > 0, (el) => el.children(s.errors.map((error) =>
      div().pb(8).text_size(12.5).line_height("18px").text_color(t.blocked).tooltip(error).child(firstLine(error) || error))))
    .child(leadSection(view, t, cx, s))
    .child(div().h(24))
    .child(workersSection(view, t, cx, s));
}
