// The mailbox: the vault's notes in a sheet over the window. Needs you on the
// left, every note beside it, and the selected note on the right. A note to
// the Owner can be answered or closed; the Owner's read mark follows opening.
// The app's own mod: rewritten from the binary on every start.

import { View, div } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState } from "gpui-base";
import { kom, readTextFile, home, props, closeOverlay, theme } from "kompitum";

const OUTSIDE = "The vault is outside your home folder, so the mailbox can't read its notes.";

const text = (v) => (typeof v === "string" ? v : "");
const base = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";

function problemText(problem) {
  if (typeof problem === "string") return problem;
  if (problem && typeof problem === "object") {
    return String(problem.message ?? problem.why ?? problem.error ?? JSON.stringify(problem));
  }
  return String(problem ?? "");
}

// Piped, kom says what went wrong as JSON: {"error": {"message", "hint"}}; show its words.
function errorText(result) {
  for (const out of [result?.stderr, result?.stdout]) {
    try {
      const message = JSON.parse(String(out ?? "")).error?.message;
      if (message) return firstLine(message);
    } catch {}
  }
  return firstLine(result?.stderr) || firstLine(result?.stdout) || `kom exited ${result?.code ?? "?"}`;
}

function parse(result) {
  if (result?.code !== 0) throw new Error(errorText(result));
  const data = JSON.parse(result?.stdout || "{}");
  if (!data || typeof data !== "object") throw new Error("The vault did not answer an object.");
  return data;
}

function notesOf(data) {
  return Array.isArray(data?.notes) ? data.notes.filter((n) => n && typeof n === "object") : [];
}

function problemsOf(data) {
  return Array.isArray(data?.problems) ? data.problems.map(problemText).filter(Boolean) : [];
}

// Mods read only under the home folder; say so instead of showing a read error.
function underHome(path, root) {
  const clean = (v) => String(v ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  const p = clean(path);
  const r = clean(root);
  return Boolean(p && r) && (p === r || p.startsWith(`${r}/`));
}

function ago(value) {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return text(value);
  const seconds = Math.max(0, Math.floor((Date.now() - d.getTime()) / 1000));
  if (seconds < 60) return "now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  if (seconds < 7 * 86400) return `${Math.floor(seconds / 86400)} d ago`;
  return day(d);
}

function day(d) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function stamp(value) {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return text(value);
  return `${day(d)} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// Text runs in a div do not break on newlines; one child per line does.
function lines(value) {
  return String(value ?? "").replace(/\r\n/g, "\n").split("\n").map((line) => div().w_full().child(line || " "));
}

// The vault addresses a note to `all` to the Owner too.
const forOwner = (note) => ["owner", "all"].includes(text(note?.to));

function readByOwner(note) {
  const reads = Array.isArray(note?.read_by) ? note.read_by : [];
  return reads.some((r) => r && r.agent === "owner");
}

export default class Mailbox extends View {
  init(_props, cx) {
    const p = props() ?? {};
    this.wanted = typeof p.note === "string" && p.note ? p.note : null;
    this.tab = "needs";
    this.needs = [];
    this.all = [];
    this.problems = [];
    this.error = null;
    this.vault = "";
    this.agent = "owner";
    this.loaded = false;
    this.loading = false;
    this.selected = null;
    this.body = null;
    this.bodyError = null;
    this.answering = false;
    this.actionError = null;
    this.busy = false;
    this.homePath = String(home() ?? "");
    this.handle = cx.focus_handle();
    this.answerState = TextareaState.new({ placeholder: "Write your answer", rows: 5 });
    this.answerState.on("change", (_e, cx) => {
      this.answer = this.answerState.value();
      cx.notify();
    });
    this.answer = "";
    this.handle.focus();
    cx.spawn(async (cx) => {
      await this.load(cx);
      cx.notify();
    });
  }

  // A load asked for while one runs runs again after it, so a refresh after Send or Close is never lost.
  load(cx) {
    this.again = true;
    if (!this.loadRun) {
      this.loadRun = (async () => {
        while (this.again) {
          this.again = false;
          await this.loadOnce(cx);
        }
        this.loadRun = null;
      })();
    }
    return this.loadRun;
  }

  async loadOnce(cx) {
    this.loading = true;
    cx.notify();
    let problems = [];
    let vault = "";
    let agent = "owner";
    let error = null;
    try {
      // One after the other: each pulls the vault, and two pulls at once trip over each other.
      const open = await kom(["vault", "open", "--json", "--as", "owner"]);
      const all = await kom(["vault", "find", "--json", "--limit", "50"]);
      if (open.code === 0) {
        const data = parse(open);
        this.needs = notesOf(data);
        vault = text(data.vault);
        agent = text(data.agent) || agent;
        problems.push(...problemsOf(data));
      } else {
        this.needs = [];
        error = errorText(open);
      }
      if (all.code === 0) {
        const data = parse(all);
        this.all = notesOf(data);
        vault = vault || text(data.vault);
        agent = text(data.agent) || agent;
        problems.push(...problemsOf(data));
      } else {
        // A find that refuses an empty search leaves only Needs you.
        this.all = null;
        if (this.tab === "all") this.tab = "needs";
        problems.push(errorText(all));
      }
      if (vault) this.vault = vault;
      this.agent = agent;
      this.problems = [...new Set(problems)];
      this.error = error;
      this.loaded = true;
      const selected = text(this.selected?.path);
      if (selected) {
        const fresh = [...this.needs, ...(this.all ?? [])].find((n) => text(n.path) === selected);
        if (fresh) this.selected = fresh;
        else this.selected = null;
      }
    } catch (e) {
      this.error = firstLine(e?.message ?? e) || "The mailbox couldn't read the vault.";
    } finally {
      this.loading = false;
      this.takeWanted(cx);
      cx.notify();
    }
  }

  takeWanted(cx) {
    const wanted = this.wanted;
    if (!wanted) return;
    this.wanted = null;
    const note = [...(this.all ?? []), ...this.needs].find((n) => text(n.path) === wanted);
    if (note) {
      if (this.all !== null && !this.needs.some((n) => text(n.path) === wanted)) this.tab = "all";
      this.select(note, cx);
      return;
    }
    // Not in the lists (an old note, say): read it by path.
    cx.spawn(async (cx) => {
      try {
        const data = parse(await kom(["vault", "read", wanted, "--json", "--as", "owner"]));
        if (!data.note || typeof data.note !== "object") throw new Error("The note isn't in the vault.");
        if (text(data.vault)) this.vault = text(data.vault);
        this.select(data.note, cx);
        const problems = problemsOf(data);
        if (problems.length) this.actionError = problems.join(" ");
      } catch (e) {
        this.actionError = firstLine(e?.message ?? e) || "The note couldn't be read.";
        cx.notify();
      }
    });
  }

  rows() {
    if (this.tab === "all") return this.all ?? [];
    return this.needs;
  }

  select(note, cx) {
    if (!note) return;
    this.selected = note;
    this.body = null;
    this.bodyError = null;
    this.answering = false;
    this.actionError = null;
    this.answer = "";
    this.answerState.set_value("");
    this.handle.focus();
    if (forOwner(note)) this.markRead(note, cx);
    this.readBody(note, cx);
    cx.notify();
  }

  markRead(note, cx) {
    const path = text(note.path);
    if (!path) return;
    cx.spawn(async (cx) => {
      try {
        // A read mark that couldn't be pulled or pushed says so, like every other vault problem.
        const problems = problemsOf(parse(await kom(["vault", "read", path, "--json", "--as", "owner"])));
        if (problems.length) this.actionError = problems.join(" ");
        await this.load(cx);
      } catch (e) {
        this.actionError = firstLine(e?.message ?? e) || "The read mark wasn't written.";
      }
      cx.notify();
    });
  }

  readBody(note, cx) {
    const path = text(note.path);
    if (!this.vault || !path) {
      this.bodyError = "The note has no vault path.";
      return;
    }
    if (!underHome(this.vault, this.homePath)) {
      this.bodyError = OUTSIDE;
      return;
    }
    const file = `${String(this.vault).replace(/[\\/]+$/, "")}/${path}`;
    cx.spawn(async (cx) => {
      try {
        const body = await readTextFile(file);
        if (text(this.selected?.path) !== path) return;
        this.body = body ?? "";
        this.bodyError = null;
      } catch (e) {
        if (text(this.selected?.path) !== path) return;
        this.bodyError = firstLine(e?.message ?? e) || "The note couldn't be read.";
      }
      cx.notify();
    });
  }

  move(delta, cx) {
    const rows = this.rows();
    if (!rows.length) return;
    const at = rows.findIndex((n) => text(n.path) === text(this.selected?.path));
    const next = at < 0 ? (delta > 0 ? 0 : rows.length - 1) : Math.max(0, Math.min(rows.length - 1, at + delta));
    this.select(rows[next], cx);
    cx.notify();
  }

  answerNote(cx) {
    if (!this.selected || this.busy) return;
    this.answering = true;
    this.actionError = null;
    this.answerState.set_value("");
    this.answer = "";
    this.answerState.focus();
    cx.notify();
  }

  send(cx) {
    const note = this.selected;
    const body = String(this.answer ?? "");
    if (!note || this.busy || !body.trim()) return;
    this.busy = true;
    this.actionError = null;
    cx.notify();
    const path = text(note.path);
    const to = text(note.from);
    const title = `Re: ${text(note.title)}`;
    cx.spawn(async (cx) => {
      try {
        const result = await kom([
          "vault", "post", "--as", "owner", "--to", to, "--kind", "answer",
          // Joined with `=`, so an answer that starts with `-` isn't taken for a flag.
          "--re", path, `--title=${title}`, `--body=${body}`,
        ]);
        if (result.code !== 0) throw new Error(errorText(result));
        this.answering = false;
        this.answer = "";
        this.answerState.set_value("");
        this.handle.focus();
        await this.load(cx);
      } catch (e) {
        this.actionError = firstLine(e?.message ?? e) || "The answer wasn't sent.";
      } finally {
        this.busy = false;
        cx.notify();
      }
    });
  }

  closeNote(cx) {
    const note = this.selected;
    if (!note || this.busy) return;
    this.busy = true;
    this.actionError = null;
    cx.notify();
    const path = text(note.path);
    cx.spawn(async (cx) => {
      try {
        const result = await kom(["vault", "done", path, "--as", "owner"]);
        if (result.code !== 0) throw new Error(errorText(result));
        await this.load(cx);
      } catch (e) {
        this.actionError = firstLine(e?.message ?? e) || "The note wasn't closed.";
      } finally {
        this.busy = false;
        cx.notify();
      }
    });
  }

  button(t, id, label, onClick) {
    return div()
      .id(id)
      .px(12)
      .py(5)
      .rounded(7)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .cursor_pointer()
      .text_size(12.5)
      .text_color(t.foreground)
      .hover((s) => s.bg(t.hover))
      .on_click(onClick)
      .child(label);
  }

  tabButton(t, id, label) {
    const on = this.tab === id;
    return h_flex()
      .id(`mail-tab-${id}`)
      .flex_1()
      .h(28)
      .items_center()
      .justify_center()
      .rounded(6)
      .cursor_pointer()
      .text_size(12)
      .font_medium()
      .when(on, (el) => el.bg(t.raised).text_color(t.foreground))
      .when(!on, (el) => el.text_color(t.muted).hover((s) => s.bg(t.hover)))
      .on_click((_e, cx) => {
        this.tab = id;
        this.answering = false;
        if (!this.rows().some((n) => text(n.path) === text(this.selected?.path))) this.selected = null;
        this.handle.focus();
        cx.notify();
      })
      .child(label);
  }

  row(t, note, index) {
    const selected = text(note.path) === text(this.selected?.path);
    const unread = forOwner(note) && !readByOwner(note);
    return v_flex()
      .id(`mail-row-${index}`)
      .w_full()
      .flex_none()
      .p(9)
      .gap(3)
      .rounded(7)
      .cursor_pointer()
      .when(selected, (el) => el.bg(t.raised))
      .when(!selected, (el) => el.hover((s) => s.bg(t.hover)))
      .on_click((_e, cx) => this.select(note, cx))
      .child(
        h_flex()
          .gap(6)
          .items_center()
          .when(unread, (el) => el.child(div().w(6).h(6).flex_none().rounded(3).bg(t.accent)))
          .child(div().flex_1().min_w(0).truncate().text_size(11).text_color(t.muted).child(`${text(note.from)} → ${text(note.to)}`))
          .child(div().flex_none().text_size(10.5).text_color(t.dim).child(ago(note.date))),
      )
      .child(div().w_full().truncate().text_size(12.5).text_color(t.foreground).child(text(note.title) || base(note.path) || "Note"));
  }

  notePane(t, cx) {
    const note = this.selected;
    if (!note) {
      return v_flex()
        .flex_1()
        .min_w(0)
        .min_h(0)
        .items_center()
        .justify_center()
        .child(div().text_size(12.5).text_color(t.muted).child(this.loaded ? "Select a note." : "Loading notes…"));
    }
    const owner = forOwner(note);
    const kind = text(note.kind) || "note";
    const file = [String(this.vault).replace(/[\\/]+$/, ""), text(note.path)].filter(Boolean).join("/");
    return v_flex()
      .flex_1()
      .min_w(0)
      .min_h(0)
      .child(
        v_flex()
          .flex_none()
          .p(16)
          .gap(5)
          .border_b(1)
          .border_color(t.border)
          .child(
            h_flex()
              .gap(8)
              .items_center()
              .child(div().flex_1().min_w(0).truncate().text_size(16).font_semibold().text_color(t.foreground).child(text(note.title) || base(note.path) || "Note"))
              .when(owner, (el) => el.child(this.button(t, "mail-answer", "Answer", (_e, cx) => this.answerNote(cx))))
              .when(owner, (el) => el.child(this.button(t, "mail-close", "Close", (_e, cx) => this.closeNote(cx)))),
          )
          .child(div().text_size(11.5).text_color(t.muted).child(`${text(note.from)} → ${text(note.to)} · ${kind} · ${stamp(note.date)}`))
          .when(Boolean(file), (el) => el.child(div().text_size(10.5).text_color(t.dim).truncate().child(file)))
          .when(Boolean(this.actionError), (el) => el.child(div().id("mail-error").text_size(11.5).text_color(t.blocked).child(this.actionError))),
      )
      .child(
        div()
          .id("mail-body")
          .flex_1()
          .min_h(0)
          .p(16)
          .overflow_y_scrollbar()
          .when(Boolean(this.bodyError), (el) => el.child(div().text_size(12.5).line_height("19px").text_color(this.bodyError === OUTSIDE ? t.muted : t.blocked).child(this.bodyError)))
          .when(!this.bodyError && this.body === null, (el) => el.child(div().text_size(12.5).text_color(t.muted).child("Reading the note…")))
          .when(!this.bodyError && this.body !== null, (el) =>
            el.child(v_flex().w_full().text_size(13).line_height("20px").text_color(t.foreground).children(lines(this.body))),
          ),
      )
      .when(this.answering, (el) =>
        el.child(
          v_flex()
            .flex_none()
            .gap(8)
            .p(12)
            .border_t(1)
            .border_color(t.border)
            .child(div().text_size(11.5).text_color(t.muted).child(`Answer ${text(note.from) || "the sender"}`))
            .child(Textarea.new(this.answerState).h(96).w_full().px(10).py(8).rounded(7).border_1().border_color(t.input).bg(t.background).text_size(13).text_color(t.foreground))
            .child(
              h_flex()
                .gap(8)
                .child(this.button(t, "mail-send", this.busy ? "Sending…" : "Send", (_e, cx) => this.send(cx)))
                .child(this.button(t, "mail-cancel", "Cancel", (_e, cx) => { this.answering = false; this.handle.focus(); cx.notify(); })),
            ),
        ),
      );
  }

  render(cx) {
    const t = theme();
    const rows = this.rows();
    return h_flex()
      .id("mailbox")
      .track_focus(this.handle)
      .w_full()
      .h(620)
      .min_h(0)
      .on_key_down((e, cx) => {
        const k = e.key;
        if (k === "escape") {
          closeOverlay(props()?.overlay);
        } else if ((k === "up" || k === "down") && !this.answering) {
          this.move(k === "up" ? -1 : 1, cx);
        } else return;
        cx.stop_propagation();
        cx.notify();
      })
      .child(
        v_flex()
          .w(260)
          .flex_none()
          .h_full()
          .min_h(0)
          .border_r(1)
          .border_color(t.border)
          .child(
            h_flex()
              .h(42)
              .flex_none()
              .px(8)
              .gap(4)
              .items_center()
              .border_b(1)
              .border_color(t.border)
              .child(this.tabButton(t, "needs", "Needs you"))
              .when(this.all !== null, (el) => el.child(this.tabButton(t, "all", "All notes"))),
          )
          .when(this.problems.length > 0, (el) =>
            el.child(
              v_flex()
                .flex_none()
                .px(10)
                .py(6)
                .gap(2)
                .border_b(1)
                .border_color(t.border)
                .children(this.problems.map((problem, i) => div().id(`mail-problem-${i}`).text_size(10.5).line_height("15px").text_color(t.blocked).child(problem))),
            ),
          )
          .when(Boolean(this.error), (el) => el.child(div().flex_none().px(10).py(6).text_size(11).text_color(t.blocked).child(this.error)))
          .child(
            v_flex()
              .id("mail-rows")
              .flex_1()
              .min_h(0)
              .p(6)
              .gap(2)
              .overflow_y_scrollbar()
              .when(this.loading && !this.loaded, (el) => el.child(div().p(8).text_size(11.5).text_color(t.muted).child("Loading notes…")))
              .children(rows.map((note, i) => this.row(t, note, i)))
              .when(this.loaded && rows.length === 0, (el) => el.child(div().p(8).text_size(11.5).text_color(t.muted).child(this.tab === "needs" ? "Nothing needs you." : "No notes yet."))),
          ),
      )
      .child(this.notePane(t, cx));
  }
}
