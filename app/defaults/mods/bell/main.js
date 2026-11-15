// The bell: what needs the Owner (the lead's question, blocked agents, ready updates, workers that ran out),
// the toasts that left, and the notes waiting in the vault, in one popup from the bell in the top bar. Reading is
// kom status --json and kom vault open; each row's button runs a command and closes the popup.

import { View, div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { kom, theme, run, props, openOverlay, closeOverlay, settings, knownAgents, agents, lead, updates, shared, setShared, text } from "kompitum";

// The toasts that left, newest first (Kompitum keeps 20); the bell shows this many.
const RECENT = "notifications.recent";
const EARLIER_ROWS = 5;

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n) => String(n).padStart(2, "0");
const cap = (s) => (s ? String(s).replace(/^./, (c) => c.toUpperCase()) : "");
const str = (v) => (typeof v === "string" ? v : "");
const base = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";

function clock(d) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

const WEEKDAY_WORDS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

// When an out worker is back, as the Limits sheet says it: today 09:10, within a week the weekday, else 23 Nov.
function backWords(value) {
  const n = typeof value === "string" ? Date.parse(value) : Number(value);
  if (!Number.isFinite(n) || n <= 0) return "";
  const when = new Date(typeof value === "string" || n > 1e12 ? n : n * 1000);
  const midnight = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((midnight(when) - midnight(new Date())) / 86400000);
  if (days === 0) return clock(when);
  if (Math.abs(days) < 7) return WEEKDAY_WORDS[when.getDay()];
  return `${when.getDate()} ${MONTHS[when.getMonth()]}`;
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
  return firstLine(result?.stderr) || firstLine(result?.stdout) || text("bell.kom-exited", { code: result?.code ?? "?" });
}

// Whatever kom status gives for workers: an array, or an object holding one.
function workersOf(workers) {
  const list = Array.isArray(workers) ? workers : Array.isArray(workers?.workers) ? workers.workers : [];
  return list.filter((w) => w && typeof w === "object"
    && typeof (w.label ?? w.name) === "string"
    && typeof (w.status ?? w.state) === "string")
    .map((w) => ({ key: w.key, label: w.label ?? w.name, cli: w.cli ?? w.key, status: w.status ?? w.state, until: w.until ?? w.until_ms }));
}

// The catalog names a worker's CLI ("OpenCode", not "opencode"); a plan after the CLI's old label ("Go") stays.
function workerName(catalog, worker) {
  const cli = str(worker.cli);
  const entry = cli ? catalog.find((item) => item.worker === cli || item.key === cli) : null;
  const label = str(worker.label);
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
    this.nextWorker = null;
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
          const status = JSON.parse(r.stdout || "{}");
          this.workers = workersOf(status.workers);
          this.nextWorker = typeof status.workers?.next === "string" ? status.workers.next : null;
          this.statusError = null;
        } else {
          this.statusError = errorText(r);
        }
      } catch (e) {
        this.statusError = firstLine(e?.message ?? e) || text("bell.status-failed");
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
        this.vaultError = firstLine(e?.message ?? e) || text("bell.vault-no-answer");
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

  // A section's name: the label size, in the secondary text color, so it reads as a heading over its rows.
  header(t, label) {
    return div().px(8).pt(12).pb(4).text_size(12).font_medium().text_color(t.muted).child(label);
  }

  // A row: a title and, when there is one, a detail. Its button runs a command and closes the popup.
  item(t, id, title, detail, button) {
    return h_flex().id(id).min_h(40).px(8).py(4).gap(12).items_center().rounded(8).hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
      .child(v_flex().flex_1().min_w(0).gap(2)
        .child(div().text_size(13).text_color(t.foreground).truncate().child(title))
        .when(Boolean(detail), (el) => el.child(div().text_size(12).text_color(t.dim).truncate().child(detail))))
      .when(Boolean(button), (el) => el.child(h_flex().id(`${id}-do`).h(24).px(10).flex_none().items_center().rounded(6).bg(t.raised)
        .text_size(12).font_medium().text_color(t.foreground).cursor_pointer().relative().keep_focus(true).tab_stop(true).focus_ring(t.focus, 6)
        .accessibility_label(button.label)
        .on_click(() => {
          button.go();
          closeOverlay(props().overlay);
        })
        .child(div().absolute().inset_0().rounded(6).hover((s) => s.bg(t.hover)).active((s) => s.bg(t.selected)))
        .child(button.label)));
  }

  // A quiet line that runs a command and closes the popup.
  link(t, id, label, go) {
    return div().id(id).px(8).py(4).text_size(12).text_color(t.muted).cursor_pointer()
      .keep_focus(true)
      .tab_stop(true)
      .focus_ring(t.focus, 0)
      .on_click(() => {
        go();
        closeOverlay(props().overlay);
      })
      .child(label);
  }

  // What needs the Owner to act: the lead's question and blocked agents. Worker tasks are not here: the Tasks row
  // covers them.
  needs(t) {
    const rows = [];
    if (lead()?.ask) {
      const program = settings()?.lead?.program ?? "claude";
      rows.push(this.item(t, "bell-lead", text("bell.lead-asking", { name: leadName(this.catalog, program) }), "",
        { label: text("bell.open"), go: () => run("lead.open") }));
    }
    for (const a of agents().filter((agent) => agent.status === "blocked")) {
      rows.push(this.item(t, `bell-agent-${a.address}`, text("bell.agent-needs-you", { name: agentName(a) }), str(a.detail),
        { label: text("bell.show"), go: () => run("pane.focus", { pane: a.pane }) }));
    }
    return rows;
  }

  // What changed that the Owner may want to know or act on when it suits them: ready updates, and workers that
  // ran out (tasks reroute by themselves, so those rows have no button).
  news(t) {
    const rows = [];
    const u = updates();
    if (u.app.ready) {
      rows.push(this.item(t, "bell-update-app", text("bell.update-app", { version: u.app.ready }), text("bell.update-app-detail"),
        { label: text("bell.restart"), go: () => run("app.update") }));
    }
    if (u.core.ready && !u.core.updating) {
      rows.push(this.item(t, "bell-update-core", text("bell.update-core", { version: u.core.ready }), text("bell.update-core-detail"),
        { label: text("bell.update"), go: () => run("app.update_core") }));
    }
    const next = this.workers.find((worker) => worker.key && worker.key === this.nextWorker);
    const goesTo = next ? text("bell.tasks-go-to", { name: workerName(this.catalog, next) }) : text("bell.tasks-next");
    for (const w of this.workers.filter((worker) => /block|out/i.test(worker.status))) {
      const back = backWords(w.until);
      rows.push(this.item(t, `bell-limit-${w.cli ?? w.label}`, back ? text("bell.worker-out-until", { name: workerName(this.catalog, w), when: back }) : text("bell.worker-out", { name: workerName(this.catalog, w) }),
        goesTo, null));
    }
    return rows;
  }

  // The toasts that left, newest first: each with its first line and when it came, and Show when it had an action.
  earlier(t) {
    const recent = shared(RECENT);
    const list = Array.isArray(recent) ? recent.filter((n) => n && typeof n === "object" && str(n.title)) : [];
    return list.slice(0, EARLIER_ROWS).map((n, index) => {
      const detail = [firstLine(n.body), Number.isFinite(n.at) ? stamp(n.at) : ""].filter(Boolean).join(" · ");
      const action = n.action && typeof n.action.name === "string" ? n.action : null;
      return this.item(t, `bell-recent-${index}`, n.title, detail, action ? { label: text("bell.show"), go: () => run(action.name, action.args ?? {}) } : null);
    });
  }

  // A waiting note: its title, its sender and when it came. Open shows it in the mailbox.
  noteItem(t, note, index) {
    const path = str(note.path);
    const title = str(note.title) || base(path).replace(/\.md$/, "") || text("bell.note");
    const detail = [str(note.from), stamp(note.date)].filter(Boolean).join(" · ");
    return this.item(t, `bell-note-${index}`, title, detail, { label: text("bell.open"), go: () => run("app.mailbox", { note: path }) });
  }

  render() {
    const t = theme();
    const other = joined();
    const needs = this.needs(t);
    const news = this.news(t);
    const earlier = this.earlier(t);
    const notes = [];
    if (this.vaultError) {
      notes.push(div().id("bell-vault-error").px(8).py(4).text_size(12).text_color(t.dim).child(this.vaultError));
    } else if (this.notes.length === 0) {
      // The empty mailbox is one row with its own button, not a sentence and a bare link.
      notes.push(this.item(t, "bell-mailbox", text("bell.mailbox"), text("bell.no-notes"), { label: text("bell.open"), go: () => run("app.mailbox") }));
    } else {
      notes.push(...this.notes.slice(0, 8).map((note, index) => this.noteItem(t, note, index)));
      if (this.notes.length > 8) notes.push(this.link(t, "bell-more", text("bell.all-notes", { count: this.notes.length }), () => run("app.mailbox")));
    }
    return v_flex().id("bell").size_full().min_h(0).bg(t.chrome).text_color(t.foreground).text_size(13).px(8).pt(2).pb(8).gap(2).overflow_y_scroll()
      // With nothing to act on, one calm line says so in place of the heading over an empty list.
      .when(needs.length > 0, (el) => el.child(this.header(t, text("bell.needs-you"))).children(needs))
      .when(needs.length === 0, (el) => el.child(div().id("bell-calm").px(8).pt(12).pb(2).text_size(13).text_color(t.muted).child(text("bell.nothing-needs-you"))))
      .when(Boolean(this.statusError), (el) => el.child(div().id("bell-status-error").px(8).pb(4).text_size(12).text_color(t.dim).child(this.statusError)))
      .when(news.length > 0, (el) => el.child(this.header(t, text("bell.updates"))).children(news))
      .when(earlier.length > 0, (el) => el.child(h_flex().px(8).pt(12).pb(4).items_center()
        .child(div().text_size(12).font_medium().text_color(t.muted).child(text("bell.earlier")))
        .child(div().flex_1())
        .child(div().id("bell-recent-clear").text_size(12).text_color(t.muted).cursor_pointer()
          .keep_focus(true)
          .tab_stop(true)
          .focus_ring(t.focus, 0)
          .hover((s) => s.text_color(t.foreground))
          .on_click(() => setShared(RECENT, []))
          .child(text("bell.clear"))))
        .children(earlier))
      .child(h_flex().px(8).pt(12).pb(4).items_center()
        .child(div().text_size(12).font_medium().text_color(t.muted).child(text("bell.notes")))
        .child(div().flex_1())
        .when(Boolean(other), (el) => el.child(div().id("bell-thread").text_size(12).text_color(t.muted).cursor_pointer()
          .keep_focus(true)
          .tab_stop(true)
          .focus_ring(t.focus, 0)
          .hover((s) => s.text_color(t.foreground))
          .on_click(() => {
            closeOverlay(props().overlay);
            openOverlay({ mod: "line", kind: "popup", width: 420, height: 460, props: { with: other.agent, name: other.name, link: other.link } });
          })
          .child(text("bell.with", { name: cap(other.agent) })))))
      .children(notes);
  }
}
