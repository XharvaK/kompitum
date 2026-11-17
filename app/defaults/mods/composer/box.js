// The one box under the conversation: chips, attach, status, send and the first-start card.

import { div, svg, image } from "gpui-kit";
import { v_flex, h_flex, Textarea } from "gpui-base";
import { run, pickFiles, pasteImage, lead as leadOf, window, home, text } from "kompitum";
import { errorText, COMPACT_FIRST_TOKENS, PULSE_MS } from "./util.js";
import { effortIds, leadUsage, modelLabel } from "./model.js";

const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

function baseName(path) {
  return String(path ?? "").split(/[\\/]/).filter(Boolean).pop() ?? String(path ?? "");
}

const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp"]);

// Whether a chip's file is one this can show a thumbnail of, judging by its extension alone.
function isImage(path) {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  if (dot < 0) return false;
  return IMAGE_EXTENSIONS.has(name.slice(dot + 1).toLowerCase());
}

function leadName(entry, key) {
  return key === "claude" ? "Claude" : entry?.name ?? key;
}

// The box says only what the lead is doing now: working, or waiting on the Owner's answer. Idle says nothing.
function leadStatus(state) {
  if (state === "working") return text("composer.working");
  if (state === "asking") return text("composer.waiting");
  return null;
}

// Ten minutes since the lead last sent a hook or a status line (`heard_ms`, Unix ms): it may have lost its closing hook.
function leadQuiet(heard) {
  return typeof heard === "number" && Date.now() - heard > 600000;
}

function statusColor(t, state) {
  return state === "asking" ? t.working : t.accent;
}

export default {
  addChip(path) {
    const pathText = String(path ?? "");
    if (!pathText || this.chips.some((chip) => chip.path === pathText)) return;
    this.chips.push({ path: pathText });
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

  dictate(cx) {
    this.text.focus();
    try {
      run("dictation.start", {});
      this.error = null;
    } catch (error) {
      this.error = errorText(error);
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
      .text_size(13)
      .font_medium()
      .cursor_pointer()
      .relative()
      .keep_focus(true)
      .tab_stop(true)
      .focus_ring(t.focus, 7)
      .on_click(action)
      .child(div().absolute().inset_0().rounded(7).hover((s) => s.bg(t.hover)).active((s) => s.bg(t.selected)))
      .child(label);
  },

  statusWord(t) {
    // A working lead that has sent no hook or status line for ten minutes may have lost its closing hook.
    if (this.state === "working" && leadQuiet(leadOf()?.heard_ms)) {
      return div()
        .text_size(13)
        .text_color(t.muted)
        .tooltip(text("composer.quiet-tooltip"))
        .child(text("composer.working-question"));
    }
    const word = leadStatus(this.state);
    if (!word) return null;
    // Working pulses for its first seconds, then the word says it alone; asking is the amber that wants the
    // Owner; waiting is still.
    return div()
      .text_size(13)
      .text_color(statusColor(t, this.state))
      .child(word)
      .when(this.pulsing(), (el) => el.loop("opacity", { from: 0.45, to: 1, period: 1200 }));
  },

  // Whether the lead started working less than PULSE_MS ago. A pulse says work began; it never runs on (Norma: loops
  // stop), and the word carries the state after it.
  pulsing() {
    return this.state === "working" && this.workingSince > 0 && Date.now() - this.workingSince < PULSE_MS;
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
      .when(isImage(chip.path), (el) => el.child(image(chip.path).w(16).h(16).rounded(3).flex_none()))
      .child(div().text_size(13).text_color(t.muted).child(baseName(chip.path)))
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
          .keep_focus(true)
          .accessibility_label(text("composer.remove-attachment"))
          .on_click((_e, cx) => this.removeChip(index, cx))
          .child(
            h_flex()
              .w(14)
              .h(14)
              .items_center()
              .justify_center()
              .rounded(4)
              .text_size(12)
              .text_color(t.dim)
              .hover((style) => style.bg(t.hover).text_color(t.foreground)).active((style) => style.bg(t.selected))
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
        this.error = text("composer.file-picker-failed", { error: String(error?.message ?? error) });
      }
      cx.notify();
    });
  },

  // The lead's task list above the box: folded to a count and the work in progress, or open to up to
  // eight rows. Shown only while a row still has work in it.
  taskStrip(t, cx) {
    const tasks = leadOf()?.tasks;
    if (!Array.isArray(tasks) || !tasks.some((row) => row?.status !== "completed")) return null;
    const done = tasks.filter((row) => row?.status === "completed").length;
    const open = Boolean(this.tasksOpen);
    const current = tasks.find((row) => row?.status === "in_progress")
      ?? tasks.find((row) => row?.status !== "completed");
    const label = current?.active || current?.subject || "";
    const mark = (row) => {
      if (row?.status === "completed") return div().text_size(13).text_color(t.dim).child("✓");
      if (row?.status === "in_progress") return div().text_size(13).text_color(t.working).child("●");
      return div().text_size(13).text_color(t.dim).child("○");
    };
    return v_flex()
      .id("composer-tasks")
      .w_full()
      .flex_none()
      .rounded(8)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .when(open, (el) =>
        el.children(tasks.slice(0, 8).map((row) =>
          h_flex()
            .id(`composer-task-${row.id}`)
            .h(24)
            .px(10)
            .gap(8)
            .items_center()
            .child(mark(row))
            .child(
              div()
                .min_w(0)
                .truncate()
                .text_size(13)
                .text_color(row?.status === "completed" ? t.dim : t.foreground)
                .child(row?.subject ?? ""),
            ))))
      .child(
        h_flex()
          .id("composer-tasks-toggle")
          .h(28)
          .px(10)
          .gap(8)
          .items_center()
          .cursor_pointer()
          .keep_focus(true)
          .on_click((_e, cx) => {
            this.tasksOpen = !open;
            cx.notify();
          })
          .child(div().text_size(13).text_color(t.muted).child(text("composer.tasks-count", { done, total: tasks.length })))
          .child(div().flex_1().min_w(0).truncate().text_size(14).text_color(t.foreground).child(label))
          .child(div().text_size(13).text_color(t.dim).child("›")),
      );
  },

  // One floating box: files, the text (no placeholder), then attach and the mode on the left; the lead's status,
  // its model and Send on the right.
  // While the lead asks, its card takes the text's place in the same box, and the text is not drawn.
  // `panel`: under the conversation the box is as tall as its text (an ask's card scrolls past 360 px); in its own
  // pane (under a shown terminal, or the first pick) it fills the pane.
  bar(t, cx, panel = false) {
    const first = this.needsFirstPick();
    const compacting = leadOf()?.compacting === true;
    const pill = first ? null : this.compactFirstPill(t);
    const canSend = this.leadReady() && !this.sending && !compacting && !this.compactThenSend
      && (Boolean(this.text?.value?.()?.trim()) || this.chips.length > 0);
    // Send is the view's one prominent action: the accent fill once there is something to send, an outline until
    // then. Hover and press lay the theme's state tints over the fill, so it lightens in dark and darkens in light.
    const sendHint = !this.leadReady()
      ? text("composer.send-starting")
      : compacting ? text("composer.send-tidying") : this.sending ? text("composer.sending")
        : this.compactFirst ? text("composer.send-compact") : text("composer.send");
    const send = h_flex()
      .id("composer-send")
      .relative()
      .w(32)
      .h(32)
      .flex_none()
      .items_center()
      .justify_center()
      .rounded_full()
      .keep_focus(true)
      .tooltip(sendHint)
      .accessibility_label(sendHint)
      .when(canSend, (el) => el.bg(t.accent_fill).cursor_pointer().on_click((_e, cx) => this.send(cx))
        .child(div().id("composer-send-state").absolute().inset_0().rounded_full().hover((s) => s.bg(t.hover)).active((s) => s.bg(t.selected))))
      // Nothing to send: an outline and a dim arrow, with no hover or press (Norma's disabled state).
      .when(!canSend, (el) => el.border_1().border_color(t.input))
      .child(svg("mods/composer/arrow-up.svg").size(16).text_color(canSend ? t.accent_foreground : t.dim));
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
      .hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
      .keep_focus(true)
      .tooltip(text("composer.attach"))
      .accessibility_label(text("composer.attach"))
      .on_click((_e, cx) => this.attach(cx))
      .child(svg("mods/composer/paperclip.svg").size(16));
    // Voice: the system's own dictation for the focused box (Win+H on Windows, Dictation on macOS).
    // Its words wait in the box like typed ones.
    const voice = window()?.dictation && !first && !this.ask
      ? h_flex()
        .id("composer-voice")
        .w(28)
        .h(28)
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(14)
        .cursor_pointer()
        .text_color(t.muted)
        .hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
        .keep_focus(true)
        .tooltip(window()?.platform === "windows" ? text("composer.dictate-windows") : text("composer.dictate"))
        .accessibility_label(window()?.platform === "windows" ? text("composer.dictate-windows") : text("composer.dictate"))
        .on_click((_e, cx) => this.dictate(cx))
        .child(svg("mods/composer/mic.svg").size(16))
      : null;
    const note = this.error
      ? div().id("composer-error").min_w(0).truncate().text_size(13).text_color(t.blocked).tooltip(this.error).child(firstLine(this.error) || this.error)
      : this.compactThenSend
        ? div().text_size(13).text_color(t.dim).child(text("composer.tidying-then-sending"))
        : compacting
          ? div().text_size(13).text_color(t.dim).child(text("composer.tidying"))
          : this.state ? this.statusWord(t) : null;
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
          .when(!first && !this.ask && Boolean(this.menu), (el) => el.child(this.menuCard(t, cx, panel)))
          .when(!first && !this.ask && !this.menu, (el) =>
            el.child(div().when(!panel, (box) => box.flex_1().min_h(0)).child(Textarea.new(this.text).w_full().text_size(14))))
          .child(
            h_flex()
              .w_full()
              .gap(6)
              .items_center()
              // What goes into the message on the left (files); who answers and sending on the right:
              // the model and its reasoning, dictation and Send.
              .child(
                h_flex()
                  .gap(8)
                  .items_center()
                  .min_w(0)
                  .child(plus),
              )
              .child(div().flex_1())
              .when(Boolean(note), (el) => el.child(div().min_w(0).child(note)))
              .when(Boolean(pill), (el) => el.child(pill))
              .when(!first, (el) => el.child(this.modelChip(t)))
              .when(Boolean(voice), (el) => el.child(voice))
              .when(!first && !this.ask && !this.menu, (el) => el.child(send)),
          );
    // The `/` list is an absolute child of the box's own frame, so it can sit above the box
    // without taking the textarea's keyboard (a Popover would).
    const strip = this.taskStrip(t, cx);
    const boxFrame = div()
      .relative()
      .when(panel, (el) => el.w_full().max_w(760))
      .when(!panel, (el) => (strip ? el.flex_1().min_h(0) : el.size_full()))
      .child(box)
      .when(this.listShown(), (el) => el.child(this.listMenu(t, cx)));
    if (!strip) return frame.child(panel ? h_flex().w_full().justify_center().child(boxFrame) : boxFrame);
    const column = v_flex()
      .when(panel, (el) => el.w_full().max_w(760))
      .when(!panel, (el) => el.size_full())
      .gap(6)
      .child(strip)
      .child(boxFrame);
    return frame.child(panel ? h_flex().w_full().justify_center().child(column) : column);
  },

  // A new conversation: the headline in the middle of the space above, and the box at the bottom, where the
  // conversation keeps it, so the first item doesn't move it. The home folder is named as such (the top bar
  // says "~"), not by the account's name.
  startView(t, cx) {
    const entry = this.leadProgram();
    const trim = (path) => String(path ?? "").replace(/[\\/]+$/, "");
    const folder = trim(this.person?.lead?.folder);
    const parts = folder.split(/[\\/]/).filter(Boolean);
    const atHome = parts.length === 0 || folder.toLowerCase() === trim(home()).toLowerCase();
    const place = atHome ? text("composer.your-home-folder") : parts[parts.length - 1];
    return v_flex()
      .id("composer-start")
      .size_full()
      .min_h(0)
      .child(
        v_flex()
          .flex_1()
          .min_h(0)
          .w_full()
          .items_center()
          .justify_center()
          .child(
            div()
              .id("composer-start-title")
              .text_size(22)
              .font_semibold()
              .text_color(t.foreground)
              .child(text("composer.whats-next")),
          )
          .child(
            div()
              .id("composer-start-where")
              .mt(6)
              .text_size(14)
              .text_color(t.muted)
              .child(text("composer.start-where", { lead: leadName(entry, entry.key), place })),
          ),
      )
      .child(this.bar(t, cx, true));
  },

  // Whether Shift+Tab may press the key: the lead has named a mode, waits, with no card and no tidy-up.
  modeLive() {
    const mode = leadOf()?.permission_mode;
    return typeof mode === "string" && Boolean(mode) && this.leadReady() && !this.ask && leadOf()?.compacting !== true;
  },

  // Shift+Tab in the box: the one key Claude Code takes for the next mode.
  pressMode(cx) {
    if (!this.modeLive()) return;
    run("lead.mode", { terminal: this.terminal });
    cx.notify();
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
      .text_size(13)
      .text_color(on ? t.accent : t.muted)
      .cursor_pointer()
      .keep_focus(true)
      .when(!on, (el) => el.hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected)))
      .tooltip(on
        ? text("composer.compact-on-tooltip", { k })
        : text("composer.compact-off-tooltip", { k }))
      .on_click((_e, cx) => {
        this.compactFirst = !this.compactFirst;
        cx.notify();
      })
      .child(text("composer.compact-first"));
  },

  startLabel() {
    const lead = this.person?.lead ?? {};
    const entry = this.leadProgram();
    const name = leadName(entry, entry.key);
    return lead.model
      ? text("composer.start-lead-model", { lead: name, model: modelLabel(entry, lead.model) })
      : text("composer.start-lead-name", { lead: name });
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
      .relative()
      .text_size(14)
      .font_medium();
    // No model picked yet: the fill goes and an outline keeps the shape (Norma's disabled state).
    const button = picked
      ? start.bg(t.accent_fill).text_color(t.accent_foreground).cursor_pointer().keep_focus(true).on_click((_e, cx) => this.startFirstPick(cx))
        .child(div().absolute().inset_0().rounded(8).hover((s) => s.bg(t.hover)).active((s) => s.bg(t.selected))).child(text("composer.start-the-lead"))
      : start.border_1().border_color(t.input).text_color(t.dim).child(text("composer.start-the-lead"));
    return v_flex()
      .id("composer-first")
      .flex_1()
      .min_h(0)
      .gap(6)
      .child(div().text_size(16).font_semibold().text_color(t.foreground).child(text("composer.which-model")))
      .child(div().text_size(13).text_color(t.dim).child(text("composer.model-change-hint")))
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
          .keep_focus(true)
          .on_click((_e, cx) => {
            this.firstModel = model.id;
            cx.notify();
          })
          .child(div().text_size(14).font_medium().text_color(t.foreground).child(model.label ?? model.id))
          .when(Boolean(model.hint), (el) =>
            el.child(div().text_size(13).text_color(t.dim).child(model.hint)))))
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
      .text_size(13)
      .text_color(t.muted)
      .child(text("composer.starting", { lead: leadName(entry, entry.key) }));
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
      .child(div().text_size(13).text_color(t.muted).child(text("composer.lead-not-running")))
      .child(this.filledButton("composer-open", this.startLabel(), () => run("lead.open"), t));
  },
};
