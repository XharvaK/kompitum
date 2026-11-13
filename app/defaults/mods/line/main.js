// The notes between the lead and an agent that joined through Connect.
// Reads only kom vault thread: kom vault read would write a read mark in the lead's name.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, v_virtual_list, VirtualListScrollHandle, Scrollbar } from "gpui-base";
import { kom, theme, notify, playSound, openOverlay, closeOverlay, props, run, appLink, openLink } from "kompitum";

// The same two colors the Board's chips use: the lead's, and the other agent's.
const OWN = "accent";
const OTHER = "ansi5";
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n) => String(n).padStart(2, "0");

const cap = (s) => (s ? String(s).replace(/^./, (c) => c.toUpperCase()) : "");
const text = (v) => (typeof v === "string" ? v : "");
const base = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

function clock(d) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

// Text runs in a div do not break on newlines; one child per line does.
function lines(value) {
  return String(value ?? "").replace(/\r\n/g, "\n").split("\n").map((line) => div().w_full().child(line || " "));
}

// Today says 09:10; another day says Nov 4 18:26.
function stamp(value) {
  const d = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(d.getTime())) return String(value ?? "");
  const now = new Date();
  const today = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  return today ? clock(d) : `${MONTHS[d.getMonth()]} ${d.getDate()} ${clock(d)}`;
}

// Piped, kom says what went wrong as JSON: {"error": {"message", "hint"}}; show its words.
function errorText(result) {
  for (const out of [result?.stderr, result?.stdout]) {
    try {
      const message = JSON.parse(String(out ?? "")).error?.message;
      if (message) return String(message);
    } catch {}
  }
  return String(result?.stderr || result?.stdout || "").trim() || `kom exited ${result?.code ?? "?"}`;
}

function problemText(problem) {
  if (typeof problem === "string") return problem;
  if (problem && typeof problem === "object") {
    return String(problem.message ?? problem.why ?? problem.error ?? JSON.stringify(problem));
  }
  return String(problem ?? "");
}

export function stripMarkdown(value) {
  return String(value ?? "")
    .replace(/\*\*/g, "")
    .replace(/__/g, "")
    .replace(/[*_`]/g, "")
    .replace(/(^|\n)\s*#{1,6}\s*/g, "$1");
}

function bodyLines(body) {
  const value = String(body ?? "").replace(/\r\n/g, "\n");
  return value ? value.split("\n").slice(0, 4).map(stripMarkdown) : [];
}

// The same test as frame/settings/vault.js
function isSyncProblem(words) {
  return /couldn't be updated|could not be pulled|may be behind|still working in the vault|nothing was pulled/i.test(String(words ?? ""));
}

// The core says a read's pull failed with these words; they belong behind
// "Details", not over a card.
function behindProblems(problems) {
  return problems.filter((problem) => isSyncProblem(problemText(problem)));
}

// The row heights the cards below are built from: padding, border, one line
// each for who and the title, then the parts that may be absent.
function noteHeight(note, agent) {
  const lines = bodyLines(note.body);
  const project = Boolean(text(note.project));
  const meta = metaOf(note, agent).length > 0;
  let children = 2;
  let height = 20 + 2 + 16 + 19;
  if (project) { children++; height += 15; }
  if (lines.length) { children++; height += lines.length * 16; }
  if (meta) { children++; height += 15; }
  return height + 4 * (children - 1) + 6;
}

function otherSide(note, agent) {
  return note.from === agent ? note.to : note.from;
}

function metaOf(note, agent) {
  const out = [];
  const other = otherSide(note, agent);
  const reads = Array.isArray(note.read_by) ? note.read_by : [];
  const read = [...reads].reverse().find((r) => r && r.agent === other);
  if (read) out.push(`read ${stamp(read.at)}`);
  if (note.done && typeof note.done === "object") {
    out.push(`done ${stamp(note.done.at)}${note.done.why ? `: ${note.done.why}` : ""}`);
  } else if (typeof note.done === "string" && note.done) {
    out.push(`done ${note.done}`);
  }
  if (note.status && note.status !== "open") out.push(String(note.status));
  return out;
}

function unread(note, agent) {
  if (note.status !== "open") return false;
  const other = otherSide(note, agent);
  const reads = Array.isArray(note.read_by) ? note.read_by : [];
  return !reads.some((r) => r && r.agent === other);
}

export default class Line extends View {
  init(_props, cx) {
    const p = props() ?? {};
    this.mode = p.note ? "note" : "panel";
    this.other = typeof p.with === "string" && p.with ? p.with : "";
    this.notes = [];
    this.vault = "";
    this.agent = "opus";
    this.unpushed = 0;
    this.problems = [];
    this.seen = new Set();
    this.apps = {};
    this.loaded = false;
    this.loading = false;
    this.error = null;
    this.details = false;
    this.startedAt = 0;
    this.newest = "";
    this.scroll = this.mode === "panel" ? VirtualListScrollHandle.new() : null;
    if (this.mode !== "panel") return;
    this.refresh(cx);
    cx.timer.every(60000, (cx) => this.refresh(cx));
    // A read that never settles (a layout switch can drop its first one) must
    // not leave the panel loading for ever: ask again.
    cx.timer.every(5000, (cx) => {
      if (this.loaded || this.error || !this.loading) return;
      if (this.startedAt > 0 && Date.now() - this.startedAt > 10000) {
        this.loading = false;
        this.refresh(cx);
      }
    });
  }

  refresh(cx) {
    // No agent has joined: there is no thread to read.
    if (this.loading || !this.other) return;
    this.loading = true;
    this.startedAt = Date.now();
    const seq = (this.seq = (this.seq ?? 0) + 1);
    cx.spawn(async (cx) => {
      try {
        const result = await kom(["vault", "thread", this.other, "--json"]);
        // A newer read started meanwhile (Retry, the watchdog): it decides.
        if (seq !== this.seq) return;
        if (result.code !== 0) {
          this.error = errorText(result);
        } else {
          let data;
          try {
            data = JSON.parse(result.stdout);
          } catch (e) {
            throw new Error(firstLine(e?.message ?? e) || "thread did not return JSON");
          }
          if (!data || typeof data !== "object") throw new Error("thread did not return an object");
          const notes = Array.isArray(data.notes) ? data.notes.filter((n) => n && typeof n === "object") : [];
          const agent = text(data.agent) || this.agent;
          const paths = new Set(notes.map((n) => n.path).filter((p) => typeof p === "string"));
          if (this.loaded) {
            const fresh = notes.filter((n) => typeof n.path === "string" && !this.seen.has(n.path));
            for (const n of fresh) {
              notify({ title: `${cap(n.from) || "Note"}: ${text(n.title) || base(n.path) || "new note"}` });
            }
            if (fresh.length) playSound("done");
          }
          const newest = notes.length ? text(notes[notes.length - 1].path) : "";
          const first = !this.loaded;
          this.notes = notes;
          this.vault = text(data.vault);
          this.agent = agent;
          this.unpushed = Number.isFinite(Number(data.unpushed)) ? Number(data.unpushed) : 0;
          this.problems = Array.isArray(data.problems) ? data.problems : [];
          this.seen = paths;
          this.loaded = true;
          this.error = null;
          if (first || newest !== this.newest) {
            this.newest = newest;
            // scroll_to_bottom() reads a count it only learns on the next layout;
            // the last index is what we know now.
            if (notes.length > 0) this.scroll.scroll_to_item(notes.length - 1);
          }
        }
      } catch (e) {
        if (seq === this.seq) this.error = String(e?.message ?? e ?? "").trim() || "thread failed";
      } finally {
        if (seq === this.seq) {
          this.loading = false;
          this.startedAt = 0;
        }
        cx.notify();
      }
    });
  }

  titlebar(t, label) {
    return h_flex()
      .h(30)
      .w_full()
      .flex_none()
      .px(10)
      .gap(8)
      .items_center()
      .bg(t.surface)
      .border_b(1)
      .border_color(t.border)
      .text_size(12)
      .child(div().flex_1().min_w(0).truncate().font_semibold().text_color(t.foreground).child(label || ""))
      .child(div()
        .id("line-close")
        .px(6)
        .py(2)
        .rounded(5)
        .text_color(t.muted)
        .cursor_pointer()
        .tooltip("Close")
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .on_click(() => closeOverlay(props().overlay))
        .child(svg("mods/workspace-chrome/x.svg").size(12)));
  }

  chip(t, label, color) {
    return div()
      .px(5)
      .h(16)
      .flex_none()
      .flex()
      .items_center()
      .rounded(4)
      .bg(color + "1f")
      .text_size(10)
      .font_medium()
      .text_color(color)
      .child(label);
  }

  senderColor(t, from) {
    if (from === this.agent) return t[OWN];
    if (this.other && from === this.other) return t[OTHER];
    return t.muted;
  }

  // Drawn by the virtual list, so no handlers live here: on_item_click on the
  // list opens the note. Heights are what noteHeight() above promises.
  card(t, note) {
    const path = text(note.path);
    const from = text(note.from);
    const title = text(note.title) || base(path) || "Note";
    const kind = text(note.kind);
    const time = stamp(note.date);
    const project = text(note.project);
    const meta = metaOf(note, this.agent);
    const lines = bodyLines(note.body);
    return v_flex()
      .p(10)
      .mb(6)
      .gap(4)
      .rounded(8)
      .bg(t.raised)
      .border_1()
      .border_color(unread(note, this.agent) ? t.accent : t.border)
      .child(h_flex().h(16).gap(6).items_center()
        .child(div().h(16).flex_none().truncate().text_size(11).font_semibold().text_color(this.senderColor(t, from)).child(cap(from) || "Note"))
        .when(Boolean(kind), (el) => el.child(this.chip(t, kind, t.muted)))
        .child(div().flex_1())
        .when(Boolean(time), (el) => el.child(div().h(16).flex_none().truncate().text_size(10.5).text_color(t.dim).child(time))))
      .child(div().h(19).w_full().truncate().text_size(12.5).font_medium().text_color(t.foreground).child(title))
      .when(Boolean(project), (el) => el.child(div().h(15).w_full().truncate().text_size(10.5).text_color(t.dim).child(project)))
      .when(lines.length > 0, (el) => el.child(v_flex().w_full().children(lines.map((line) =>
        div().h(16).w_full().truncate().text_size(11.5).text_color(t.muted).child(line)))))
      .when(meta.length > 0, (el) => el.child(div().h(15).w_full().truncate().text_size(10.5).text_color(t.dim).child(meta.join(" · "))));
  }

  openNote(note) {
    openOverlay({
      mod: "line",
      kind: "window",
      key: `line-note:${text(note.path)}`,
      width: 760,
      height: 560,
      props: { note, vault: this.vault, agent: this.agent, with: this.other },
    });
  }

  retry(cx) {
    this.loading = false;
    this.refresh(cx);
  }

  // Whether an app answers the link's scheme, asked once per link.
  appFor(link) {
    if (!(link in this.apps)) {
      let app = null;
      try {
        app = appLink(link.split(":")[0]);
      } catch {
        app = null;
      }
      this.apps[link] = Boolean(app);
    }
    return this.apps[link];
  }

  // Opens the joined system's link, asking again now; without an app, says so.
  openDoor(link, name) {
    const scheme = link.split(":")[0];
    let app = null;
    try {
      app = appLink(scheme);
    } catch {}
    if (app) return openLink(link);
    notify({ title: `Could not open ${name}`, body: `Nothing on this computer opens ${scheme}:// links.`, level: "error" });
  }

  renderPanel() {
    const t = theme();
    const link = typeof props()?.link === "string" ? props().link : "";
    const name = props()?.name ? String(props().name) : cap(this.other);
    const behind = behindProblems(this.problems);
    const details = behind;
    const filed = this.problems.filter((problem) => !isSyncProblem(problemText(problem)));
    return v_flex()
      .id("line")
      .size_full()
      .min_h(0)
      .bg(t.chrome)
      .text_color(t.foreground)
      .text_size(13)
      .child(h_flex().h(42).flex_none().px(12).gap(8).items_center().border_b(1).border_color(t.border)
        .child(div().font_semibold().child("Notes"))
        .child(this.other
          ? div().flex_1().min_w(0).truncate().text_size(11).text_color(t.dim).child(`with ${cap(this.other)}`)
          : div().flex_1())
        .when(Boolean(link) && this.appFor(link), (el) => el.child(h_flex().id("line-door").px(6).py(2).gap(4).items_center().rounded(5)
          .text_size(11).text_color(t.muted).cursor_pointer().hover((s) => s.text_color(t.foreground))
          .tooltip(`Open ${name}`)
          .on_click(() => this.openDoor(link, name))
          .child(name)
          .child(svg("mods/lead/arrow-up-right.svg").size(11))))
        .child(div()
          .id("line-refresh")
          .px(6)
          .py(2)
          .rounded(5)
          .text_color(t.muted)
          .cursor_pointer()
          .tooltip("Refresh")
          .hover((s) => s.bg(t.hover).text_color(t.foreground))
          .on_click((_e, cx) => this.refresh(cx))
          .child(svg("frame/settings/refresh.svg").size(13))))
      .when(this.unpushed > 0, (el) => el.child(div().px(12).pt(6).text_size(11).text_color(t.muted)
        .child(`${this.unpushed} unpushed`)))
      .when(details.length > 0, (el) => el.child(h_flex().id("line-behind").px(12).pt(6).gap(8).items_center()
        .child(div().text_size(11).text_color(t.muted).child("Couldn't sync the notes."))
        .child(div().id("line-retry").text_size(11).text_color(t.accent).cursor_pointer()
          .hover((s) => s.text_color(t.foreground))
          .on_click((_e, cx) => this.retry(cx))
          .child("Retry"))
        .child(div().id("line-details").text_size(11).text_color(t.dim).cursor_pointer()
          .hover((s) => s.text_color(t.foreground))
          .on_click((_e, cx) => { this.details = !this.details; cx.notify(); })
          .child("Details"))))
      .when(this.details && details.length > 0, (el) => el.child(v_flex().px(12).pt(4).gap(2)
        .children(details.map((problem) => {
          const message = problemText(problem);
          return div().text_size(11).text_color(t.dim).tooltip(message).child(firstLine(message) || message);
        }))))
      .when(Boolean(this.error), (el) => el.child(h_flex().id("line-error").px(12).pt(6).gap(8).items_center()
        .child(div().flex_1().min_w(0).text_size(11).text_color(t.muted).tooltip(this.error).child(firstLine(this.error) || this.error))
        .child(div().id("line-error-retry").text_size(11).text_color(t.accent).cursor_pointer()
          .hover((s) => s.text_color(t.foreground))
          .on_click((_e, cx) => this.retry(cx))
          .child("Retry"))))
      .when(filed.length > 0, (el) => el.child(div().id("line-filed").px(12).pt(6).text_size(11).text_color(t.dim).cursor_pointer()
        .hover((s) => s.text_color(t.foreground))
        .on_click(() => { closeOverlay(props()?.overlay); run("app.mailbox"); })
        .child(`${filed.length} ${filed.length === 1 ? "note" : "notes"} couldn't be filed · Open the mailbox`)))
      .child(v_flex().flex_1().min_h(0).relative()
        .when(this.notes.length === 0, (el) => el.p(12).child(div().text_size(12).text_color(t.muted)
          .child(!this.other ? "No agent has joined yet." : this.error ? "No notes to show." : this.loaded ? "No notes yet." : "Loading notes…")))
        .when(this.notes.length > 0, (el) => el
          .child(v_virtual_list(
            "line-notes",
            this.notes.length,
            this.notes.map((note) => noteHeight(note, this.agent)),
            (i) => text(this.notes[i].path) || String(i),
            (range) => this.notes.slice(range.start, range.end).map((note) => this.card(t, note)),
          )
            .size_full()
            .p(10)
            .track_scroll(this.scroll)
            .on_item_click((key) => {
              const note = this.notes.find((n) => text(n.path) === key);
              if (note) this.openNote(note);
            }))
          .child(Scrollbar.vertical("line-notes").absolute().inset_0())));
  }

  renderNote() {
    const t = theme();
    const p = props() ?? {};
    const note = p.note ?? {};
    const agent = text(p.agent) || "opus";
    const from = text(note.from);
    const kind = text(note.kind);
    const title = text(note.title) || base(note.path) || "Note";
    const time = stamp(note.date);
    const project = text(note.project);
    const meta = metaOf(note, agent);
    const body = String(note.body ?? "");
    const file = [text(p.vault).replace(/[\\/]+$/, ""), text(note.path)].filter(Boolean).join("/");
    return v_flex()
      .id("line-note")
      .size_full()
      .min_h(0)
      .bg(t.popover)
      .text_color(t.foreground)
      .child(this.titlebar(t, title))
      .child(v_flex()
        .id("line-note-body")
        .flex_1()
        .min_h(0)
        .overflow_y_scrollbar()
        .p(14)
        .gap(8)
        .child(h_flex().gap(6).items_center()
          .child(div().text_size(11.5).font_semibold().text_color(this.senderColor(t, from)).child(cap(from) || "Note"))
          .when(Boolean(kind), (el) => el.child(this.chip(t, kind, t.muted)))
          .when(Boolean(time), (el) => el.child(div().text_size(11).text_color(t.dim).child(time)))
          .when(Boolean(project), (el) => el.child(div().text_size(11).text_color(t.dim).child(project))))
        .when(meta.length > 0, (el) => el.child(div().text_size(11).text_color(t.dim).child(meta.join(" · "))))
        .child(v_flex().w_full().text_size(12.5).text_color(t.foreground).children(lines(body || "No body.")))
        .when(note.cut === true, (el) => el.child(div().text_size(11).text_color(t.muted)
          .child(`cut at 4000 characters; the file is ${file || text(note.path)}`))));
  }

  render() {
    if (this.mode === "note") return this.renderNote();
    return this.renderPanel();
  }
}
