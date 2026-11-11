// The mailbox: the vault's notes in a sheet over the window. Needs you on the
// left, every note beside it, and the selected note on the right. A note to
// the Owner can be answered or closed; the Owner's read mark follows opening.
// The app's own mod: rewritten from the binary on every start.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState, TextView } from "gpui-base";
import { kom, readTextFile, home, props, closeOverlay, notify, theme } from "kompitum";

const OUTSIDE = "The vault is outside your home folder, so the mailbox can't read its notes.";
// The vault's marks are append-only, so a close written now could never be undone.
const UNDO_MS = 6000;

const text = (v) => (typeof v === "string" ? v : "");
const base = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
const cap = (s) => (s ? String(s).replace(/^./, (c) => c.toUpperCase()) : "");
const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

// The same test as frame/settings/vault.js
function isSyncProblem(words) {
  return /couldn't be updated|could not be pulled|may be behind|still working in the vault|nothing was pulled/i.test(String(words ?? ""));
}

// A problem reads `<path>: <reason>`. The note's name is its file name, without the date prefix and `.md`.
function problemParts(problem) {
  const match = /^(.*?): ([\s\S]*)$/.exec(String(problem));
  const path = match ? match[1] : String(problem);
  const reason = match ? match[2].trim() : "";
  const name = base(path).replace(/^\d{4}-\d{2}-\d{2}_\d{4}_/, "").replace(/\.md$/, "");
  return { name, reason: plainReason(reason) };
}

function plainReason(reason) {
  const unknown = /^has an unknown status: (.*)$/.exec(reason);
  if (unknown) return `Its status is “${unknown[1]}”, which Kompitum doesn't know.`;
  if (!reason) return "";
  const said = cap(reason);
  return /[.!?]$/.test(said) ? said : `${said}.`;
}

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
      if (message) return String(message);
    } catch {}
  }
  return String(result?.stderr || result?.stdout || "").trim() || `kom exited ${result?.code ?? "?"}`;
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

// A note is Markdown under a front matter block. The body starts after the
// closing `---` and the blank line that follows it, as the vault reads it.
function bodyOf(raw) {
  const normal = String(raw ?? "").replace(/^\uFEFF/, "").replace(/\r\n/g, "\n");
  if (!normal.startsWith("---\n")) return normal;
  const end = normal.indexOf("\n---", 3);
  if (end < 0) return normal;
  const after = normal.slice(end + 4);
  if (after && !after.startsWith("\n")) return normal;
  return after.replace(/^\n\n?/, "");
}

// The vault addresses a note to `all` to the Owner too.
const forOwner = (note) => ["owner", "all"].includes(text(note?.to));

function readByOwner(note) {
  const reads = Array.isArray(note?.read_by) ? note.read_by : [];
  return reads.some((r) => r && r.agent === "owner");
}

// A note with no recipient has only its sender; never a dangling arrow.
function route(note) {
  const from = text(note?.from);
  const to = text(note?.to);
  return to ? `${from} → ${to}` : from;
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
    // Show puts the notes that couldn't be filed in the reading pane, in place of a note.
    this.filedOpen = false;
    this.body = null;
    this.bodyError = null;
    this.answering = false;
    this.actionError = null;
    this.busy = false;
    // Notes whose Close is waiting out its undo window: still open in the vault.
    this.closing = [];
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
      this.error = String(e?.message ?? e ?? "").trim() || "The mailbox couldn't read the vault.";
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
        this.actionError = String(e?.message ?? e ?? "").trim() || "The note couldn't be read.";
        cx.notify();
      }
    });
  }

  rows() {
    const rows = this.tab === "all" ? (this.all ?? []) : this.needs;
    // A note in its undo window reads as closed, though the vault still has it open.
    const closing = this.closing.map((p) => p.path);
    return closing.length ? rows.filter((n) => !closing.includes(text(n.path))) : rows;
  }

  select(note, cx) {
    if (!note) return;
    this.selected = note;
    this.filedOpen = false;
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
        this.actionError = String(e?.message ?? e ?? "").trim() || "The read mark wasn't written.";
      }
      cx.notify();
    });
  }

  // Show: the notes that couldn't be filed, in the reading pane. Selecting a note leaves it.
  openFiled(cx) {
    this.selected = null;
    this.filedOpen = true;
    this.body = null;
    this.bodyError = null;
    this.answering = false;
    this.actionError = null;
    this.answer = "";
    this.answerState.set_value("");
    this.handle.focus();
    cx.notify();
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
        this.bodyError = String(e?.message ?? e ?? "").trim() || "The note couldn't be read.";
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
        this.actionError = String(e?.message ?? e ?? "").trim() || "The answer wasn't sent.";
      } finally {
        this.busy = false;
        cx.notify();
      }
    });
  }

  // Close keeps the note open in the vault for six seconds: `vault done` is
  // append-only, so Undo can only work by not having written yet. Leaving the
  // mailbox writes what is still owed instead of dropping it.
  closeNote(cx) {
    const note = this.selected;
    const path = text(note?.path);
    if (!note || !path || this.busy || this.closing.some((p) => p.path === path)) return;
    const pending = { note, path, timer: null };
    pending.timer = cx.timer.after(UNDO_MS, (cx) => this.finishClose(pending, cx));
    this.closing.push(pending);
    this.actionError = null;
    this.answering = false;
    this.answer = "";
    this.answerState.set_value("");
    this.selected = null;
    this.body = null;
    this.bodyError = null;
    this.handle.focus();
    cx.notify();
  }

  undoClose(cx) {
    const pending = this.closing[this.closing.length - 1];
    if (!pending) return;
    pending.timer?.cancel();
    this.closing = this.closing.filter((p) => p !== pending);
    this.actionError = null;
    this.select(pending.note, cx);
  }

  finishClose(pending, cx) {
    if (!this.closing.includes(pending)) return;
    pending.timer?.cancel();
    this.closing = this.closing.filter((p) => p !== pending);
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const result = await kom(["vault", "done", pending.path, "--as", "owner"]);
        if (result.code !== 0) throw new Error(errorText(result));
        await this.load(cx);
      } catch (e) {
        // The vault refused it, so the note is still open: show it again with the words.
        if (!this.selected) this.select(pending.note, cx);
        this.actionError = String(e?.message ?? e ?? "").trim() || "The note wasn't closed.";
      } finally {
        cx.notify();
      }
    });
  }

  // The mailbox's own `close(cx)`: the six seconds end when it leaves the screen.
  async close(cx) {
    const pending = this.closing;
    this.closing = [];
    for (const p of pending) {
      p.timer?.cancel();
      try {
        const result = await kom(["vault", "done", p.path, "--as", "owner"]);
        if (result.code !== 0) throw new Error(errorText(result));
      } catch (e) {
        notify({
          title: "The note wasn't closed",
          body: String(e?.message ?? e ?? "").trim() || "The vault didn't answer.",
          level: "error",
        });
      }
    }
  }

  // Quiet footer actions, and the one filled action for the note.
  textButton(t, id, label, onClick) {
    return div()
      .id(id)
      .px(10)
      .py(5)
      .rounded(7)
      .cursor_pointer()
      .text_size(12.5)
      .text_color(t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      .on_click(onClick)
      .child(label);
  }

  primaryButton(t, id, label, onClick) {
    return div()
      .id(id)
      .px(12)
      .py(5)
      .rounded(7)
      .cursor_pointer()
      .bg(t.accent_fill)
      .text_color(t.accent_foreground)
      .text_size(12.5)
      .font_medium()
      .hover((s) => s.opacity(0.9))
      .on_click(onClick)
      .child(label);
  }

  tabs(t) {
    const items = [["needs", "Needs you"]];
    if (this.all !== null) items.push(["all", "All notes"]);
    return h_flex()
      .id("mail-tabs")
      .self_start()
      .p(2)
      .gap(2)
      .rounded(8)
      .border_1()
      .border_color(t.border)
      .bg(t.background)
      .children(
        items.map(([id, label]) => {
          const on = this.tab === id;
          return div()
            .id(`mail-tab-${id}`)
            .px(12)
            .py(4)
            .rounded(6)
            .cursor_pointer()
            .text_size(13)
            .when(on, (el) => el.bg(t.raised).text_color(t.foreground).font_medium())
            .when(!on, (el) => el.text_color(t.muted).hover((s) => s.text_color(t.foreground)))
            .on_click((_e, cx) => {
              this.tab = id;
              this.answering = false;
              if (!this.rows().some((n) => text(n.path) === text(this.selected?.path))) this.selected = null;
              this.handle.focus();
              cx.notify();
            })
            .child(label);
        }),
      );
  }

  row(t, note, index) {
    const selected = text(note.path) === text(this.selected?.path);
    const waits = forOwner(note) && !readByOwner(note);
    return v_flex()
      .id(`mail-row-${index}`)
      .w_full()
      .flex_none()
      .px(10)
      .py(12)
      .gap(4)
      .rounded(8)
      .cursor_pointer()
      .when(selected, (el) => el.bg(t.selected))
      .when(!selected, (el) => el.hover((s) => s.bg(t.hover)))
      .on_click((_e, cx) => this.select(note, cx))
      .child(
        h_flex()
          .w_full()
          .gap(6)
          .items_center()
          .child(div().flex_1().min_w(0).truncate().text_size(11.5).text_color(t.dim).child(route(note)))
          .child(div().flex_none().text_size(11.5).text_color(t.dim).child(ago(note.date))),
      )
      .child(
        h_flex()
          .w_full()
          .gap(6)
          .items_center()
          .when(waits, (el) => el.child(div().w(6).h(6).flex_none().rounded(3).bg(t.accent)))
          .child(div().flex_1().min_w(0).truncate().text_size(13.5).text_color(t.foreground).child(text(note.title) || base(note.path) || "Note")),
      );
  }

  bodyPane(t) {
    const markdown = bodyOf(this.body);
    const empty = !markdown.trim();
    return div()
      .id("mail-body")
      .flex_1()
      .min_h(0)
      .px(24)
      .py(20)
      .overflow_y_scrollbar()
      .when(Boolean(this.bodyError), (el) =>
        el.child(
          div()
            .max_w(680)
            .text_size(13)
            .line_height("20px")
            .text_color(this.bodyError === OUTSIDE ? t.muted : t.blocked)
            .tooltip(this.bodyError)
            .child(firstLine(this.bodyError) || this.bodyError),
        ),
      )
      .when(!this.bodyError && this.body === null, (el) =>
        el.child(div().text_size(13).text_color(t.muted).child("Reading the note…")),
      )
      .when(!this.bodyError && this.body !== null, (el) =>
        el.child(
          v_flex()
            .w_full()
            .max_w(680)
            .when(empty, (col) => col.child(div().text_size(14).text_color(t.dim).child("This note has no text.")))
            .when(!empty, (col) =>
              col.child(TextView.markdown("mail-body-text", markdown).text_size(14).text_color(t.foreground).selectable(true)),
            ),
        ),
      );
  }

  // The notes that couldn't be filed, two lines each: the name, then why.
  filedPane(t, filed) {
    return v_flex()
      .id("mail-filed")
      .flex_1()
      .min_w(0)
      .min_h(0)
      .h_full()
      .p(24)
      .gap(12)
      .overflow_y_scrollbar()
      .children(filed.map((problem, i) => {
        const { name, reason } = problemParts(problem);
        return v_flex()
          .id(`mail-filed-${i}`)
          .flex_none()
          .gap(2)
          .child(div().text_size(14).text_color(t.foreground).child(name))
          .when(Boolean(reason), (el) => el.child(div().text_size(12).text_color(t.dim).child(reason)));
      }));
  }

  notePane(t, cx) {
    const note = this.selected;
    if (!note && this.filedOpen) {
      const filed = this.problems.filter((problem) => !isSyncProblem(problem));
      if (filed.length) return this.filedPane(t, filed);
    }
    if (!note) {
      if (this.closing.length) {
        return v_flex()
          .flex_1()
          .min_w(0)
          .min_h(0)
          .h_full()
          .child(div().flex_1())
          .child(
            h_flex()
              .flex_none()
              .px(16)
              .py(10)
              .gap(8)
              .items_center()
              .border_t(1)
              .border_color(t.border)
              .child(this.textButton(t, "mail-undo", "Closed. Undo", (_e, cx) => this.undoClose(cx))),
          );
      }
      return v_flex()
        .flex_1()
        .min_w(0)
        .min_h(0)
        .h_full()
        .items_center()
        .justify_center()
        // While the list loads it says so itself; the reading pane stays empty.
        .when(this.loaded, (el) => el.child(div().text_size(13).text_color(t.dim).child("Pick a note to read it.")));
    }
    const owner = forOwner(note);
    const kind = text(note.kind) || "note";
    const meta = [route(note), kind, stamp(note.date)].filter(Boolean).join(" · ");
    return v_flex()
      .id("mail-note")
      .flex_1()
      .min_w(0)
      .min_h(0)
      .h_full()
      .child(
        v_flex()
          .flex_none()
          .px(24)
          .pt(20)
          .pb(14)
          .gap(5)
          .border_b(1)
          .border_color(t.border)
          .child(div().id("mail-note-title").w_full().max_w(680).truncate().text_size(18).font_semibold().text_color(t.foreground).child(text(note.title) || base(note.path) || "Note"))
          .child(div().w_full().max_w(680).truncate().text_size(12).text_color(t.dim).child(meta)),
      )
      .child(this.bodyPane(t))
      .when(owner || this.answering || Boolean(this.actionError), (el) =>
        el.child(
          v_flex()
            .flex_none()
            .border_t(1)
            .border_color(t.border)
            .when(this.answering, (panel) =>
              panel.child(
                v_flex()
                  .gap(8)
                  .px(16)
                  .pt(12)
                  .child(div().text_size(12).text_color(t.muted).child(`Answer ${text(note.from) || "the sender"}`))
                  .child(Textarea.new(this.answerState).h(96).w_full().px(10).py(8).rounded(7).border_1().border_color(t.input).bg(t.background).text_size(13).text_color(t.foreground)),
              ),
            )
            .child(
              h_flex()
                .flex_none()
                .px(16)
                .py(10)
                .gap(8)
                .items_center()
                .when(Boolean(this.actionError), (bar) =>
                  bar.child(div().id("mail-error").flex_1().min_w(0).truncate().text_size(11.5).text_color(t.blocked).tooltip(this.actionError).child(firstLine(this.actionError) || this.actionError)),
                )
                .child(div().flex_1())
                .when(!this.answering && owner, (bar) =>
                  bar
                    .child(this.textButton(t, "mail-close", "Close", (_e, cx) => this.closeNote(cx)))
                    .child(this.primaryButton(t, "mail-answer", "Answer", (_e, cx) => this.answerNote(cx))),
                )
                .when(this.answering, (bar) =>
                  bar
                    .child(this.textButton(t, "mail-cancel", "Cancel", (_e, cx) => { this.answering = false; this.handle.focus(); cx.notify(); }))
                    .child(this.primaryButton(t, "mail-send", this.busy ? "Sending…" : "Send", (_e, cx) => this.send(cx))),
                ),
            ),
        ),
      );
  }

  // The sheet's top: its name, and the close that the Settings sheet draws too.
  sheetHeader(t) {
    return h_flex()
      .h(52)
      .w_full()
      .flex_none()
      .pl(16)
      .pr(12)
      .items_center()
      .border_b(1)
      .border_color(t.border)
      .child(div().flex_1().min_w(0).text_size(15).font_semibold().text_color(t.foreground).child("Mailbox"))
      .child(
        div()
          .id("mail-sheet-close")
          .w(28)
          .h(28)
          .flex_none()
          .rounded(6)
          .flex()
          .items_center()
          .justify_center()
          .text_color(t.muted)
          .cursor_pointer()
          .hover((s) => s.bg(t.hover).text_color(t.foreground))
          .on_click(() => closeOverlay(props()?.overlay))
          .child(svg("mods/workspace-chrome/x.svg").size(14)),
      );
  }

  render(cx) {
    const t = theme();
    const rows = this.rows();
    const sync = this.problems.filter((problem) => isSyncProblem(problem));
    const filed = this.problems.filter((problem) => !isSyncProblem(problem));
    return v_flex()
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
      .child(this.sheetHeader(t))
      .child(
        h_flex()
          .flex_1()
          .min_h(0)
          .child(
            v_flex()
              .w(300)
              .flex_none()
              .h_full()
              .min_h(0)
              .border_r(1)
              .border_color(t.border)
              .child(
                h_flex()
                  .flex_none()
                  .px(10)
                  .py(8)
                  .border_b(1)
                  .border_color(t.border)
                  .child(this.tabs(t)),
              )
              .when(sync.length > 0, (el) =>
                el.child(
                  v_flex()
                    .flex_none()
                    .px(10)
                    .py(6)
                    .gap(2)
                    .border_b(1)
                    .border_color(t.border)
                    .children(sync.map((problem, i) => div().id(`mail-problem-${i}`).text_size(12).line_height("17px").text_color(t.blocked).tooltip(problem).child(firstLine(problem) || problem))),
                ),
              )
              .when(Boolean(this.error), (el) => el.child(div().id("mail-load-error").flex_none().px(10).py(6).text_size(12).text_color(t.blocked).tooltip(this.error).child(firstLine(this.error) || this.error)))
              .child(
                v_flex()
                  .id("mail-rows")
                  .flex_1()
                  .min_h(0)
                  .p(8)
                  .gap(2)
                  .overflow_y_scrollbar()
                  .when(this.loading && !this.loaded, (el) => el.child(div().p(8).text_size(11.5).text_color(t.muted).child("Loading notes…")))
                  .children(rows.map((note, i) => this.row(t, note, i)))
                  .when(this.loaded && rows.length === 0, (el) => el.child(div().p(8).text_size(12.5).text_color(t.dim).child(this.tab === "all" ? "No notes yet." : "Nothing needs you."))),
              )
              .when(filed.length > 0, (el) =>
                el.child(
                  h_flex()
                    .id("mail-filed-bar")
                    .h(30)
                    .w_full()
                    .flex_none()
                    .px(12)
                    .gap(8)
                    .items_center()
                    .border_t(1)
                    .border_color(t.border)
                    .child(div().flex_1().min_w(0).truncate().text_size(12).text_color(t.muted).child(`${filed.length} ${filed.length === 1 ? "note" : "notes"} couldn't be filed`))
                    .child(
                      div()
                        .id("mail-show-filed")
                        .px(6)
                        .py(2)
                        .rounded(5)
                        .cursor_pointer()
                        .text_size(12)
                        .text_color(t.accent)
                        .hover((s) => s.text_color(t.foreground))
                        .on_click((_e, cx) => this.openFiled(cx))
                        .child("Show"),
                    ),
                ),
              ),
          )
          .child(this.notePane(t, cx)),
      );
  }
}
