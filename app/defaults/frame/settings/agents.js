// The Agents page: plans, the lead, and the workers the core tries in order.
// Reads the CLI catalog from programs.json (through knownAgents) and the
// core's own `kom workers --json` and `kom status --json`.

import { div, svg, image } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { kom, exec, knownAgents, window } from "kompitum";

const BUILTIN = new Set(["cc", "codex", "go", "free"]);
const PLANS_EVERY_MS = 60000;
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
  view.agentPage ??= {
    known: null,
    workers: null,
    status: null,
    errors: [],
    busy: false,
    usage: {},
    usageError: {},
    usageAt: {},
    listModels: {},
    modelOpen: false,
    effortOpen: false,
    otherModel: false,
    addOpen: false,
    add: {},
    drag: null,
  };
  return view.agentPage;
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

// The CLI's own report wins over Kompitum's estimate. Asked when the page
// opens, then at most once a minute.
async function loadUsage(view, cx, s, force) {
  const now = Date.now();
  for (const entry of s.known ?? []) {
    if (!entry.usage?.length || !planEntries(s).some((p) => p.key === entry.key)) continue;
    if (!force && now - (s.usageAt[entry.key] ?? 0) < PLANS_EVERY_MS) continue;
    s.usageAt[entry.key] = now;
    try {
      const r = await execCli(entry, entry.usage);
      if (r.code !== 0) throw new Error(firstLine(r.stderr || r.stdout) || `exit ${r.code}`);
      const meters = parseUsage(r.stdout);
      if (!meters) throw new Error(firstLine(r.stdout) || "its report has no usage meters");
      s.usage[entry.key] = meters;
      delete s.usageError[entry.key];
    } catch (error) {
      delete s.usage[entry.key];
      s.usageError[entry.key] = firstLine(errorText(error));
    }
  }
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

function workersFor(s, entry) {
  return (s.workers?.workers ?? []).filter((worker) => worker.cli === entry.worker);
}

function orderedWorkers(s, entry) {
  const all = workersFor(s, entry);
  const byKey = new Map(all.map((w) => [w.key, w]));
  return (s.workers?.order ?? []).map((key) => byKey.get(key)).filter(Boolean);
}

function planEntries(s) {
  const seen = new Set();
  const entries = [];
  for (const key of s.workers?.order ?? []) {
    const worker = (s.workers?.workers ?? []).find((w) => w.key === key);
    const entry = worker ? programFor(s, worker.cli) : null;
    if (entry?.plans?.length && !seen.has(entry.key)) {
      seen.add(entry.key);
      entries.push(entry);
    }
  }
  return entries;
}

const WINDOWS = [
  { label: "5H", spend: "five_hours", report: "rolling", block: "5-hour", clock: true },
  { label: "Week", spend: "week", report: "weekly", block: "weekly", clock: false },
  { label: "Month", spend: "month", report: "monthly", block: "monthly", clock: false },
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

function planLimit(plan, model, label) {
  if (plan?.models) return plan.models?.[model]?.[label];
  return plan?.windows?.[label];
}

function planWindows(s, entry, plan, external, workers, t) {
  const first = orderedWorkers(s, entry)[0] ?? workers[0];
  const model = first ? routeModel(first.route) : null;
  const out = [];
  for (const window of WINDOWS) {
    const blocked = workers.find((w) => w.state === "out" && w.window === window.block);
    if (blocked) {
      out.push(meter(t, window.label, 100, resetWords(blocked.until_ms, window.clock), { exhausted: true }));
      continue;
    }
    const report = external?.[window.report];
    if (Number.isFinite(Number(report?.percent))) {
      out.push(meter(t, window.label, report.percent, resetWords(report.resetsAt, window.clock)));
      continue;
    }
    const limit = planLimit(plan, model, window.label);
    if (!Number.isFinite(Number(limit)) || Number(limit) <= 0) continue;
    const spent = workers.reduce((sum, worker) => sum + Number(worker.spend?.[window.spend]?.cost_usd ?? 0), 0);
    out.push(meter(t, window.label, (spent / Number(limit)) * 100, null, { approx: true }));
  }
  return out;
}

function claudeRow(t, claude, known) {
  const entry = (known ?? []).find((a) => a.key === "claude") ?? { icon: "icons/claude.svg", tint: false };
  const windows = [];
  if (claude) {
    const five = claude.five_hour_used_percentage;
    const fiveAt = resetWords(claude.five_hour_resets_at, true);
    if (five != null || fiveAt) windows.push(meter(t, "5H", five, fiveAt));
    const week = claude.seven_day_used_percentage;
    const weekAt = resetWords(claude.seven_day_resets_at, false);
    if (week != null || weekAt) windows.push(meter(t, "Week", week, weekAt));
  }
  return v_flex().w_full().py(12).gap(8).border_b(1).border_color(t.border)
    .child(h_flex().gap(8).items_center()
      .child(programIcon(entry, 16, t))
      .child(div().text_size(14).text_color(t.foreground).child("Claude Code")))
    .when(windows.length > 0, (el) => el.children(windows))
    .when(windows.length === 0, (el) => el.child(div().text_size(12.5).text_color(t.dim).child("Claude Code hasn't reported its plan yet.")));
}

function planRow(view, t, cx, s, entry) {
  const selected = view.person?.plans?.[entry.key];
  const plan = (entry.plans ?? []).find((p) => p.id === selected);
  const workers = workersFor(s, entry);
  const head = h_flex().gap(8).items_center()
    .child(programIcon(entry, 16, t))
    .child(div().text_size(14).text_color(t.foreground).child(entry.name ?? entry.key));
  const choices = view.segmented(
    t,
    `st-plan-${entry.key}`,
    (entry.plans ?? []).map((p) => [p.id, p.label]),
    selected,
    (id, cx) => {
      view.change((person) => {
        person.plans = { ...(person.plans ?? {}), [entry.key]: id };
      }, cx);
    },
  );
  const external = s.usage[entry.key];
  const error = s.usageError[entry.key];
  const others = workers
    .filter((worker) => worker.state === "out" && !WINDOWS.some((window) => window.block === worker.window))
    .map((worker) => worker.sentence)
    .filter(Boolean);
  return v_flex().w_full().py(12).gap(8).border_b(1).border_color(t.border)
    .child(head)
    .when((entry.plans ?? []).length > 0, (el) => el.child(choices))
    .when(!plan, (el) => el.child(div().text_size(12.5).text_color(t.dim).child("Pick this CLI's plan.")))
    .when(Boolean(plan), (el) => el.children(planWindows(s, entry, plan, external, workers, t)))
    .children(others.map((sentence) => div().text_size(12.5).line_height("18px").text_color(t.working).child(sentence)))
    .when(Boolean(plan && error), (el) => el.child(div().text_size(12.5).line_height("18px").text_color(t.dim)
      .child(`${entry.name ?? entry.key} can't report its plan yet (${error}), so these are Kompitum's own estimates.`)));
}

function programIcon(entry, size, t) {
  if (!entry?.icon) return div().w(size).h(size).flex_none();
  return entry.tint
    ? svg(entry.icon).size(size).flex_none().text_color(t.dim)
    : image(entry.icon).w(size).h(size).flex_none();
}

function chip(t, id, label, on, onClick) {
  return div()
    .id(id)
    .px(10)
    .py(5)
    .rounded(7)
    .border_1()
    .border_color(on ? t.accent : t.border)
    .bg(on ? t.selected : t.raised)
    .cursor_pointer()
    .text_size(13)
    .text_color(on ? t.foreground : t.muted)
    .hover((s) => s.bg(t.hover))
    .on_click(onClick)
    .child(label);
}

// Claude's `default` alias; other CLIs keep the model their own config names until one is picked.
function defaultModel(entry) {
  return entry?.key === "claude" ? "default" : null;
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
    const token = line.replace(/^[-*]\s*/, "").split(/\s+/)[0];
    if (!/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(token)) continue;
    if (!/[./-]/.test(token)) continue;
    lines.push({ id: token, label: line.replace(/^[-*]\s*/, "") });
  }
  return lines;
}

function modelPicker(view, t, cx, entry) {
  const s = ensureAgents(view);
  const models = entry.models ?? [];
  const groups = new Map();
  for (const model of models) {
    const provider = model.id.includes("/") ? model.id.split("/")[0] : "Other";
    if (!groups.has(provider)) groups.set(provider, []);
    groups.get(provider).push(model);
  }
  const list = v_flex().w_full().gap(4);
  for (const [provider, items] of groups) {
    list.child(div().pt(4).text_size(11).font_semibold().text_color(t.dim).child(provider.charAt(0).toUpperCase() + provider.slice(1)));
    for (const model of items) {
      list.child(div().id(`st-lead-model-${model.id}`).px(10).py(6).rounded(6).cursor_pointer().text_size(13).text_color(t.foreground)
        .hover((el) => el.bg(t.hover))
        .on_click((_e, cx) => { s.modelOpen = false; saveLead(view, { model: model.id }, cx); cx.notify(); })
        .child(model.label));
    }
  }
  const live = s.listModels[entry.key];
  if (entry.list_models?.length) {
    list.child(div().pt(4).text_size(11).font_semibold().text_color(t.dim).child("From the CLI"));
    if (!live || live.loading) list.child(div().text_size(12.5).text_color(t.dim).child("Reading models…"));
    else if (live.error) list.child(div().text_size(12.5).text_color(t.blocked).child(live.error));
    else {
      for (const model of live.lines) {
        list.child(div().id(`st-lead-live-${model.id}`).px(10).py(6).rounded(6).cursor_pointer().text_size(13).text_color(t.foreground)
          .hover((el) => el.bg(t.hover))
          .on_click((_e, cx) => { s.modelOpen = false; saveLead(view, { model: model.id }, cx); cx.notify(); })
          .child(model.label));
      }
    }
  }
  list.child(div().id("st-lead-other").px(10).py(6).rounded(6).cursor_pointer().text_size(13).text_color(t.muted)
    .hover((el) => el.bg(t.hover))
    .on_click((_e, cx) => {
      s.otherModel = true;
      s.otherInput ??= InputState.new({ placeholder: "Model id" });
      s.otherInput.on("submit", (_e, cx) => {
        const id = s.otherInput.value().trim();
        if (id) saveLead(view, { model: id }, cx);
        s.otherModel = false;
        cx.notify();
      });
      s.otherInput.set_value("");
      s.otherInput.focus();
      cx.notify();
    })
    .child("Other…"));
  if (s.otherModel) {
    list.child(div().px(4).py(4).child(
      Input.new(s.otherInput).w_full().h(30).px(9).rounded(7).border_1().border_color(t.input).bg(t.background).text_size(13).text_color(t.foreground),
    ));
  }
  return list;
}

function cap(word) {
  return String(word ?? "").replace(/^./, (c) => c.toUpperCase());
}

function effortPicker(view, t, cx, entry, selected) {
  return v_flex()
    .w_full()
    .gap(4)
    .children(
      (entry.efforts ?? []).map((level) =>
        div()
          .id(`st-lead-effort-${level}`)
          .px(10)
          .py(6)
          .rounded(6)
          .cursor_pointer()
          .text_size(13)
          .text_color(t.foreground)
          .when(level === selected, (el) => el.bg(t.selected))
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => {
            const state = ensureAgents(view);
            state.effortOpen = false;
            saveLead(view, { effort: level }, cx);
            cx.notify();
          })
          .child(cap(level)),
      ),
    );
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
  const model = lead.model ?? defaultModel(entry);
  const effort = lead.effort;
  const sState = ensureAgents(view);
  const pick = (item, cx) => {
    const next = defaultModel(item);
    const keep = (item.efforts ?? []).includes(effort) ? effort : null;
    saveLead(view, { program: item.key, model: next, effort: keep }, cx);
    sState.modelOpen = false;
    sState.effortOpen = false;
    sState.otherModel = false;
    cx.notify();
  };
  // The CLIs that can lead, as one list you pick from: the chosen one has the check.
  const cliList = v_flex()
    .id("st-lead-cli")
    .w_full()
    .rounded(10)
    .border_1()
    .border_color(t.border)
    .bg(t.surface)
    .overflow_hidden()
    .when(view.lit("lead"), (el) => el.border_color(t.accent))
    .children(list.map((item, index) => {
      const on = item.key === entry.key;
      return h_flex()
        .id(`st-lead-cli-${item.key}`)
        .w_full()
        .h(40)
        .px(12)
        .gap(10)
        .items_center()
        .cursor_pointer()
        .when(index > 0, (el) => el.border_t(1).border_color(t.border))
        .hover((el) => el.bg(t.hover))
        .on_click((_e, cx) => pick(item, cx))
        .child(programIcon(item, 16, t))
        .child(div().flex_1().min_w(0).truncate().text_size(13.5).text_color(on ? t.foreground : t.muted)
          .when(on, (el) => el.font_medium()).child(item.name ?? item.key))
        .when(on, (el) => el.child(svg("frame/settings/check.svg").size(14).flex_none().text_color(t.accent)));
    }));
  const menuButton = (id, label, open, onClick) =>
    h_flex()
      .id(id)
      .h(28)
      .pl(10)
      .pr(8)
      .gap(6)
      .items_center()
      .rounded(7)
      .border_1()
      .border_color(open ? t.accent : t.input)
      .bg(t.raised)
      .cursor_pointer()
      .text_size(13)
      .text_color(t.foreground)
      .hover((el) => el.bg(t.hover))
      .on_click(onClick)
      .child(label)
      .child(svg("mods/workspace-chrome/chevron-down.svg").size(12).text_color(t.dim));
  const modelMenu = menuButton("st-lead-model", model ? modelLabel(entry, model) : "Its own default", sState.modelOpen, (_e, cx) => {
    sState.modelOpen = !sState.modelOpen;
    sState.effortOpen = false;
    if (sState.modelOpen) loadModelList(view, cx, entry);
    cx.notify();
  });
  const effortMenu = entry.efforts?.length
    ? menuButton("st-lead-effort", effort ? cap(effort) : "Its own default", sState.effortOpen, (_e, cx) => {
        sState.effortOpen = !sState.effortOpen;
        sState.modelOpen = false;
        cx.notify();
      })
    : null;
  const card = (child) => div().w_full().mt(6).p(4).rounded(10).border_1().border_color(t.border).bg(t.surface).child(child);
  return v_flex().w_full()
    .child(view.heading(t, "Lead"))
    .child(div().pb(8).text_size(12.5).line_height("18px").text_color(t.dim)
      .child("The agent you talk to. It plans, asks you, and hands work to the workers below."))
    .child(cliList)
    .child(view.row(t, "Model", null, modelMenu, "lead_model"))
    .when(Boolean(sState.modelOpen), (el) => el.child(card(modelPicker(view, t, cx, entry))))
    .when(Boolean(effortMenu), (el) => el.child(view.row(t, "Reasoning", null, effortMenu, "lead_model")))
    .when(Boolean(sState.effortOpen), (el) => el.child(card(effortPicker(view, t, cx, entry, effort))))
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

function addPanel(view, t, cx, s) {
  const order = s.workers?.order ?? [];
  const builtin = (s.workers?.workers ?? []).filter((w) => BUILTIN.has(w.key) && !order.includes(w.key));
  const entries = (s.known ?? []).filter((entry) => entry.installed && entry.worker);
  const subs = new Set(subagents(view));
  const parts = [];
  if (builtin.length) {
    parts.push(v_flex().gap(4)
      .child(div().text_size(11).font_semibold().text_color(t.dim).child("Built-in workers"))
      .children(builtin.map((worker) => h_flex().gap(8).items_center().py(4)
        .child(div().flex_1().text_size(13).text_color(t.foreground).child(worker.label))
        .child(view.button(t, `st-add-builtin-${worker.key}`, "Add", (_e, cx) => komCall(view, ["workers", "--order", [...order, worker.key].join(",")], cx))))));
  }
  for (const entry of entries) {
    const state = s.add[entry.key] ??= {
      model: defaultModel(entry),
      effort: entry.efforts?.includes("max") ? "max" : entry.efforts?.[0] ?? null,
    };
    const model = state.model ?? defaultModel(entry) ?? "";
    const effort = state.effort ?? entry.efforts?.[0] ?? null;
    parts.push(v_flex().gap(5).py(6)
      .child(h_flex().gap(8).items_center()
        .child(programIcon(entry, 16, t))
        .child(div().text_size(13).text_color(t.foreground).child(entry.name ?? entry.key)))
      .child(h_flex().gap(6).flex_wrap()
        .children((entry.models ?? []).map((m) =>
          chip(t, `st-add-model-${entry.key}-${m.id}`, m.label, m.id === model, (_e, cx) => { state.model = m.id; cx.notify(); }))))
      .when((entry.efforts ?? []).length > 0, (el) => el.child(h_flex().gap(6).flex_wrap()
        .children((entry.efforts ?? []).map((level) =>
          chip(t, `st-add-effort-${entry.key}-${level}`, level, level === effort, (_e, cx) => { state.effort = level; cx.notify(); })))))
      .child(h_flex().gap(6).items_center()
        .child(view.button(t, `st-add-worker-${entry.key}`, "Add worker", (_e, cx) => {
          const label = `${entry.name ?? entry.key} · ${modelLabel(entry, model)}`;
          const key = uniqueKey(s, slug(label));
          komCall(view, ["workers", "--add", key, "--cli", entry.worker, "--route", addRoute(entry, model, effort), "--label", label], cx);
        }))));
  }
  if (leadProgram(view, s)?.key === "claude") {
    const choices = [["sonnet", "Claude Code with Sonnet"], ["haiku", "Claude Code with Haiku"]].filter(([name]) => !subs.has(name));
    if (choices.length) {
      parts.push(v_flex().gap(4)
        .child(div().text_size(11).font_semibold().text_color(t.dim).child("Claude subagents"))
        .children(choices.map(([name, label]) => h_flex().gap(8).items_center().py(4)
          .child(div().flex_1().text_size(13).text_color(t.foreground).child(label))
          .child(view.button(t, `st-add-subagent-${name}`, "Add", (_e, cx) => {
            view.change((person) => {
              const lead = { ...(person.lead ?? {}) };
              lead.subagents = [...(lead.subagents ?? []), name];
              person.lead = lead;
            }, cx);
          })))));
    }
  }
  return v_flex().w_full().pt(12).gap(4)
    .child(div().text_size(12).font_semibold().text_color(t.muted).child("Add a worker"))
    .children(parts.length ? parts : [div().text_size(12.5).text_color(t.dim).child("Every available CLI is already in the order.")])
    .child(h_flex().pt(6).child(view.button(t, "st-add-close", "Done", (_e, cx) => { s.addOpen = false; cx.notify(); })));
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
      .children(clis.map((cli) => div().text_size(12.5).text_color(t.dim).child(cliLine(cli, Date.now()))))));
}

export function agentsPage(view, t, cx) {
  const s = ensureAgents(view);
  return v_flex().w_full()
    .when(s.errors.length > 0, (el) => el.children(s.errors.map((error) =>
      div().pb(8).text_size(12.5).line_height("18px").text_color(t.blocked).child(error))))
    .child(leadSection(view, t, cx, s))
    .child(div().h(24))
    .child(workersSection(view, t, cx, s))
    .child(div().h(24))
    .child(v_flex().w_full().when(view.lit("plans"), (el) => el.bg(t.selected).rounded(7))
      .child(view.heading(t, "Plans"))
      .child(div().pb(6).text_size(12.5).text_color(t.dim).child("How much of each plan is used."))
      .child(claudeRow(t, s.status?.claude, s.known))
      .children(planEntries(s).map((entry) => planRow(view, t, cx, s, entry))));
}
