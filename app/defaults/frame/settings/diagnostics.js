// Copy diagnostics: one plain-text block with the versions, the lead, the workers and the look,
// with the home folder and anything token-shaped taken out.

import { shared, settings, updates, home, lead as leadOf, exec, kom, copyText, system, window, text } from "kompitum";

const TOKEN = /[A-Za-z0-9_-]{32,}/g;

function firstLine(text) {
  return String(text ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";
}

// The home folder reads as `~`, and a run that looks like a token (32 or more letters, digits, `-`
// or `_`) as `<hidden>`: this block is meant to be pasted into a report. Windows writes the same path
// with either slash and ignores case, so the folder matches both ways; it must end where a name ends,
// so a longer folder that only starts the same isn't cut.
export function sanitize(text) {
  const folder = String(home() ?? "").replace(/[\\/]+$/, "");
  let out = String(text ?? "");
  if (folder) {
    const pattern = folder
      .split(/[\\/]/)
      .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
      .join("[\\\\/]");
    out = out.replace(new RegExp(`${pattern}(?![A-Za-z0-9_.-])`, "gi"), "~");
  }
  return out.replace(TOKEN, "<hidden>");
}

function stateWord(lead) {
  if (!lead?.state) return "not running";
  return lead.permission_mode ? `${lead.state} · ${lead.permission_mode}` : lead.state;
}

export function diagnosticsText(info) {
  const lines = [
    `kompitum-app: ${info.app ?? ""}`,
    `kompitum-core: ${info.core ?? ""}`,
    `os: ${info.os ?? ""}`,
    `claude: ${info.claude ?? ""}`,
    `lead: ${stateWord(info.lead)}`,
    "workers:",
  ];
  for (const worker of info.workers ?? []) {
    const until = worker.state === "out" && worker.until ? ` until ${worker.until}` : "";
    lines.push(`${worker.label}: ${worker.state}${until}`);
  }
  lines.push(
    `theme: ${info.theme ?? ""}`,
    `reduce_motion: ${Boolean(info.reduce_motion)}`,
    `quiet_while_fullscreen: ${Boolean(info.quiet_while_fullscreen)}`,
  );
  if (info.errors?.length) lines.push("Recent errors:", ...info.errors.slice(-20));
  return sanitize(lines.join("\n"));
}

async function claudeVersion() {
  const seen = shared("claude.version");
  if (typeof seen === "string" && seen.trim()) return seen.trim();
  try {
    const r = window()?.platform === "windows"
      ? await exec("cmd.exe", ["/d", "/c", "claude", "--version"])
      : await exec("claude", ["--version"]);
    if (r.code === 0) return firstLine(r.stdout) || firstLine(r.stderr) || "unknown";
  } catch {}
  return "unknown";
}

function workerLines(workers) {
  return (workers?.workers ?? []).map((worker) => ({
    label: worker.label ?? worker.key,
    state: worker.state ?? "unknown",
    until: worker.until_ms ? new Date(Number(worker.until_ms)).toISOString().slice(0, 10) : null,
  }));
}

export function copyDiagnostics(view, cx) {
  cx.spawn(async (cx) => {
    view.diagnosticError = null;
    try {
      const u = updates();
      const s = settings();
      const sys = system();
      let workers = null;
      try {
        const r = await kom(["workers", "--json"]);
        if (r.code === 0) workers = JSON.parse(r.stdout || "null");
      } catch {}
      const lead = leadOf();
      const text = diagnosticsText({
        app: u.app?.version,
        core: u.core?.version,
        os: [sys?.name, sys?.version].filter(Boolean).join(" "),
        claude: await claudeVersion(),
        lead: lead ? { state: lead.state, permission_mode: lead.permission_mode } : null,
        workers: workerLines(workers),
        theme: s.theme,
        reduce_motion: s.reduce_motion,
        quiet_while_fullscreen: s.quiet_while_fullscreen,
      });
      copyText(text);
      view.copiedAt = Date.now();
    } catch (error) {
      view.diagnosticError = String(error?.message ?? error ?? text("settings.it-didnt-work"));
    }
    cx.notify();
  });
}
