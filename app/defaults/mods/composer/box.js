// The one box under the conversation: chips, attach, status, send and the first-start card.

import { div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea } from "gpui-base";
import { run, pickFiles, pasteImage, lead as leadOf } from "kompitum";
import { errorText, hairline, COMPACT_FIRST_TOKENS } from "./util.js";
import { effortIds, leadUsage, modelLabel } from "./model.js";

const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

function baseName(path) {
  return String(path ?? "").split(/[\\/]/).filter(Boolean).pop() ?? String(path ?? "");
}

function leadName(entry, key) {
  return key === "claude" ? "Claude" : entry?.name ?? key;
}

// The box says only what the lead is doing now: working, or waiting on the Owner's answer. Idle says nothing.
function leadStatus(state) {
  if (state === "working") return "Working";
  if (state === "asking") return "Waiting for you";
  return null;
}

// Ten minutes since the lead last sent a hook or a status line (`heard_ms`, Unix ms): it may have lost its closing hook.
function leadQuiet(heard) {
  return typeof heard === "number" && Date.now() - heard > 600000;
}

function statusColor(t, state) {
  return state === "asking" ? t.working : t.accent;
}

// The share of the context the lead holds, from 0 up to 1 (capped); null when the count or the limit is unknown.
function contextFraction(usage) {
  const tokens = usage?.context_tokens;
  const max = usage?.compact_at;
  if (typeof tokens !== "number" || !Number.isFinite(tokens) || typeof max !== "number" || !(max > 0)) return null;
  return Math.min(1, Math.max(0, tokens / max));
}

// The two windows that count: a window is absent when its reset is in the past or its percentage is not a number.
function usageWindows(usage) {
  const now = Date.now() / 1000;
  return [
    { short: "5h", long: "5-hour limit", window: usage?.five_hour },
    { short: "week", long: "Week", window: usage?.seven_day },
  ]
    .map(({ short, long, window }) => ({ short, long, pct: window?.used_percentage, resets: window?.resets_at }))
    .filter((w) => typeof w.pct === "number" && Number.isFinite(w.pct))
    .filter((w) => !(typeof w.resets === "number" && w.resets < now));
}

// Claude Code's permission modes (its hooks' `permission_mode`): the word the box shows, and what the mode does.
const MODES = {
  default: { word: "Ask first", line: "Asks before it edits files or runs commands." },
  acceptEdits: { word: "Accept edits", line: "Edits files without asking; asks before commands." },
  plan: { word: "Plan first", line: "Plans and changes nothing until you approve." },
  auto: { word: "Auto", line: "Approves routine actions itself; asks about risky ones." },
  bypassPermissions: { word: "Full access", line: "Runs everything without asking." },
  dontAsk: { word: "Don't ask", line: "Runs only what is already allowed and refuses the rest." },
};

export default {
  addChip(path) {
    const text = String(path ?? "");
    if (!text || this.chips.some((chip) => chip.path === text)) return;
    this.chips.push({ path: text });
    this.syncDraft();
  },

  removeChip(index, cx) {
    this.chips.splice(index, 1);
    this.syncDraft();
    cx.notify();
  },

  pastedImage(cx) {
    let path = null;
    try {
      path = pasteImage();
    } catch (error) {
      this.error = errorText(error);
    }
    if (path) {
      this.addChip(path);
      this.text.focus();
    }
    cx.notify();
  },

  filledButton(id, label, action, t) {
    return h_flex()
      .id(id)
      .h(28)
      .px(12)
      .items_center()
      .rounded(7)
      .bg(t.accent_fill)
      .text_color(t.accent_foreground)
      .text_size(12.5)
      .font_medium()
      .cursor_pointer()
      .hover((style) => style.opacity(0.88))
      .on_click(action)
      .child(label);
  },

  statusWord(t) {
    // A working lead that has sent no hook or status line for ten minutes may have lost its closing hook.
    if (this.state === "working" && leadQuiet(leadOf()?.heard_ms)) {
      return div()
        .text_size(11.5)
        .text_color(t.muted)
        .tooltip("No word from the lead for 10 minutes.")
        .child("Working?");
    }
    const word = leadStatus(this.state);
    if (!word) return null;
    // Working pulses; asking is the amber that wants the Owner; waiting is still.
    return div()
      .text_size(11.5)
      .text_color(statusColor(t, this.state))
      .child(word)
      .when(this.state === "working", (el) => el.loop("opacity", { from: 0.35, to: 1, period: 900 }));
  },

  chip(t, cx, chip, index) {
    return h_flex()
      .id(`composer-chip-${index}`)
      .h(22)
      .px(7)
      .gap(5)
      .flex_none()
      .items_center()
      .rounded(6)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .child(div().text_size(11.5).text_color(t.muted).child(baseName(chip.path)))
      .child(
        h_flex()
          .id(`composer-chip-remove-${index}`)
          // A 28 px hit area with negative margins: it takes the 14 px the × took in the chip's row and height.
          .w(28)
          .h(28)
          .ml(-7)
          .mr(-7)
          .mt(-3)
          .mb(-3)
          .items_center()
          .justify_center()
          .cursor_pointer()
          .on_click((_e, cx) => this.removeChip(index, cx))
          .child(
            h_flex()
              .w(14)
              .h(14)
              .items_center()
              .justify_center()
              .rounded(4)
              .text_size(11)
              .text_color(t.dim)
              .hover((style) => style.bg(t.hover).text_color(t.foreground))
              .child("×"),
          ),
      );
  },

  // "+": the system's file picker; each file waits as a chip above the text.
  attach(cx) {
    cx.spawn(async (cx) => {
      try {
        for (const path of (await pickFiles()) ?? []) this.addChip(path);
      } catch (error) {
        this.error = `Couldn't open the file picker: ${String(error?.message ?? error)}`;
      }
      cx.notify();
    });
  },

  // One floating box: files, the text (no placeholder), then "+", the lead's status, its model and Send.
  // While the lead asks, its card takes the text's place in the same box, and the text is not drawn.
  // `panel`: under the conversation the box is as tall as its text (an ask's card scrolls past 360 px); in its own
  // pane (under a shown terminal, or the first pick) it fills the pane.
  bar(t, cx, panel = false) {
    const first = this.needsFirstPick();
    const compacting = leadOf()?.compacting === true;
    const mode = first ? null : this.modeControl(t);
    const pill = first ? null : this.compactFirstPill(t);
    const canSend = this.leadReady() && !this.sending && !compacting && !this.compactThenSend
      && (Boolean(this.text?.value?.()?.trim()) || this.chips.length > 0);
    const send = h_flex()
      .id("composer-send")
      .w(30)
      .h(30)
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(15)
      .cursor_pointer()
      .tooltip(!this.leadReady()
        ? "Claude is asking something before it starts."
        : compacting ? "Claude is tidying up. Your message waits in the box." : this.sending ? "Sending…"
          : this.compactFirst ? "Compact, then send (Enter)" : "Send (Enter)")
      .when(canSend, (el) => el.bg(t.foreground).hover((s) => s.opacity(0.85)))
      .when(!canSend, (el) => el.border_1().border_color(t.input))
      .on_click((_e, cx) => this.send(cx))
      .child(svg("mods/composer/arrow-up.svg").size(16).text_color(canSend ? t.background : t.dim));
    const plus = h_flex()
      .id("composer-attach")
      .w(28)
      .h(28)
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(14)
      .cursor_pointer()
      .text_color(t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      .tooltip("Attach files")
      .on_click((_e, cx) => this.attach(cx))
      .child(svg("mods/composer/paperclip.svg").size(16));
    const note = this.error
      ? div().id("composer-error").min_w(0).truncate().text_size(11.5).text_color(t.blocked).tooltip(this.error).child(firstLine(this.error) || this.error)
      : this.compactThenSend
        ? div().text_size(11.5).text_color(t.dim).child("Tidying up, then sending…")
        : compacting
          ? div().text_size(11.5).text_color(t.dim).child("Tidying up…")
          : this.state ? this.statusWord(t) : null;
    // The limit and the ring read the lead's usage; neither is drawn on the first-start card.
    const usage = first ? null : leadUsage();
    const fraction = contextFraction(usage);
    const windows = usage ? usageWindows(usage) : [];
    const limit = usage ? this.usageLimit(t, windows) : null;
    const ring = fraction == null ? null : this.usageControl(t, usage, fraction, windows);
    const frame = div()
      .when(panel, (el) => el.w_full().flex_none().px(20).py(10))
      .when(!panel, (el) => el.size_full().p(10))
      .bg(t.background);
    // Under the conversation the box shares its column: at most 760 px wide, centred.
    const box = v_flex()
          .id("composer-box")
          .when(panel, (el) => el.w_full().max_w(760))
          .when(!panel, (el) => el.size_full())
          .px(14)
          .pt(12)
          .pb(8)
          .gap(6)
          .rounded(20)
          .border_1()
          .border_color(this.ask ? (this.ask.kind === "permission" ? t.blocked : t.input) : t.border)
          .bg(t.raised)
          .when(this.chips.length > 0, (el) =>
            el.child(h_flex().gap(6).flex_wrap().children(this.chips.map((chip, index) => this.chip(t, cx, chip, index)))))
          .when(first, (el) => el.child(this.firstPickCard(t, cx)))
          .when(!first && Boolean(this.ask), (el) =>
            el.child(v_flex().id("composer-asks")
              .when(panel, (asks) => asks.max_h(360))
              .when(!panel, (asks) => asks.flex_1().min_h(0))
              .overflow_y_scrollbar().child(this.card(t, cx))))
          .when(!first && !this.ask, (el) =>
            el.child(div().when(!panel, (box) => box.flex_1().min_h(0)).child(Textarea.new(this.text).w_full().text_size(13.5).line_height("20px"))))
          .child(
            h_flex()
              .w_full()
              .gap(6)
              .items_center()
              .child(
                h_flex()
                  .gap(6)
                  .items_center()
                  .min_w(0)
                  .child(plus)
                  .when(!first, (el) => el.child(hairline(t)).child(this.modelChip(t)))
                  .when(Boolean(mode), (el) => el.child(hairline(t)).child(mode)),
              )
              .child(div().flex_1())
              .when(Boolean(note), (el) => el.child(div().min_w(0).child(note)))
              .when(Boolean(limit), (el) => el.child(limit))
              .when(Boolean(pill), (el) => el.child(pill))
              .when(Boolean(ring), (el) => el.child(ring))
              .when(!first && !this.ask, (el) => el.child(send)),
          );
    return frame.child(panel ? h_flex().w_full().justify_center().child(box) : box);
  },

  // A new conversation: the headline and the box sit a little above the middle. The first item moves the box down.
  // Two equal spacers centre the block; the 120 px under it lifts it a little above the middle.
  startView(t, cx) {
    const entry = this.leadProgram();
    const parts = String(this.person?.lead?.folder ?? "").split(/[\\/]/).filter(Boolean);
    const place = parts.length > 0 ? parts[parts.length - 1] : "your home folder";
    return v_flex()
      .id("composer-start")
      .size_full()
      .min_h(0)
      .items_center()
      .child(div().flex_1())
      .child(
        v_flex()
          .w_full()
          .items_center()
          .mb(120)
          .child(
            div()
              .id("composer-start-title")
              .text_size(26)
              .font_semibold()
              .text_color(t.foreground)
              .child("What's next?"),
          )
          .child(
            div()
              .id("composer-start-where")
              .mt(6)
              .text_size(13.5)
              .text_color(t.muted)
              .child(`${leadName(entry, entry.key)} in ${place}`),
          )
          .child(div().w_full().mt(18).child(this.bar(t, cx, true))),
      )
      .child(div().flex_1());
  },

  // The mode word after the model: live while the lead waits (a click presses Shift+Tab in its terminal), dim while
  // it works, asks or tidies up, and dim while a guessed mode waits for the next hook.
  modeControl(t) {
    const mode = leadOf()?.permission_mode;
    if (typeof mode !== "string" || !mode) return null;
    const { word, line = "" } = MODES[mode] ?? { word: mode };
    const guessed = leadOf()?.mode_guessed === true;
    const live = this.leadReady() && !this.ask && leadOf()?.compacting !== true;
    const tip = live ? [line, "Click for the next mode (Shift+Tab)."].filter(Boolean).join(" ") : line;
    return h_flex()
      .id("composer-mode")
      .h(28)
      .px(6)
      .items_center()
      .rounded(7)
      .text_size(12.5)
      .text_color(live && !guessed ? t.muted : t.dim)
      .when(live, (el) => el
        .cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_click((_e, cx) => {
          run("lead.mode", { terminal: this.terminal });
          cx.notify();
        }))
      .when(Boolean(tip), (el) => el.tooltip(tip))
      .child(word);
  },

  // "Compact first" beside Send, from COMPACT_FIRST_TOKENS of context, while a draft waits and the lead waits.
  // On: the next Send compacts the lead first (send does that); off: it keeps the context.
  compactFirstPill(t) {
    const tokens = leadUsage()?.context_tokens;
    if (!this.leadReady() || this.ask || this.state !== "waiting" || leadOf()?.compacting === true) return null;
    if (!this.hasDraft() || this.compactThenSend || !(tokens >= COMPACT_FIRST_TOKENS)) return null;
    const k = Math.round(tokens / 1000);
    const on = Boolean(this.compactFirst);
    return h_flex()
      .id("composer-compact-first")
      .h(24)
      .px(9)
      .flex_none()
      .items_center()
      .rounded(12)
      .border_1()
      .border_color(on ? t.accent : t.border)
      .text_size(11.5)
      .text_color(on ? t.accent : t.muted)
      .cursor_pointer()
      .when(!on, (el) => el.hover((s) => s.bg(t.hover)))
      .tooltip(on
        ? `Your next message compacts ${k}K tokens first. Click to keep them.`
        : `Your next message keeps all ${k}K tokens. Click to compact first.`)
      .on_click((_e, cx) => {
        this.compactFirst = !this.compactFirst;
        cx.notify();
      })
      .child("Compact first");
  },

  startLabel() {
    const lead = this.person?.lead ?? {};
    const entry = this.leadProgram();
    const model = lead.model ? ` (${modelLabel(entry, lead.model)})` : "";
    return `Start ${leadName(entry, entry.key)}${model}`;
  },

  firstPickCard(t, cx) {
    const entry = this.leadProgram();
    const models = entry?.models ?? [];
    const picked = this.firstModel;
    const start = h_flex()
      .id("composer-first-start")
      .h(32)
      .px(14)
      .items_center()
      .rounded(8)
      .bg(t.solid)
      .text_color(t.solid_foreground)
      .text_size(13)
      .font_medium()
      .child("Start the lead");
    const button = picked
      ? start.cursor_pointer().hover((s) => s.opacity(0.88)).on_click((_e, cx) => this.startFirstPick(cx))
      : start.opacity(0.45);
    return v_flex()
      .id("composer-first")
      .flex_1()
      .min_h(0)
      .gap(6)
      .child(div().text_size(14).font_semibold().text_color(t.foreground).child("Which model should the lead use?"))
      .child(div().text_size(12.5).text_color(t.dim).child("You can change it any time from the model menu below."))
      .children(models.map((model) =>
        v_flex()
          .id(`composer-first-${model.id}`)
          .h(44)
          .px(12)
          .gap(2)
          .justify_center()
          .rounded(10)
          .border_1()
          .border_color(picked === model.id ? t.accent : t.border)
          .when(picked === model.id, (el) => el.bg(t.selected))
          .cursor_pointer()
          .on_click((_e, cx) => {
            this.firstModel = model.id;
            cx.notify();
          })
          .child(div().text_size(13.5).font_medium().text_color(t.foreground).child(model.label ?? model.id))
          .when(Boolean(model.hint), (el) =>
            el.child(div().text_size(12).text_color(t.dim).child(model.hint)))))
      .when(effortIds(entry).length > 0, (el) => el.child(this.effortBody(t, entry, this.person?.lead ?? {})))
      .child(button);
  },

  // The lead asks something in its terminal before it starts (the trust prompt): the box can't answer that, so one row says so.
  startingRow(t) {
    const entry = this.leadProgram();
    return h_flex()
      .id("composer-starting")
      .w_full()
      .h(44)
      .items_center()
      .justify_center()
      .text_size(12.5)
      .text_color(t.muted)
      .child(`${leadName(entry, entry.key)} is asking something before it starts. Answer it above.`);
  },

  // No live lead: one quiet line and the way to start it, in the middle of whatever room it has.
  empty(t) {
    return h_flex()
      .id("composer-empty")
      .size_full()
      .px(12)
      .gap(12)
      .justify_center()
      .items_center()
      .border_t(1)
      .border_color(t.border)
      .bg(t.chrome)
      .child(div().text_size(12.5).text_color(t.muted).child("The lead isn't running"))
      .child(this.filledButton("composer-open", this.startLabel(), () => run("lead.open"), t));
  },
};
