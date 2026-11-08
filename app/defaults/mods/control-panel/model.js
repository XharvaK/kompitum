export const CELL = 8, FONT = 40 / 3, ROW = 28, HISTORY_LIMIT = 128;
export const READ_INTERVAL = 5000, EXIT_INTERVAL = 15000;
export const STATUS = { working: "working", blocked: "needs you", done: "done", idle: "idle", dead: "exited", unavailable: "unavailable" };
const reported = new Set(["working", "blocked", "idle", "done"]);
export const statusOf = value => value === "error" ? "blocked" : value;

export function stripEscapes(value) {
  return String(value ?? "")
    .replace(/(?:\x1b\]|\x9d)[\s\S]*?(?:\x07|\x1b\\|\x9c|$)/g, "")
    .replace(/(?:\x1b[P^_X]|[\x90\x98\x9e\x9f])[\s\S]*?(?:\x1b\\|\x9c|$)/g, "")
    .replace(/(?:\x1b\[|\x9b)[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\x1b[ -/]*[@-~]/g, "")
    .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/g, "");
}
export const plain = value => stripEscapes(value).replace(/[\r\n\t]+/g, " ").trim();
export function lastLine(value) {
  const lines = stripEscapes(value).replace(/\r\n/g, "\n").split(/[\r\n]/);
  return (lines.reverse().find(line => line.trim()) ?? "").replace(/\t/g, "    ").trim();
}
function widthOf(ch) {
  if (/\p{Mark}/u.test(ch) || ch === "\u200d" || ch === "\ufe0f") return 0;
  const n = ch.codePointAt(0);
  return n >= 0x1100 && (n <= 0x115f || n === 0x2329 || n === 0x232a ||
    (n >= 0x2e80 && n <= 0xa4cf) || (n >= 0xac00 && n <= 0xd7a3) ||
    (n >= 0xf900 && n <= 0xfaff) || (n >= 0xfe10 && n <= 0xfe6f) ||
    (n >= 0xff01 && n <= 0xff60) || (n >= 0xffe0 && n <= 0xffe6) || n >= 0x1f000) ? 2 : 1;
}
export const cellWidth = value => [...String(value)].reduce((n, ch) => n + widthOf(ch), 0);
export function truncate(value, cells) {
  const text = plain(value);
  if (cells <= 0) return "";
  if (cellWidth(text) <= cells) return text;
  let out = "", used = 0;
  for (const ch of text) {
    const width = widthOf(ch);
    if (used + width > cells - 1) break;
    out += ch; used += width;
  }
  return out.replace(/[\u200d\ufe0f]+$/g, "") + "…";
}
export function elapsed(statusMs, now) {
  if (!Number.isFinite(statusMs) || statusMs <= 0 || statusMs > now) return "--:--";
  const seconds = Math.floor((now - statusMs) / 1000);
  if (seconds < 3600) return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  const hours = Math.floor(seconds / 3600);
  if (hours < 24) return `${hours}h ${String(Math.floor(seconds / 60) % 60).padStart(2, "0")}m`;
  const days = Math.floor(hours / 24);
  return days < 1000 ? `${days}d ${hours % 24}h` : "999d+";
}
const machineOf = address => address.includes("/") ? address.slice(0, address.lastIndexOf("/")) : null;
const compare = (a, b) => String(a).localeCompare(String(b), "en", { numeric: true });
export function collectRows(agents = [], terminals = [], spaces = []) {
  const raw = new Map(terminals.map(t => [t.address, t]));
  const result = new Map();
  for (const a of agents) {
    if (!a.address) continue;
    const t = raw.get(a.address);
    const status = statusOf(a.status);
    if (!reported.has(status)) continue;
    result.set(a.address, { ...a, title: plain(a.title) || a.address, status, child: false,
      status_ms: t?.status_ms ?? null, rawDone: false, report: t?.report ?? {} });
  }
  const location = terminal => {
    const visited = new Set();
    for (let t = terminal; t && !visited.has(t.address);) {
      visited.add(t.address);
      if (result.has(t.address)) return result.get(t.address);
      const machine = machineOf(t.address);
      const labels = t.labels ?? {};
      const qualify = id => machine && id && !id.startsWith(`${machine}/`) ? `${machine}/${id}` : id;
      const space = spaces.find(s => (s.machine ?? null) === machine && s.id === qualify(labels["limen.space"]));
      const tab = space?.tabs?.find(tab => tab.id === qualify(labels["limen.tab"]));
      if (space && tab) return { space: space.id, space_name: space.name, tab: tab.id, tab_name: tab.name, tab_index: tab.index };
      t = raw.get(t.parent) ?? raw.get(qualify(t.parent));
    }
    return { space: "", space_name: "unassigned", tab: "", tab_name: "child", tab_index: 0 };
  };
  for (const t of terminals) {
    if (!t.address || !t.parent || result.has(t.address) || (t.state && t.state !== "live")) continue;
    const report = t.report ?? {}, status = statusOf(t.status ?? report.state);
    if (!reported.has(status)) continue;
    const loc = location(t);
    result.set(t.address, { address: t.address, title: plain(t.title || t.name || report.title || t.program?.name) || t.address,
      space: loc.space, space_name: loc.space_name, tab: loc.tab, tab_name: loc.tab_name, tab_index: loc.tab_index,
      status: status === "done" ? "idle" : status, rawDone: status === "done", child: true, pane: null,
      focused: false, status_ms: t.status_ms ?? null, report, program: t.program });
  }
  return sortRows([...result.values()], spaces);
}
export function sortRows(rows, spaces = []) {
  const order = new Map(spaces.map((s, i) => [s.id, i]));
  return [...rows].sort((a, b) => (order.get(a.space) ?? 1e6) - (order.get(b.space) ?? 1e6) ||
    compare(a.space, b.space) || (a.tab_index ?? 0) - (b.tab_index ?? 0) ||
    compare(a.tab, b.tab) || compare(a.address, b.address));
}
export function partitionRows(rows) {
  return { blocked: rows.filter(a => a.status === "blocked").sort((a, b) =>
    (a.state_started_ms ?? a.status_ms ?? Infinity) - (b.state_started_ms ?? b.status_ms ?? Infinity) || compare(a.address, b.address)),
  rest: rows.filter(a => a.status !== "blocked") };
}
export function stateStart(row, previous, spaces, now) {
  const valid = value => Number.isFinite(value) && value > 0 && value <= now;
  const changed = previous?.status != null && (previous.status !== row.status || Boolean(previous.rawDone) !== Boolean(row.rawDone));
  let start = changed ? now : valid(previous?.state_started_ms) ? previous.state_started_ms : null;
  if (valid(row.status_ms) && (start === null || row.status_ms > start)) start = row.status_ms;
  if (start === null) {
    const tab = spaces.find(s => s.id === row.space)?.tabs?.find(t => t.id === row.tab);
    const candidates = row.status === "working" ? [tab?.working_since, tab?.changed_at] : [tab?.changed_at];
    start = candidates.find(valid) ?? now;
  }
  return start;
}
export function remember(previous, live, now, spaces = []) {
  const memory = new Map(previous.filter(a => a?.address).map(a => [a.address, a]));
  for (const a of live) {
    const state_started_ms = stateStart(a, memory.get(a.address), spaces, now);
    memory.delete(a.address);
    memory.set(a.address, { address: a.address, title: a.title, space: a.space, space_name: a.space_name,
      tab: a.tab, tab_name: a.tab_name, tab_index: a.tab_index, child: a.child,
      status: a.status, rawDone: Boolean(a.rawDone), state_started_ms, lastSeen: now, exit: null });
  }
  return [...memory.values()].slice(-HISTORY_LIMIT);
}
export function missingRows(history, live, terminals) {
  const present = new Set([...live, ...terminals].map(a => a.address));
  return history.filter(a => !present.has(a.address)).map(a => ({ ...a,
    status: a.exit?.state === "finished" ? "dead" : "unavailable", status_ms: null }));
}
export function confirmExits(history, payload, liveAddresses) {
  if (!payload || !Array.isArray(payload.terminals)) throw new Error("Unexpected exit reply");
  const all = new Map(payload.terminals.map(t => [t.address, t]));
  return history.map(a => {
    if (liveAddresses.has(a.address)) return a;
    const t = all.get(a.address);
    return t?.state === "finished" ? { ...a, exit: { state: "finished", code: t.exit_code ?? null, reason: plain(t.reason) || "exited" } } : a;
  });
}
export function gridSpec(width) {
  const fixed = width < 1200 ? [14, 12, 26, 20, 8] : [14, 16, 36, 28, 8];
  const padding = 24, gap = 16;
  const last = Math.max(8, Math.floor((width - padding * 2 - gap * 5) / CELL) - fixed.reduce((a, b) => a + b, 0));
  const columns = [...fixed, last];
  return { columns, padding, gap, width: columns.reduce((a, b) => a + b, 0) * CELL + padding * 2 + gap * 5 };
}
export function viewport(rows, start, count) {
  const capacity = Math.max(0, Math.floor(count));
  const offset = Math.max(0, Math.min(Math.floor(start), Math.max(0, rows.length - capacity)));
  return { rows: rows.slice(offset, offset + capacity), offset, capacity, total: rows.length };
}
export function panelSlices(rows, height, starts = { blocked: 0, rest: 0 }) {
  const parts = partitionRows(rows);
  // 28 labels, 32 attention label, 32 roster label, 28 footer.
  const available = Math.max(0, Math.floor((height - 120) / ROW));
  const attention = Math.min(Math.max(1, parts.blocked.length), 4, available);
  return { blocked: viewport(parts.blocked, starts.blocked, attention),
    rest: viewport(parts.rest, starts.rest, Math.max(0, available - attention)) };
}
export function readableRows(slices) {
  return [...slices.blocked.rows, ...slices.rest.rows].filter(a => reported.has(a.status));
}
export function nodeVisible(node, id, visible = true) {
  if (!node) return false;
  visible = visible && node.hidden !== true;
  if (node.id === id) return visible;
  const children = node.type === "stack" ? (node.children ?? []).filter(child => child.id === node.active) : node.children ?? [];
  return children.some(child => nodeVisible(child, id, visible));
}
