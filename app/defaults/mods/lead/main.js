// The Lead switch and the Door: who leads between Opus and Manager, and one
// click to Manager's own chat. A hand-off is a note, not a lock. Nothing is
// gathered or sent on its own: the pill opens a window, and the window's send
// button posts the note.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, TextView } from "gpui-base";
import { kom, theme, notify, openOverlay, closeOverlay, shared, setShared, props, appLink, openLink } from "kompitum";

const LEAD_KEY = "lead";
const DOOR = "grokbot://";
const TASK_SECTIONS = [["review", "To review"], ["running", "Running"], ["queued", "Queued"]];

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
  if (e == null) return "The command failed.";
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
  if (typeof e.code === "number") return `The command failed with exit code ${e.code}.`;
  return String(e);
}

async function komJson(args) {
  const r = await kom(args);
  if (!r || r.code !== 0) throw new Error(errorText(r));
  let data;
  try {
    data = JSON.parse(r.stdout);
  } catch {
    throw new Error(`The command returned no JSON: ${String(r.stdout ?? "").trim() || "empty output"}`);
  }
  if (data && data.error) throw new Error(messageOf(data) ?? "The command failed.");
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
  if (!times.length) return "Claude's reset time is unknown.";
  return `Claude is back at ${localTime(Math.max(...times))} (local time).`;
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
  bits.push(`started by ${task.by ? String(task.by) : "unknown"}`);
  const took = minutes(task.seconds);
  if (took) bits.push(took);
  const cost = money(task.cost_usd);
  if (cost) bits.push(cost);
  return `${task.name ? String(task.name) : "(unnamed)"} — ${bits.join(", ")}`;
}

function managerTaskLine(task) {
  const bits = [];
  if (task.state) bits.push(String(task.state));
  const cost = money(task.cost_usd);
  if (cost) bits.push(cost);
  return `${task.name ? String(task.name) : "(unnamed)"}${bits.length ? ` — ${bits.join(", ")}` : ""}`;
}

function openNoteLine(note) {
  const title = note.title ? String(note.title) : "(untitled)";
  const from = note.from ? String(note.from) : "unknown";
  return `${title} — from ${from} — ${note.path ? String(note.path) : "no path"}`;
}

function backNoteLine(note) {
  return `${note.title ? String(note.title) : "(untitled)"} — ${note.path ? String(note.path) : "no path"}`;
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
    "The Owner handed Manager the lead because Claude's quota is out.",
    resetLine(status),
  ];
  for (const [key, title] of TASK_SECTIONS) {
    const tasks = asArray(status.tasks && status.tasks[key]);
    if (tasks.length) parts.push(`## ${title}\n${tasks.map((task) => `- ${taskLine(task)}`).join("\n")}`);
  }
  const notes = asArray(vault.notes).filter((note) => note && note.status === "open");
  if (notes.length) parts.push(`## Open for Opus\n${noteLines(notes)}`);
  parts.push("This is the raw list: no summary was written.");
  return markdown(parts);
}

async function backMarkdown(at) {
  const [status, vault] = await Promise.all([
    komJson(["status", "--json"]),
    komJson(["vault", "thread", "manager", "--json", "--as", "opus"]),
  ]);
  const parts = ["The lead is back with Opus."];
  const tasks = [];
  if (status.tasks && typeof status.tasks === "object") {
    for (const section of Object.values(status.tasks)) {
      for (const task of asArray(section)) {
        if (task && task.by === "manager") tasks.push(task);
      }
    }
  }
  if (tasks.length) {
    parts.push(`## Manager's tasks since the hand-off\n${tasks.map((task) => `- ${managerTaskLine(task)}`).join("\n")}`);
  }
  const notes = asArray(vault.notes).filter((note) => {
    if (!note || note.from !== "manager") return false;
    const when = Date.parse(note.date);
    return Number.isFinite(when) && when > at;
  });
  if (notes.length) {
    parts.push(`## Manager's notes since the hand-off\n${notes.map((note) => `- ${backNoteLine(note)}`).join("\n")}`);
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
            title: to === "opus" ? "Take the lead back for Opus" : "Hand the lead to Manager",
            markdown: body,
            send: to,
          },
        });
      } catch (e) {
        notify({ title: "The lead did not change", body: errorText(e), level: "error" });
      } finally {
        this.busy = false;
        cx.notify();
      }
    });
  }

  door() {
    const link = String(props()?.door ?? DOOR);
    const scheme = link.split(":")[0];
    const fail = (body) => notify({ title: "Could not open Manager", body, level: "error" });
    let app;
    try {
      app = appLink(scheme);
    } catch (e) {
      return fail(errorText(e));
    }
    if (!app) return fail(`Nothing on this computer opens ${scheme}:// links.`);
    openLink(link);
  }

  send(cx) {
    const p = props() ?? {};
    if (this.sending) return;
    const to = p.send === "opus" ? "opus" : "manager";
    const title = to === "opus" ? "The lead is back with Opus" : "You have the lead until Claude is back";
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
            title: to === "manager" ? "Hand-off written" : "Note written",
            body:
              typeof result.said === "string" && result.said.trim()
                ? result.said.trim()
                : "The note is written but still only on this computer.",
            level: "warning",
          });
        } else {
          notify({
            title: to === "manager" ? "Lead handed over" : "Lead taken back",
            body: to === "manager" ? "The hand-off went to Manager." : "The lead is back with Opus.",
            level: "success",
          });
        }
      } catch (e) {
        this.sending = false;
        cx.notify();
        notify({ title: "The note was not sent", body: errorText(e), level: "error" });
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
      .text_size(12.5)
      .font_medium()
      .cursor_pointer()
      .hover((s) => s.opacity(0.88))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click(action)
      .child(text);
  }

  renderPill(t) {
    const state = readLead();
    const manager = state.who === "manager";
    return h_flex()
      .id("lead-root")
      .h_full()
      .items_center()
      .gap(6)
      .text_size(11.5)
      .child(
        h_flex()
          .id("lead-pill")
          .h(20)
          .px(8)
          .items_center()
          .rounded(10)
          .border_1()
          .border_color(t.border)
          .bg(manager ? t.selected : t.raised)
          .text_color(t.foreground)
          .cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => this.flip(cx))
          .child(`Lead · ${manager ? "Manager" : "Opus"}`),
      )
      .child(
        h_flex()
          .id("lead-door")
          .h(20)
          .px(7)
          .items_center()
          .rounded(7)
          .border_1()
          .border_color(t.border)
          .bg(t.raised)
          .text_color(t.muted)
          .cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_click(() => this.door())
          .child("Manager ↗"),
      );
  }

  renderHandoff(t) {
    const p = props() ?? {};
    const sendLabel = p.send === "opus" ? "Send to Opus" : "Send to Manager";
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
          .child(div().text_size(13).font_semibold().child(p.title ?? "The lead"))
          .child(
            h_flex()
              .id("lead-close")
              .w(22)
              .h(22)
              .items_center()
              .justify_center()
              .rounded(6)
              .text_color(t.dim)
              .cursor_pointer()
              .hover((s) => s.bg(t.hover))
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
              .text_size(12.5)
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
          .child(this.button("lead-cancel", "Cancel", false, () => closeOverlay(p.overlay), t))
          .child(
            this.button(
              "lead-send",
              this.sending ? "Sending…" : sendLabel,
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
