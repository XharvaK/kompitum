// Settings export and import: one JSON file with the person's settings (never a secret) and the
// workers they added, and a sheet that says what importing one changes.

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { kom, readTextFile, writeTextFile, pickFiles, pickSavePath, window, text } from "kompitum";
import { BUILTIN } from "./agents.js";

const EXCLUDED = /token|secret|password|key/i;

function errorText(error) {
  return String(error?.message ?? error ?? text("settings.it-didnt-work")).trim();
}

function firstLine(text) {
  return String(text ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";
}

function parseKomError(r, args) {
  let message = firstLine(r.stderr || r.stdout);
  try {
    message = JSON.parse(r.stdout || r.stderr).error?.message ?? message;
  } catch {}
  return message || `kom ${args.join(" ")} failed`;
}

// Every field whose name mentions a credential stays behind, at any depth; `summon_key` is a
// setting, not a secret.
export function keepField(name) {
  const lower = String(name).toLowerCase();
  return lower === "summon_key" || !EXCLUDED.test(lower);
}

export function filterPerson(value) {
  if (Array.isArray(value)) return value.map(filterPerson);
  if (!value || typeof value !== "object") return value;
  const out = {};
  for (const [name, child] of Object.entries(value)) {
    if (keepField(name)) out[name] = filterPerson(child);
  }
  return out;
}

function mergeValue(current, incoming) {
  if (incoming && typeof incoming === "object" && !Array.isArray(incoming)) {
    const base = current && typeof current === "object" && !Array.isArray(current) ? current : {};
    const out = { ...base };
    for (const [name, child] of Object.entries(incoming)) {
      if (keepField(name)) out[name] = mergeValue(base[name], child);
    }
    return out;
  }
  return incoming;
}

// The imported settings on top of the current ones; a secret the export left out keeps its current
// value, wherever it sits.
export function mergePerson(current, imported) {
  const base = current && typeof current === "object" ? current : {};
  const out = { ...base };
  for (const [name, value] of Object.entries(imported ?? {})) {
    if (keepField(name)) out[name] = mergeValue(base[name], value);
  }
  return out;
}

export function exportDocument(person, workers, now = new Date()) {
  const added = (workers?.workers ?? [])
    .filter((worker) => worker?.key && !BUILTIN.has(worker.key))
    .map((worker) => ({
      key: worker.key,
      cli: worker.cli,
      label: worker.label ?? worker.key,
      route: worker.route,
    }));
  return {
    kompitum_settings: 1,
    exported: now.toISOString(),
    person: filterPerson(person ?? {}),
    workers: {
      order: workers?.order ?? [],
      mode: workers?.mode ?? "auto",
      added,
    },
  };
}

function short(value) {
  const text = typeof value === "string" ? value : JSON.stringify(value ?? null);
  const plain = text ?? "";
  return plain.length > 40 ? `${plain.slice(0, 40)}…` : plain;
}

function labelOf(worker, key) {
  return worker?.label ?? key;
}

export function importChanges(currentPerson, currentWorkers, document) {
  const changes = [];
  for (const [name, value] of Object.entries(document?.person ?? {})) {
    if (!keepField(name)) continue;
    const before = currentPerson?.[name];
    const after = mergeValue(before, value);
    if (JSON.stringify(before) !== JSON.stringify(after)) {
      changes.push(text("settings.import-change", { field: name, before: short(before), after: short(after) }));
    }
  }
  const workers = document?.workers ?? {};
  const now = currentWorkers?.workers ?? [];
  const have = new Set(now.map((worker) => worker.key));
  for (const worker of workers.added ?? []) {
    if (!worker?.key || have.has(worker.key)) continue;
    changes.push(text("settings.import-add-worker", { worker: labelOf(worker, worker.key) }));
  }
  const currentOrder = currentWorkers?.order ?? [];
  const order = workers.order ?? [];
  for (let index = 0; index < order.length; index += 1) {
    const key = order[index];
    const was = currentOrder.indexOf(key);
    if (was < 0 || was === index) continue;
    const worker = now.find((entry) => entry.key === key);
    changes.push(text("settings.import-move-worker", { worker: labelOf(worker, key), from: was + 1, to: index + 1 }));
  }
  return changes;
}

export function workerCommands(document, currentWorkers) {
  const workers = document?.workers ?? {};
  const have = new Set((currentWorkers?.workers ?? []).map((worker) => worker.key));
  const commands = [];
  for (const worker of workers.added ?? []) {
    if (!worker?.key || have.has(worker.key)) continue;
    const args = ["workers", "--add", worker.key, "--cli", worker.cli];
    if (worker.label) args.push("--label", worker.label);
    args.push("--route", worker.route);
    commands.push(args);
  }
  if ((workers.order ?? []).length > 0) commands.push(["workers", "--order", workers.order.join(",")]);
  if (workers.mode) commands.push(["workers", "--mode", workers.mode]);
  return commands;
}

export function exportSettings(view, cx) {
  cx.spawn(async (cx) => {
    view.exportNote = null;
    try {
      const raw = await readTextFile(view.file());
      const person = raw ? JSON.parse(raw) : {};
      const r = await kom(["workers", "--json"]);
      if (r.code !== 0) throw new Error(parseKomError(r, ["workers", "--json"]));
      const workers = JSON.parse(r.stdout || "null");
      const text = `${JSON.stringify(exportDocument(person, workers), null, 2)}\n`;
      const path = await pickSavePath("kompitum-settings.json");
      if (!path) return;
      // The save dialog confirms replacing a file on Windows; elsewhere an existing file is refused.
      let expected = null;
      if (window()?.platform === "windows") {
        try {
          expected = await readTextFile(path);
        } catch {}
      }
      await writeTextFile(path, text, expected);
      view.exportNote = text("settings.exported");
    } catch (error) {
      view.exportNote = text("settings.export-failed", { error: firstLine(errorText(error)) });
    }
    cx.notify();
  });
}

export function importSettings(view, cx) {
  cx.spawn(async (cx) => {
    view.importError = null;
    view.importSheet = null;
    try {
      const paths = await pickFiles();
      const path = paths?.[0];
      if (!path) return;
      const raw = await readTextFile(path);
      let document = null;
      try {
        document = JSON.parse(raw ?? "");
      } catch {}
      if (!document || document.kompitum_settings !== 1) {
        view.importError = text("settings.not-a-settings-file");
        cx.notify();
        return;
      }
      await view.load();
      let workers = null;
      try {
        const r = await kom(["workers", "--json"]);
        if (r.code === 0) workers = JSON.parse(r.stdout || "null");
      } catch {}
      view.importSheet = {
        document,
        workers,
        changes: importChanges(view.person, workers, document),
      };
    } catch (error) {
      view.importError = text("settings.read-file-failed", { error: firstLine(errorText(error)) });
    }
    cx.notify();
  });
}

function runWorkerImport(view, document, workers, cx) {
  cx.spawn(async (cx) => {
    try {
      for (const args of workerCommands(document, workers)) {
        const r = await kom(args);
        if (r.code !== 0) throw new Error(parseKomError(r, args));
      }
      view.importNote = text("settings.imported");
    } catch (error) {
      view.importNote = text("settings.import-workers-failed", { error: firstLine(errorText(error)) });
    }
    cx.notify();
  });
}

export function applyImport(view, cx) {
  const sheet = view.importSheet;
  if (!sheet) return;
  view.importSheet = null;
  view.importNote = text("settings.importing");
  view.change((person) => {
    const merged = mergePerson(person, sheet.document.person ?? {});
    for (const name of Object.keys(person)) delete person[name];
    Object.assign(person, merged);
  }, cx, (cx) => runWorkerImport(view, sheet.document, sheet.workers, cx));
  cx.notify();
}

export function importSheet(view, t, cx) {
  const sheet = view.importSheet;
  return v_flex()
    .id("st-import-sheet")
    .w_full()
    .gap(10)
    .child(div().text_size(16).font_semibold().text_color(t.foreground).child(text("settings.import-settings-title")))
    .child(div().text_size(13).line_height("18px").text_color(t.dim).child(text("settings.import-changes")))
    .when(sheet.changes.length === 0, (el) =>
      el.child(div().text_size(14).text_color(t.muted).child(text("settings.nothing-will-change"))))
    .children(sheet.changes.map((change, index) =>
      div()
        .id(`st-import-change-${index}`)
        .w_full()
        .text_size(14)
        .line_height("20px")
        .text_color(t.foreground)
        .child(change)))
    .child(
      h_flex()
        .w_full()
        .gap(8)
        .justify_end()
        .child(view.button(t, "st-import-cancel", text("settings.cancel"), (_e, cx) => {
          view.importSheet = null;
          cx.notify();
        }))
        .child(view.primaryButton(t, "st-import-confirm", text("settings.import"), (_e, cx) => applyImport(view, cx))),
    );
}
