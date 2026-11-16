// The composer's cards (permission, plan and question): their keys, answers and parts.

import { div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, TextView } from "gpui-base";
import { run, settings, text } from "kompitum";
import { CARD_WAIT_MS } from "./util.js";
import { shortPath } from "./paths.js";

function planText(ask) {
  const plan = ask?.input?.tool_input?.plan;
  return typeof plan === "string" && plan.trim() ? plan : null;
}

function repeatsLabel(label, description) {
  const words = String(description ?? "").trim().toLowerCase().replace(/^(choose|pick|select)\s+/, "");
  return words === String(label ?? "").trim().toLowerCase();
}

function compactJson(value) {
  try {
    return JSON.stringify(value ?? {});
  } catch {
    return String(value ?? "");
  }
}

function cut(text, limit = 300) {
  const value = String(text ?? "");
  return value.length > limit ? `${value.slice(0, limit)}…` : value;
}

// The folder a card's paths read from: the ask's own cwd, else the lead's folder.
function askFolder(ask) {
  const cwd = ask?.input?.cwd;
  if (typeof cwd === "string" && cwd) return cwd;
  const folder = settings()?.lead?.folder;
  return typeof folder === "string" && folder ? folder : null;
}

// What Claude wants, from the tool and its input. `mono` marks the detail that
// reads best as the command itself.
function permissionSummary(ask) {
  const name = String(ask?.input?.tool_name ?? "");
  const input = ask?.input?.tool_input && typeof ask.input.tool_input === "object" ? ask.input.tool_input : {};
  const folder = askFolder(ask);
  if (name === "Bash") return { title: text("composer.run-command"), detail: String(input.command ?? ""), mono: true };
  if (name === "Edit" || name === "Write") return { title: text("composer.edit-path", { path: shortPath(String(input.file_path ?? ""), folder) }) };
  if (name === "Read") return { title: text("composer.read-path", { path: shortPath(String(input.file_path ?? ""), folder) }) };
  if (name === "WebFetch") return { title: text("composer.fetch-url", { url: String(input.url ?? "") }) };
  if (name === "WebSearch") return { title: text("composer.search-web", { query: String(input.query ?? "") }) };
  if (name.startsWith("mcp__")) {
    const rest = name.slice("mcp__".length);
    const at = rest.indexOf("__");
    if (at > 0) return { title: text("composer.mcp-tool", { tool: rest.slice(at + 2), server: rest.slice(0, at) }) };
  }
  return { title: name || text("composer.a-tool"), detail: cut(compactJson(input)) };
}

function permissionDestination(ask) {
  const suggestions = ask?.input?.permission_suggestions;
  const destination = Array.isArray(suggestions) ? suggestions[0]?.destination : null;
  if (destination === "session") return text("composer.for-session");
  if (destination === "localSettings" || destination === "projectSettings") return text("composer.in-project");
  if (destination === "userSettings") return text("composer.for-all-projects");
  return null;
}

function allowAlwaysLabel(ask) {
  const words = permissionDestination(ask);
  return words ? text("composer.allow-always-destination", { destination: words }) : text("composer.allow-always");
}

function askQuestions(ask) {
  const questions = ask?.input?.tool_input?.questions;
  return Array.isArray(questions) ? questions : [];
}

// A preview is a tool result's kind of text: at most sixteen lines, then an ellipsis row.
function previewText(text) {
  const lines = String(text ?? "").split(/\r?\n/);
  const shown = lines.slice(0, 16);
  if (lines.length > 16) shown.push("…");
  return shown.join("\n");
}

function pickedPreview(question, picked) {
  if (question?.multiSelect) return null;
  const label = typeof picked === "string" ? picked : null;
  if (!label) return null;
  const option = (question.options ?? []).find((item) => String(item.label ?? "") === label);
  const preview = option?.preview;
  return typeof preview === "string" && preview ? preview : null;
}

export default {
  cardEscape(ask, cx) {
    if (this.noteOpen) {
      this.closeNote(cx);
      return;
    }
    this.answer(ask, ask.kind === "plan" ? { action: "keep_planning" } : { action: "deny" }, cx);
  },

  cardKey(event, cx) {
    const ask = this.ask;
    if (this.noteOpen) {
      if (event.keystroke === "escape") {
        this.cardEscape(ask, cx);
        cx.stop_propagation();
      }
      return;
    }
    // A key held back by the wait or by a draft is still stopped here, so it reaches nothing else.
    if (ask.kind === "question") {
      if (/^[1-9]$/.test(event.keystroke)) {
        if (Date.now() - this.askSince >= CARD_WAIT_MS) this.pickOption(ask, Number(event.keystroke) - 1, cx);
        cx.stop_propagation();
      } else if (event.keystroke === "space") {
        const chosen = this.cardOptions(ask)[this.optionAt];
        if (chosen && Date.now() - this.askSince >= CARD_WAIT_MS) this.chooseOption(chosen.question, chosen.label, cx);
        cx.stop_propagation();
      } else if (event.keystroke === "enter" && this.allAnswered(ask)) {
        if (this.enterArmed()) this.sendAnswers(ask, cx);
        cx.stop_propagation();
      }
      return;
    }
    if (event.keystroke === "enter") {
      if (this.enterArmed()) this.answer(ask, ask.kind === "plan" ? { action: "approve" } : { action: "allow" }, cx);
      cx.stop_propagation();
    } else if (event.keystroke === "escape") {
      this.cardEscape(ask, cx);
      cx.stop_propagation();
    }
  },

  answer(ask, answer, cx) {
    if (!ask) return;
    this.noteOpen = false;
    this.noteInput = null;
    run("lead.answer", { id: ask.id, answer });
    // The card is going: the box is home again.
    if (this.text) this.text.focus();
    cx.notify();
  },

  openNote(action, cx) {
    this.noteAction = action;
    this.noteInput = InputState.new({
      placeholder: action === "deny" ? text("composer.note-deny") : text("composer.note-change"),
    });
    this.noteInput.on("submit", (_event, cx) => this.sendNote(cx));
    this.noteOpen = true;
    this.noteInput.focus();
    cx.notify();
  },

  closeNote(cx) {
    this.noteOpen = false;
    this.noteInput = null;
    this.noteAction = null;
    this.handle.focus();
    cx.notify();
  },

  sendNote(cx) {
    if (!this.ask || !this.noteInput) return;
    this.answer(this.ask, { action: this.noteAction, note: this.noteInput.value() }, cx);
  },

  // The text of a question's "Something else…" field, trimmed.
  questionText(question) {
    return String(this.answerTexts?.[question.question] ?? "").trim();
  },

  // The answer a question sends: its pick, or the field's text; in a multi-choice, the text joins the picks.
  answerFor(question) {
    const picked = this.answers[question.question];
    const written = this.questionText(question);
    if (question.multiSelect) {
      const labels = Array.isArray(picked) ? picked : [];
      return written ? [...labels, written] : labels;
    }
    if (typeof picked === "string" && picked) return picked;
    return written;
  },

  questionAnswered(question) {
    const answer = this.answerFor(question);
    if (Array.isArray(answer)) return answer.length > 0;
    return typeof answer === "string" && answer.length > 0;
  },

  allAnswered(ask) {
    const questions = askQuestions(ask);
    return questions.length > 0 && questions.every((question) => this.questionAnswered(question));
  },

  // One input per question, made when the ask is taken (never while rendering), released when it ends.
  syncQuestionInputs(ask, cx) {
    const id = ask?.kind === "question" ? ask.id : null;
    if (id === this.questionAsk) return;
    for (const input of Object.values(this.questionInputs ?? {})) {
      if (input && typeof input.release === "function") input.release();
    }
    this.questionInputs = {};
    this.answerTexts = {};
    this.questionAsk = id;
    if (id) {
      for (const question of askQuestions(ask)) {
        const input = InputState.new({ placeholder: text("composer.something-else") });
        input.on("change", (_event, cx) => {
          this.answerTexts[question.question] = input.value();
          // One choice: the field and the pick are alternatives.
          if (!question.multiSelect) delete this.answers[question.question];
          cx.notify();
        });
        this.questionInputs[question.question] = input;
      }
    }
    if (cx) cx.notify();
  },

  clearQuestionText(question) {
    this.answerTexts[question.question] = "";
    const input = this.questionInputs?.[question.question];
    if (input) input.set_value("");
  },

  chooseOption(question, label, cx) {
    if (question.multiSelect) {
      const chosen = Array.isArray(this.answers[question.question]) ? this.answers[question.question] : [];
      this.answers[question.question] = chosen.includes(label)
        ? chosen.filter((item) => item !== label)
        : [...chosen, label];
    } else {
      // Clearing the field fires its change first, then the pick is the answer.
      this.clearQuestionText(question);
      this.answers[question.question] = label;
    }
    cx.notify();
  },

  pickOption(ask, index, cx) {
    const question = askQuestions(ask).find((item) => !this.questionAnswered(item));
    if (!question) return;
    const option = question.options?.[index];
    if (!option) return;
    const label = String(option.label ?? "");
    if (!label) return;
    this.chooseOption(question, label, cx);
  },

  // Every option of the open card, in the order the card draws them: what the
  // arrows move through and Space picks.
  cardOptions(ask) {
    return askQuestions(ask).flatMap((question) =>
      (question.options ?? [])
        .map((option) => ({ question, label: String(option.label ?? "") }))
        .filter((option) => option.label));
  },

  cardArrow(direction, cx) {
    if (this.ask?.kind !== "question" || this.noteOpen) return;
    const list = this.cardOptions(this.ask);
    if (list.length === 0) return;
    const at = Math.max(0, Math.min(list.length - 1, Number.isInteger(this.optionAt) ? this.optionAt : 0));
    this.optionAt = (at + direction + list.length) % list.length;
    cx.notify();
  },

  toggleOption(question, label, cx) {
    this.chooseOption(question, label, cx);
  },

  sendAnswers(ask, cx) {
    if (!this.allAnswered(ask)) return;
    const answers = {};
    for (const question of askQuestions(ask)) {
      const answer = this.answerFor(question);
      if (Array.isArray(answer) ? answer.length > 0 : Boolean(answer)) {
        answers[question.question] = answer;
      }
    }
    this.answer(ask, { action: "answers", answers }, cx);
  },

  answerInTerminal(ask, cx) {
    this.answer(ask, { action: "terminal" }, cx);
  },

  cardTag(t, label) {
    return div()
      .px(5)
      .h(16)
      .flex_none()
      .flex()
      .items_center()
      .rounded(4)
      .bg(t.muted + "1f")
      .text_size(12)
      .font_medium()
      .text_color(t.muted)
      .child(label);
  },

  textButton(id, label, action, t) {
    return h_flex()
      .id(id)
      .h(28)
      .px(8)
      .items_center()
      .rounded(6)
      .text_size(13)
      .text_color(t.muted)
      .cursor_pointer()
      .hover((style) => style.bg(t.hover).text_color(t.foreground)).active((style) => style.bg(t.selected))
      .keep_focus(true)
      .tab_stop(true)
      .focus_ring(t.focus, 6)
      .on_click(action)
      .child(label);
  },

  cardButton(id, label, action, t, { filled = false, tooltip = null } = {}) {
    const button = filled ? this.filledButton(id, label, action, t) : h_flex()
      .id(id)
      .h(28)
      .px(12)
      .items_center()
      .rounded(7)
      .border_1()
      .border_color(t.input)
      .bg(t.raised)
      .text_color(t.foreground)
      .text_size(13)
      .font_medium()
      .cursor_pointer()
      .hover((style) => style.bg(t.hover).text_color(t.foreground)).active((style) => style.bg(t.selected))
      .keep_focus(true)
      .tab_stop(true)
      .focus_ring(t.focus, 7)
      .on_click(action)
      .child(label);
    return tooltip ? button.tooltip(tooltip) : button;
  },

  noteField(t, cx) {
    if (!this.noteOpen || !this.noteInput) return null;
    return h_flex()
      .w_full()
      .gap(6)
      .items_center()
      .child(
        div()
          .flex_1()
          .min_w(0)
          .h(28)
          .px(8)
          .flex()
          .items_center()
          .rounded(7)
          .border_1()
          .border_color(t.input)
          .bg(t.surface)
          .child(Input.new(this.noteInput).w_full().text_size(13).text_color(t.foreground)),
      )
      .child(this.cardButton("composer-note-cancel", text("composer.cancel"), (_e, cx) => this.closeNote(cx), t))
      .child(this.cardButton("composer-note-send", text("composer.send-label"), (_e, cx) => this.sendNote(cx), t, { filled: true }));
  },

  // The line under a card's buttons: what Enter and Esc do, or a note that the box holds a draft.
  cardHint(t, idle, typing) {
    return div().text_size(12).text_color(t.dim).child(this.hasDraft() ? typing : idle);
  },

  permissionCard(t, cx, ask) {
    const summary = permissionSummary(ask);
    const suggestions = Array.isArray(ask.input?.permission_suggestions) ? ask.input.permission_suggestions : [];
    const head = v_flex()
      .w_full()
      .gap(4)
      .child(
        h_flex()
          .gap(6)
          .items_center()
          .child(this.cardTag(t, text("composer.tag-permission")))
          .child(div().text_size(14).font_medium().text_color(t.foreground).child(summary.title)),
      );
    if (summary.detail) {
      head.child(
        div()
          .text_size(13)
          .text_color(t.muted)
          .when(Boolean(summary.mono), (el) => el.font_family(settings()?.font_family ?? "Google Sans Code"))
          .child(summary.detail),
      );
    }
    if (ask.input?.cwd) head.child(div().text_size(13).text_color(t.dim).child(text("composer.runs-in", { folder: ask.input.cwd })));
    const buttons = h_flex().w_full().gap(8).items_center()
      .child(this.cardButton("composer-allow", text("composer.allow"), (_e, cx) => this.answer(ask, { action: "allow" }, cx), t, { filled: true }))
      .when(suggestions.length > 0, (el) => el.child(
        this.cardButton(
          "composer-allow-always",
          allowAlwaysLabel(ask),
          (_e, cx) => this.answer(ask, { action: "allow_always" }, cx),
          t,
          { tooltip: permissionDestination(ask) ?? text("composer.where-rules") },
        ),
      ))
      .child(this.cardButton("composer-deny", text("composer.deny"), (_e, cx) => this.openNote("deny", cx), t));
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.border).bg(t.surface)
      .child(head)
      .when(this.noteOpen, (el) => el.child(this.noteField(t, cx)))
      .when(!this.noteOpen, (el) => el.child(buttons))
      .when(!this.noteOpen, (el) => el.child(this.cardHint(
        t,
        text("composer.permission-hint"),
        text("composer.permission-draft-hint"),
      )))
      .child(this.terminalAnswerLink(ask, t, cx));
  },

  planCard(t, cx, ask) {
    const plan = planText(ask);
    const buttons = h_flex().w_full().gap(6).items_center()
      .child(this.cardButton(
        "composer-approve",
        text("composer.approve-accept-edits"),
        (_e, cx) => this.answer(ask, { action: "approve", mode: "acceptEdits" }, cx),
        t,
        { filled: true },
      ))
      .child(this.cardButton(
        "composer-approve-ask",
        text("composer.approve-ask-first"),
        (_e, cx) => this.answer(ask, { action: "approve", mode: "default" }, cx),
        t,
      ))
      .child(this.cardButton("composer-keep-planning", text("composer.change-plan"), (_e, cx) => this.openNote("keep_planning", cx), t));
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.border).bg(t.surface)
      .child(
        h_flex()
          .gap(6)
          .items_center()
          .child(this.cardTag(t, text("composer.tag-plan")))
          .child(div().text_size(14).font_medium().text_color(t.foreground).child(text("composer.plan-ready"))),
      )
      .when(Boolean(plan), (el) => el.child(
        div()
          .w_full()
          .max_h(240)
          .overflow_y_scrollbar()
          .child(TextView.markdown("composer-plan-text", plan).text_size(13).text_color(t.muted).selectable(true)),
      ))
      .when(this.noteOpen, (el) => el.child(this.noteField(t, cx)))
      .when(!this.noteOpen, (el) => el.child(buttons))
      .when(!this.noteOpen, (el) => el.child(this.cardHint(
        t,
        text("composer.plan-hint"),
        text("composer.plan-draft-hint"),
      )))
      .child(this.terminalAnswerLink(ask, t, cx));
  },

  questionCard(t, cx, ask) {
    const questions = askQuestions(ask);
    let optionAt = 0;
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.border).bg(t.surface)
      .child(h_flex().child(this.cardTag(t, text("composer.tag-question"))))
      .children(questions.map((question, questionIndex) => {
        const answer = this.answers[question.question];
        const chosen = Array.isArray(answer) ? answer : typeof answer === "string" ? [answer] : [];
        const input = this.questionInputs?.[question.question] ?? null;
        const preview = pickedPreview(question, this.answers[question.question]);
        return v_flex()
          .w_full()
          .gap(4)
          .role(question.multiSelect ? "group" : "radio_group")
          .child(div().text_size(13).font_medium().text_color(t.foreground).child(question.header || question.question))
          .when(Boolean(question.header), (el) => el.child(div().text_size(13).text_color(t.muted).child(question.question)))
          .children((question.options ?? []).map((option, optionIndex) => {
            const label = String(option.label ?? "");
            const picked = chosen.includes(label);
            const at = optionAt++;
            return h_flex()
              .id(`composer-option-${questionIndex}-${optionIndex}`)
              .w_full()
              .gap(6)
              .items_center()
              .px(8)
              .py(5)
              .rounded(7)
              .border_1()
              .border_color(picked || this.optionAt === at ? t.accent : t.input)
              .bg(picked ? t.selected : t.raised)
              .cursor_pointer()
              .role(question.multiSelect ? "check_box" : "radio_button")
              .aria_selected(picked)
              .hover((style) => style.bg(t.hover).text_color(t.foreground)).active((style) => style.bg(t.selected))
              .on_click((_e, cx) => {
                this.optionAt = at;
                this.toggleOption(question, label, cx);
              })
              .child(div().text_size(13).font_medium().text_color(t.foreground)
                .child(optionIndex < 9 ? text("composer.option-numbered", { number: optionIndex + 1, label }) : label))
              .when(
                Boolean(option.description) && !repeatsLabel(label, option.description),
                (el) => el.child(div().text_size(13).text_color(t.dim).child(option.description)),
              );
          }))
          .when(Boolean(input), (el) => el.child(
            div()
              .id(`composer-other-${questionIndex}`)
              .w_full()
              .h(28)
              .px(8)
              .flex()
              .items_center()
              .rounded(7)
              .border_1()
              .border_color(t.input)
              .bg(t.surface)
              .child(Input.new(input).w_full().text_size(13).text_color(t.foreground)),
          ))
          .when(Boolean(preview), (el) => el.child(
            div()
              .id(`composer-preview-${questionIndex}`)
              .w_full()
              .p(8)
              .rounded(8)
              .border_1()
              .border_color(t.border)
              .text_size(13)
              .font_family(settings()?.font_family ?? "Google Sans Code")
              .text_color(t.dim)
              .child(previewText(preview)),
          ));
      }))
      .child(
        h_flex().w_full().gap(6).items_center()
          .child(this.sendAnswersButton(ask, t, cx))
          .child(div().flex_1())
          .child(this.terminalAnswerLink(ask, t, cx)),
      );
  },

  sendAnswersButton(ask, t, cx) {
    const enabled = this.allAnswered(ask);
    const button = h_flex()
      .id("composer-send-answers")
      .h(28)
      .px(12)
      .items_center()
      .rounded(7)
      .text_size(13)
      .font_medium();
    // Not every question answered: the fill goes and an outline keeps the shape (UD's disabled state).
    if (!enabled) return button.border_1().border_color(t.input).text_color(t.dim).child(text("composer.send-answers"));
    return button
      .relative()
      .bg(t.accent_fill)
      .text_color(t.accent_foreground)
      .cursor_pointer()
      .keep_focus(true)
      .tab_stop(true)
      .focus_ring(t.focus, 7)
      .on_click((_e, cx) => this.sendAnswers(ask, cx))
      .child(div().absolute().inset_0().rounded(7).hover((s) => s.bg(t.hover)).active((s) => s.bg(t.selected)))
      .child(text("composer.send-answers"));
  },

  terminalAnswerLink(ask, t, cx) {
    return h_flex()
      .id("composer-terminal-answer")
      .items_center()
      .cursor_pointer()
      .text_size(13)
      .text_color(t.dim)
      .keep_focus(true)
      .tab_stop(true)
      .focus_ring(t.focus, 0)
      .hover((style) => style.text_color(t.foreground))
      .on_click((_e, cx) => this.answerInTerminal(ask, cx))
      .child(text("composer.answer-in-terminal"));
  },

  // The menu Claude shows in its terminal, drawn where an ask card goes. Its rows are a single choice list;
  // the buttons are the menu's own actions, with the fixed link out to the terminal.
  // The card's words and rows scroll inside it; its buttons stay at its foot, always in view.
  // Under a conversation the words and rows take at most 420 px, as the ask cards' do: the frame there has no set
  // height, so a part that may shrink would be squeezed to nothing. Alone in the tab, the card fills the room there.
  menuCard(t, cx, panel) {
    const menu = this.menu;
    const mono = settings()?.font_family ?? "Google Sans Code";
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.input).bg(t.surface)
      .when(!panel, (card) => card.flex_1().min_h(0))
      .child(v_flex().id("composer-asks").w_full().gap(8)
        .when(panel, (asks) => asks.max_h(420))
        .when(!panel, (asks) => asks.flex_shrink(1).min_h(0))
        .overflow_y_scrollbar()
        .child(div().text_size(13).text_color(t.muted).child(text("composer.menu-eyebrow")))
        .child(div().text_size(15).font_medium().text_color(t.foreground).child(String(menu?.title ?? "")))
        .children((menu?.body ?? []).map((block) => this.menuBlock(t, block, mono, 14)))
        .children((menu?.rows ?? []).map((row, index) => this.menuRow(t, row, index, cx)))
        .children((menu?.notes ?? []).map((block) => this.menuBlock(t, block, mono, 13))))
      .child(div().flex_none().child(this.menuButtons(t, cx)));
  },

  menuBlock(t, block, mono, size) {
    if (block?.mono) {
      return div()
        .w_full()
        .max_h(240)
        .overflow_y_scrollbar()
        .child(div().text_size(13).font_family(mono).text_color(t.dim).child(String(block.text ?? "")));
    }
    return div().w_full().text_size(size).text_color(t.muted).child(String(block?.text ?? ""));
  },

  menuRow(t, row, index, cx) {
    const on = this.menuAt === index;
    const label = String(row?.label ?? "");
    return h_flex()
      .id(`composer-menu-row-${index}`)
      .w_full()
      .gap(8)
      .items_center()
      .px(8)
      .py(5)
      .rounded(7)
      .border_1()
      .border_color(on ? t.accent : t.input)
      .bg(on ? t.selected : t.raised)
      .cursor_pointer()
      .role("radio_button")
      .aria_selected(on)
      .on_click((event, cx) => {
        if (this.menuBusy) return;
        this.menuAt = index;
        if (event?.click_count >= 2) this.menuAction("enter", cx);
        else cx.notify();
      })
      .child(div().w(18).flex_none().text_size(14).text_color(t.muted).child(index < 9 ? `${index + 1}.` : ""))
      .child(div().min_w(0).text_size(14).text_color(t.foreground).child(label))
      .when(Boolean(row?.current), (el) => el.child(svg("mods/workspace-chrome/check.svg").size(13).text_color(t.muted)))
      .when(Boolean(row?.detail), (el) =>
        el.child(div().min_w(0).truncate().text_size(13).text_color(t.muted).child(String(row.detail))));
  },

  menuButtons(t, cx) {
    const actions = this.menu?.actions ?? [];
    const enter = actions.find((action) => action.key === "enter");
    const esc = actions.find((action) => action.key === "esc");
    const letters = actions.filter((action) => /^[a-z0-9]$/.test(String(action.key ?? "")));
    const buttons = h_flex().w_full().gap(6).items_center().when(this.menuBusy, (el) => el.opacity(0.5));
    if (enter) {
      buttons.child(this.cardButton(
        "composer-menu-enter",
        enter.label || text("composer.menu-choose"),
        (_e, cx) => this.menuAction(enter.key, cx),
        t,
        { filled: true },
      ));
    }
    for (const action of letters) {
      buttons.child(this.cardButton(
        `composer-menu-${action.key}`,
        String(action.label ?? ""),
        (_e, cx) => this.menuAction(action.key, cx),
        t,
      ));
    }
    if (esc) {
      buttons.child(this.cardButton(
        "composer-menu-esc",
        String(esc.label ?? ""),
        (_e, cx) => this.menuAction(esc.key, cx),
        t,
      ));
    }
    buttons.child(div().flex_1());
    buttons.child(this.textButton(
      "composer-menu-show-terminal",
      text("composer.menu-show-terminal"),
      () => run("lead.terminal", { terminal: this.terminal, show: true }),
      t,
    ));
    return buttons;
  },

  menuAction(key, cx) {
    if (this.menuBusy) return;
    const action = this.menuActionFor(key);
    if (!action) return;
    const args = { terminal: this.terminal, key: action.key };
    if (action.key !== "esc") {
      const row = (this.menu?.rows ?? [])[this.menuAt];
      if (row) {
        args.row = this.menuAt;
        args.label = String(row.label ?? "");
      }
    }
    run("lead.menu", args);
    cx.notify();
  },

  menuActionFor(key) {
    return (this.menu?.actions ?? []).find((action) => action.key === key) ?? null;
  },

  menuHighlight(index, cx) {
    if (this.menuBusy) return;
    const rows = this.menu?.rows ?? [];
    if (!rows[index]) return;
    this.menuAt = index;
    cx.notify();
  },

  menuArrow(direction, cx) {
    if (this.menuBusy) return;
    const rows = this.menu?.rows ?? [];
    if (rows.length === 0) return;
    const at = Math.max(0, Math.min(rows.length - 1, Number.isInteger(this.menuAt) ? this.menuAt : 0));
    this.menuAt = (at + direction + rows.length) % rows.length;
    cx.notify();
  },

  menuKey(event, cx) {
    if (!this.menu) return;
    const keystroke = event?.keystroke;
    if (keystroke === "enter") {
      if (Date.now() - this.menuSince >= CARD_WAIT_MS) this.menuAction("enter", cx);
      cx.stop_propagation();
    } else if (keystroke === "escape") {
      this.menuAction("esc", cx);
      cx.stop_propagation();
    } else if (/^[1-9]$/.test(keystroke)) {
      const at = Number(keystroke) - 1;
      if ((this.menu.rows ?? [])[at]) this.menuHighlight(at, cx);
      cx.stop_propagation();
    } else if (keystroke === "space") {
      this.menuHighlight(this.menuAt, cx);
      cx.stop_propagation();
    }
  },

  menuEscape(cx) {
    if (this.menuBusy) return;
    this.menuAction("esc", cx);
  },

  card(t, cx) {
    const card = this.ask.kind === "plan"
      ? this.planCard(t, cx, this.ask)
      : this.ask.kind === "question"
        ? this.questionCard(t, cx, this.ask)
        : this.permissionCard(t, cx, this.ask);
    if (!this.hasDraft()) return card;
    return v_flex()
      .w_full()
      .gap(6)
      .child(div().id("composer-draft-kept").text_size(13).text_color(t.dim).child(text("composer.draft-kept")))
      .child(card);
  },
};
