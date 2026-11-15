// The lead switch, and the status bar's right end: Notes with the agent that
// joined through Connect, and a click to that system's own app when one is
// installed. A hand-off is a note, not a lock. Nothing is gathered or sent on
// its own: the pill opens a window, and the window's send button posts the note.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, TextView } from "gpui-base";
import { kom, theme, notify, openOverlay, closeOverlay, shared, setShared, props, appLink, openLink, settings, text } from "kompitum";

const LEAD_KEY = "lead";
const TASK_SECTIONS = [["review", text("lead.to-review")], ["running", text("lead.running")], ["queued", text("lead.queued")]];

const cap = (word) => (word ? word[0].toUpperCase() + word.slice(1) : "");

// The first system that joined through Connect, or null: who the notes are with, and its app's link.
function joined() {
  const connect = settings()?.connect ?? {};
  const id = Object.keys(connect).sort().find((key) => connect[key]?.agent);
  if (!id) return null;
  const agent = String(connect[id].agent);
  return { id, agent, name: connect[id].name || cap(agent), link: connect[id].link || `${id}://` };
}

function asState(value) {
  if (!value || typeof value !== "object") return null;
  if (value.who !== "opus" && value.who !== "manager") return null;
  const at = Number(value.at);
  return { who: value.who, at: Number.isFinite(at) ? at : 0 };
}

function readStored() {
  try {
    const raw = localStorage.getItem(LEAD_KEY);
    return raw ? asState(JSON.parse(raw)) : null;
  } catch {
    return null;
  }
}

function writeStored(state) {
  try {
    localStorage.setItem(LEAD_KEY, JSON.stringify(state));
  } catch {}
}

// The pill and a hand-off window are separate copies of a mod, and each copy
// has its own localStorage. The shared value carries a flip between them; the
// copy showing the pill writes it down as well, so localStorage keeps who
// leads and when it last changed.
function readLead() {
  const local = readStored() ?? { who: "opus", at: 0 };
  const remote = asState(shared(LEAD_KEY));
  if (remote && remote.at > local.at) {
    writeStored(remote);
    return remote;
  }
  return local;
}

function writeLead(who) {
  const state = { who, at: Date.now() };
  writeStored(state);
  setShared(LEAD_KEY, state);
  return state;
}

function messageOf(value) {
  if (!value || typeof value !== "object") return null;
  if (typeof value.message === "string" && value.message.trim()) return value.message.trim();
  const error = value.error;
  if (typeof error === "string" && error.trim()) return error.trim();
  if (error && typeof error === "object" && typeof error.message === "string" && error.message.trim()) {
    return error.message.trim();
  }
  return null;
}

function errorText(e) {
  if (e == null) return text("lead.command-failed");
  if (typeof e === "string") return e;
  if (typeof e.message === "string" && e.message.trim()) return e.message.trim();
  for (const raw of [e.stderr, e.stdout]) {
    if (typeof raw !== "string" || !raw.trim()) continue;
    try {
      const said = messageOf(JSON.parse(raw.trim()));
      if (said) return said;
    } catch {}
    return raw.trim();
  }
  if (typeof e.code === "number") return text("lead.command-exit", { code: e.code });
  return String(e);
}

async function komJson(args) {
  const r = await kom(args);
  if (!r || r.code !== 0) throw new Error(errorText(r));
  let data;
  try {
    data = JSON.parse(r.stdout);
  } catch {
    throw new Error(text("lead.no-json", { message: String(r.stdout ?? "").trim() || text("lead.empty-output") }));
  }
  if (data && data.error) throw new Error(messageOf(data) ?? text("lead.command-failed"));
  return data;
}

async function komOk(args) {
  const r = await kom(args);
  if (!r || r.code !== 0) throw new Error(errorText(r));
  return r;
}

function pad(n) {
  return String(n).padStart(2, "0");
}

function localTime(seconds) {
  const d = new Date(seconds * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function resetLine(status) {
  const claude = status && status.claude;
  const times = [];
  if (claude) {
    for (const key of ["five_hour_resets_at", "seven_day_resets_at"]) {
      const n = Number(claude[key]);
      if (Number.isFinite(n) && n > 0) times.push(n);
    }
  }
  if (!times.length) return text("lead.reset-unknown");
  return text("lead.back-at", { when: localTime(Math.max(...times)) });
}

// A Claude window the status line says is used up, with a reset still to come.
function claudeOut(status) {
  const claude = status && status.claude;
  if (!claude) return false;
  return ["five_hour", "seven_day"].some((window) => {
    const used = Number(claude[`${window}_used_percentage`]);
    const reset = Number(claude[`${window}_resets_at`]);
    return Number.isFinite(used) && used >= 100 && Number.isFinite(reset) && reset > 0;
  });
}

function minutes(seconds) {
  const n = Number(seconds);
  return Number.isFinite(n) ? `${Math.max(0, Math.floor(n / 60))}m` : "";
}

function money(cost) {
  const n = Number(cost);
  return Number.isFinite(n) ? `$${n.toFixed(2)}` : "";
}

function taskLine(task) {
  const bits = [];
  if (task.repo) bits.push(String(task.repo));
  if (task.worker) bits.push(String(task.worker));
  bits.push(text("lead.started-by", { by: task.by ? String(task.by) : text("lead.unknown") }));
  const took = minutes(task.seconds);
  if (took) bits.push(took);
  const cost = money(task.cost_usd);
  if (cost) bits.push(cost);
  return text("lead.task-line", { name: task.name ? String(task.name) : text("lead.unnamed"), bits: bits.join(", ") });
}

function managerTaskLine(task) {
  const bits = [];
  if (task.state) bits.push(String(task.state));
  const cost = money(task.cost_usd);
  if (cost) bits.push(cost);
  const name = task.name ? String(task.name) : text("lead.unnamed");
  return bits.length ? text("lead.manager-task-line", { name, bits: bits.join(", ") }) : name;
}

function openNoteLine(note) {
  const title = note.title ? String(note.title) : text("lead.untitled");
  const from = note.from ? String(note.from) : text("lead.unknown");
  return text("lead.open-note-line", { title, from, path: note.path ? String(note.path) : text("lead.no-path") });
}

function backNoteLine(note) {
  return text("lead.back-note-line", { title: note.title ? String(note.title) : text("lead.untitled"), path: note.path ? String(note.path) : text("lead.no-path") });
}

function noteLines(notes) {
  return notes.map((note) => `- ${openNoteLine(note)}`).join("\n");
}

function asArray(value) {
  return Array.isArray(value) ? value : [];
}

function markdown(parts) {
  return `${parts.filter(Boolean).join("\n\n")}\n`;
}

async function handoffMarkdown() {
  const [status, vault] = await Promise.all([
    komJson(["status", "--json"]),
    komJson(["vault", "open", "--json", "--as", "opus"]),
  ]);
  const parts = [
    `The Owner handed Manager the lead.${claudeOut(status) ? " Claude's quota is out." : ""}`,
    resetLine(status),
  ];
  for (const [key, title] of TASK_SECTIONS) {
    const tasks = asArray(status.tasks && status.tasks[key]);
    if (tasks.length) parts.push(`${text("lead.section", { title })}\n${tasks.map((task) => `- ${taskLine(task)}`).join("\n")}`);
  }
  const notes = asArray(vault.notes).filter((note) => note && note.status === "open");
  if (notes.length) parts.push(`${text("lead.open-for-opus")}\n${noteLines(notes)}`);
  parts.push(text("lead.raw-list"));
  return markdown(parts);
}

async function backMarkdown(at) {
  const [status, vault] = await Promise.all([
    komJson(["status", "--json"]),
    komJson(["vault", "thread", "manager", "--json", "--as", "opus"]),
  ]);
  const parts = [text("lead.back-first")];
  const tasks = [];
  if (status.tasks && typeof status.tasks === "object") {
    for (const section of Object.values(status.tasks)) {
      for (const task of asArray(section)) {
        if (task && task.by === "manager") tasks.push(task);
      }
    }
  }
  if (tasks.length) {
    parts.push(`${text("lead.manager-tasks")}\n${tasks.map((task) => `- ${managerTaskLine(task)}`).join("\n")}`);
  }
  const notes = asArray(vault.notes).filter((note) => {
    if (!note || note.from !== "manager") return false;
    const when = Date.parse(note.date);
    return Number.isFinite(when) && when > at;
  });
  if (notes.length) {
    parts.push(`${text("lead.manager-notes")}\n${notes.map((note) => `- ${backNoteLine(note)}`).join("\n")}`);
  }
  return markdown(parts);
}

export default class Lead extends View {
  init(_props, cx) {
    // An overlay's props come through props(), as in every other mod; init's own argument does not carry them.
    const p = props();
    this.handle = cx.focus_handle();
    this.handoff = Boolean(p && p.mode === "handoff" && typeof p.markdown === "string");
    this.sending = false;
    this.busy = false;
    this.apps = {};
    if (this.handoff) cx.timer.after(80, () => this.handle.focus());
  }

  flip(cx) {
    if (this.busy) return;
    const state = readLead();
    const to = state.who === "manager" ? "opus" : "manager";
    this.busy = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const body = to === "opus" ? await backMarkdown(state.at) : await handoffMarkdown();
        openOverlay({
          mod: "lead",
          kind: "window",
          key: "lead-handoff",
          width: 640,
          height: 540,
          props: {
            mode: "handoff",
            title: to === "opus" ? text("lead.take-back-title") : text("lead.handoff-title"),
            markdown: body,
            send: to,
          },
        });
      } catch (e) {
        notify({ title: text("lead.did-not-change"), body: errorText(e), level: "error" });
      } finally {
        this.busy = false;
        cx.notify();
      }
    });
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

  door(other) {
    const link = String(props()?.door ?? other.link);
    const scheme = link.split(":")[0];
    const fail = (body) => notify({ title: text("lead.could-not-open", { name: other.name }), body, level: "error" });
    let app;
    try {
      app = appLink(scheme);
    } catch (e) {
      return fail(errorText(e));
    }
    if (!app) return fail(text("lead.nothing-opens", { scheme }));
    openLink(link);
  }

  send(cx) {
    const p = props() ?? {};
    if (this.sending) return;
    const to = p.send === "opus" ? "opus" : "manager";
    const title = to === "opus" ? text("lead.back-title") : text("lead.hold-title");
    this.sending = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await komOk([
          "vault",
          "post",
          "--as",
          "owner",
          "--to",
          to,
          "--kind",
          "request",
          "--title",
          title,
          "--body",
          String(p.markdown ?? ""),
        ]);
        let result = null;
        try {
          result = JSON.parse(r.stdout);
        } catch {}
        writeLead(to);
        closeOverlay(p.overlay);
        if (result && result.pushed === false) {
          notify({
            title: to === "manager" ? text("lead.handoff-written") : text("lead.note-written"),
            body:
              typeof result.said === "string" && result.said.trim()
                ? result.said.trim()
                : text("lead.note-local"),
            level: "warning",
          });
        } else {
          notify({
            title: to === "manager" ? text("lead.handed-over") : text("lead.taken-back"),
            body: to === "manager" ? text("lead.handoff-went") : text("lead.back-title"),
            level: "success",
          });
        }
      } catch (e) {
        this.sending = false;
        cx.notify();
        notify({ title: text("lead.note-not-sent"), body: errorText(e), level: "error" });
      }
    });
  }

  button(id, text, primary, action, t) {
    return h_flex()
      .id(id)
      .h(30)
      .px(12)
      .items_center()
      .rounded(7)
      .border_1()
      .border_color(primary ? t.solid : t.input)
      .bg(primary ? t.solid : t.raised)
      .text_color(primary ? t.solid_foreground : t.foreground)
      .text_size(13)
      .font_medium()
      .cursor_pointer()
      .relative()
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click(action)
      // Hover and press: the theme's state layers over the fill (a style sets only one background).
      .child(div().absolute().top(-1).left(-1).right(-1).bottom(-1).rounded(7).hover((s) => s.bg(t.hover)).active((s) => s.bg(t.selected)))
      .child(text);
  }

  // The status bar's right end: nothing until an agent joins through Connect; then Notes with it,
  // and, when an app answers its link, a divider and a click to that app. No lead name appears.
  renderPill(t) {
    const item = (id) => h_flex().id(id).h_full().px(6).gap(5).items_center().rounded(5).cursor_pointer()
      .text_color(t.muted).hover((s) => s.text_color(t.foreground))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation());
    let bar = h_flex()
      .id("lead-root")
      .size_full()
      .justify_end()
      .items_center()
      .gap(6)
      .pr(10)
      .bg(t.chrome)
      .border_t(1)
      .border_color(t.border)
      .text_size(13);
    const other = joined();
    if (!other) return bar;
    bar = bar.child(
      item("lead-notes")
        .tooltip(text("lead.notes-with", { agent: cap(other.agent) }))
        .on_click(() => openOverlay({ mod: "line", kind: "popup", width: 420, height: 460, props: { with: other.agent } }))
        .child(text("lead.notes"))
        .child(svg("mods/lead/chevron-up.svg").size(12)),
    );
    if (!this.appFor(other.link)) return bar;
    return bar
      .child(div().w(1).h(12).bg(t.border))
      .child(
        item("lead-door")
          .tooltip(text("lead.open-app", { name: other.name }))
          .on_click(() => this.door(other))
          .child(other.name)
          .child(svg("mods/lead/arrow-up-right.svg").size(11)),
      );
  }

  renderHandoff(t) {
    const p = props() ?? {};
    const sendLabel = p.send === "opus" ? text("lead.send-opus") : text("lead.send-manager");
    return v_flex()
      .id("lead-handoff")
      .size_full()
      .min_h(0)
      .bg(t.popover)
      .text_color(t.foreground)
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key === "escape") {
          closeOverlay(p.overlay);
          cx.stop_propagation();
        }
      })
      .child(
        h_flex()
          .h(38)
          .px(14)
          .items_center()
          .justify_between()
          .bg(t.chrome)
          .border_b(1)
          .border_color(t.border)
          .child(div().text_size(14).font_semibold().child(p.title ?? text("lead.the-lead")))
          .child(
            h_flex()
              .id("lead-close")
              .accessibility_label(text("lead.close"))
              .w(22)
              .h(22)
              .items_center()
              .justify_center()
              .rounded(6)
              .text_color(t.dim)
              .cursor_pointer()
              .hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
              .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
              .on_click(() => closeOverlay(p.overlay))
              .child(svg("mods/workspace-chrome/x.svg").size(12)),
          ),
      )
      .child(
        v_flex()
          .id("lead-body")
          .flex_1()
          .min_h(0)
          .px(16)
          .py(12)
          .overflow_y_scrollbar()
          .child(
            TextView.markdown("lead-markdown", String(p.markdown ?? ""))
              .text_size(13)
              .text_color(t.foreground)
              .selectable(true),
          ),
      )
      .child(
        h_flex()
          .h(54)
          .px(16)
          .items_center()
          .justify_end()
          .gap(8)
          .border_t(1)
          .border_color(t.border)
          .child(this.button("lead-cancel", text("lead.cancel"), false, () => closeOverlay(p.overlay), t))
          .child(
            this.button(
              "lead-send",
              this.sending ? text("lead.sending") : sendLabel,
              true,
              (_e, cx) => this.send(cx),
              t,
            ),
          ),
      );
  }

  render() {
    const t = theme();
    return this.handoff ? this.renderHandoff(t) : this.renderPill(t);
  }
}
