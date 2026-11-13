// The bell: what needs the Owner (the lead's question, blocked agents, ready updates, workers that ran out)
// and the notes waiting in the vault, in one popup from the sidebar's foot. Reading is kom status --json and
// kom vault open; each row's button runs a command and closes the popup.

import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { kom, theme, run, props, openOverlay, closeOverlay, settings, knownAgents, agents, lead, updates } from "kompitum";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n) => String(n).padStart(2, "0");
const cap = (s) => (s ? String(s).replace(/^./, (c) => c.toUpperCase()) : "");
const text = (v) => (typeof v === "string" ? v : "");
const base = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";

function clock(d) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

// Today says 09:10; another day says Nov 4 18:26.
function stamp(value) {
  const d = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(d.getTime())) return String(value ?? "");
  const now = new Date();
  const today = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  return today ? clock(d) : `${MONTHS[d.getMonth()]} ${d.getDate()} ${clock(d)}`;
}

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

// Whatever kom status gives for workers: an array, or an object holding one.
function workersOf(workers) {
  const list = Array.isArray(workers) ? workers : Array.isArray(workers?.workers) ? workers.workers : [];
  return list.filter((w) => w && typeof w === "object"
    && typeof (w.label ?? w.name) === "string"
    && typeof (w.status ?? w.state) === "string")
    .map((w) => ({ label: w.label ?? w.name, cli: w.cli ?? w.key, status: w.status ?? w.state, until: w.until ?? w.until_ms }));
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

// The catalog names a worker's CLI ("OpenCode", not "opencode"); a plan after the CLI's old label ("Go") stays.
function workerName(catalog, worker) {
  const cli = text(worker.cli);
  const entry = cli ? catalog.find((item) => item.worker === cli || item.key === cli) : null;
  const label = text(worker.label);
  if (!entry?.name) return label;
  return label.toLowerCase().startsWith(cli.toLowerCase()) ? entry.name + label.slice(cli.length) : entry.name;
}

// The lead's program name, in the composer's words ("Claude", not its catalog name).
function leadName(catalog, key) {
  return key === "claude" ? "Claude" : catalog.find((item) => item.key === key)?.name ?? key;
}

// A blocked agent's name: its own title, else its tab's, else its program's.
const agentName = (a) => a.title || a.tab_name || a.program?.name || "An agent";

// The first system that joined through Connect, or null: who the notes are with, and its app's link.
function joined() {
  const connect = settings()?.connect ?? {};
  const key = Object.keys(connect).sort().find((k) => connect[k]?.agent);
  if (!key) return null;
  const agent = String(connect[key].agent);
  return { agent, name: connect[key].name || cap(agent), link: connect[key].link || `${key}://` };
}

export default class Bell extends View {
  init(_props, cx) {
    this.workers = [];
    this.notes = [];
    this.statusError = null;
    this.vaultError = null;
    this.catalog = [];
    this.reading = false;
    this.read(cx);
    cx.timer.every(10000, (cx) => this.read(cx));
  }

  // The status, the vault and the agents' names, one after the other. A failed read keeps the last data and
  // remembers its first line.
  read(cx) {
    if (this.reading) return;
    this.reading = true;
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["status", "--json"]);
        if (r?.code === 0) {
          this.workers = workersOf(JSON.parse(r.stdout || "{}").workers);
          this.statusError = null;
        } else {
          this.statusError = errorText(r);
        }
      } catch (e) {
        this.statusError = firstLine(e?.message ?? e) || "status failed";
      }
      try {
        const r = await kom(["vault", "open", "--json", "--as", "owner"]);
        if (r?.code === 0) {
          const data = JSON.parse(r.stdout || "{}");
          this.notes = Array.isArray(data?.notes) ? data.notes.filter((n) => n && typeof n === "object") : [];
          this.vaultError = null;
        } else {
          this.vaultError = errorText(r);
        }
      } catch (e) {
        this.vaultError = firstLine(e?.message ?? e) || "The vault didn't answer.";
      }
      // The agents' names change only with an install: read them once.
      if (this.catalog.length === 0) {
        try {
          this.catalog = await knownAgents();
        } catch {}
      }
      this.reading = false;
      cx.notify();
    });
  }

  // A row: a title and, when there is one, a detail. Its button runs a command and closes the popup.
  item(t, id, title, detail, button) {
    return h_flex().id(id).min_h(40).px(8).py(4).gap(10).items_center().rounded(8).hover((s) => s.bg(t.hover))
      .child(v_flex().flex_1().min_w(0)
        .child(div().text_size(13).text_color(t.foreground).truncate().child(title))
        .when(Boolean(detail), (el) => el.child(div().text_size(11.5).text_color(t.dim).truncate().child(detail))))
      .when(Boolean(button), (el) => el.child(h_flex().id(`${id}-do`).h(24).px(10).flex_none().items_center().rounded(6).bg(t.raised)
        .text_size(12).font_medium().text_color(t.foreground).cursor_pointer().hover((s) => s.opacity(0.85))
        .on_click(() => {
          button.go();
          closeOverlay(props().overlay);
        })
        .child(button.label)));
  }

  // A quiet line that runs a command and closes the popup.
  link(t, id, label, go) {
    return div().id(id).px(8).py(4).text_size(12).text_color(t.muted).cursor_pointer()
      .on_click(() => {
        go();
        closeOverlay(props().overlay);
      })
      .child(label);
  }

  // What needs the Owner, in the bell's order. Worker tasks are not here: the Tasks row covers them.
  needs(t) {
    const rows = [];
    if (lead()?.ask) {
      const program = settings()?.lead?.program ?? "claude";
      rows.push(this.item(t, "bell-lead", `${leadName(this.catalog, program)} is asking you`, "",
        { label: "Open", go: () => run("lead.open") }));
    }
    for (const a of agents().filter((agent) => agent.status === "blocked")) {
      rows.push(this.item(t, `bell-agent-${a.address}`, `${agentName(a)} needs you`, text(a.detail),
        { label: "Show", go: () => run("pane.focus", { pane: a.pane }) }));
    }
    const u = updates();
    if (u.app.ready) {
      rows.push(this.item(t, "bell-update-app", `Kompitum ${u.app.ready} is ready`, "Restart to update. Your terminals keep running.",
        { label: "Restart", go: () => run("app.update") }));
    }
    if (u.core.ready && !u.core.updating) {
      rows.push(this.item(t, "bell-update-core", `kompitum-core ${u.core.ready} is ready`, "Your terminals keep running.",
        { label: "Update", go: () => run("app.update_core") }));
    }
    for (const w of this.workers.filter((worker) => /block|out/i.test(worker.status))) {
      const until = untilText(w.until);
      rows.push(this.item(t, `bell-limit-${w.cli ?? w.label}`, `${workerName(this.catalog, w)} is out${until ? ` until ${until}` : ""}`,
        "Tasks go to the next worker.", null));
    }
    return rows;
  }

  // A waiting note: its title, its sender and when it came. Open shows it in the mailbox.
  noteItem(t, note, index) {
    const path = text(note.path);
    const title = text(note.title) || base(path).replace(/\.md$/, "") || "Note";
    const detail = [text(note.from), stamp(note.date)].filter(Boolean).join(" · ");
    return this.item(t, `bell-note-${index}`, title, detail, { label: "Open", go: () => run("app.mailbox", { note: path }) });
  }

  render() {
    const t = theme();
    const other = joined();
    const needs = this.needs(t);
    const notes = [];
    if (this.vaultError) {
      notes.push(div().id("bell-vault-error").px(8).py(4).text_size(11.5).text_color(t.dim).child(this.vaultError));
    } else if (this.notes.length === 0) {
      notes.push(div().id("bell-no-notes").px(8).py(6).text_size(12).text_color(t.dim).child("No notes waiting."));
      notes.push(this.link(t, "bell-mailbox", "Open the mailbox", () => run("app.mailbox")));
    } else {
      notes.push(...this.notes.slice(0, 8).map((note, index) => this.noteItem(t, note, index)));
      if (this.notes.length > 8) notes.push(this.link(t, "bell-more", `All ${this.notes.length} notes`, () => run("app.mailbox")));
    }
    return v_flex().id("bell").size_full().min_h(0).bg(t.chrome).text_color(t.foreground).text_size(13).p(8).gap(2).overflow_y_scroll()
      .child(div().px(8).pt(6).pb(4).text_size(11.5).font_medium().text_color(t.dim).child("Needs you"))
      .when(Boolean(this.statusError), (el) => el.child(div().id("bell-status-error").px(8).pb(4).text_size(11.5).text_color(t.dim).child(this.statusError)))
      .children(needs.length ? needs : [div().id("bell-calm").px(8).py(6).text_size(12).text_color(t.dim).child("Nothing needs you.")])
      .child(h_flex().px(8).pt(10).pb(4).items_center()
        .child(div().text_size(11.5).font_medium().text_color(t.dim).child("Notes"))
        .child(div().flex_1())
        .when(Boolean(other), (el) => el.child(div().id("bell-thread").text_size(11.5).text_color(t.muted).cursor_pointer()
          .hover((s) => s.text_color(t.foreground))
          .on_click(() => {
            closeOverlay(props().overlay);
            openOverlay({ mod: "line", kind: "popup", width: 420, height: 460, props: { with: other.agent, name: other.name, link: other.link } });
          })
          .child(`With ${cap(other.agent)}`))))
      .children(notes);
  }
}
