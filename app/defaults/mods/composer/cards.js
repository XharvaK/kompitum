// The composer's cards (permission, plan and question): their keys, answers and parts.

import { div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, TextView } from "gpui-base";
import { run, settings } from "kompitum";

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

// What Claude wants, from the tool and its input. `mono` marks the detail that
// reads best as the command itself.
function permissionSummary(toolName, toolInput) {
  const name = String(toolName ?? "");
  const input = toolInput && typeof toolInput === "object" ? toolInput : {};
  if (name === "Bash") return { title: "Run a command", detail: String(input.command ?? ""), mono: true };
  if (name === "Edit" || name === "Write") return { title: `Edit ${String(input.file_path ?? "")}` };
  if (name === "Read") return { title: `Read ${String(input.file_path ?? "")}` };
  if (name === "WebFetch") return { title: `Fetch ${String(input.url ?? "")}` };
  if (name === "WebSearch") return { title: `Search the web for ${String(input.query ?? "")}` };
  if (name.startsWith("mcp__")) {
    const rest = name.slice("mcp__".length);
    const at = rest.indexOf("__");
    if (at > 0) return { title: `${rest.slice(at + 2)} on ${rest.slice(0, at)}` };
  }
  return { title: name || "A tool", detail: cut(compactJson(input)) };
}

const DESTINATIONS = {
  session: "for this session",
  localSettings: "in this project",
  projectSettings: "in this project",
  userSettings: "for all your projects",
};

function permissionDestination(ask) {
  const suggestions = ask?.input?.permission_suggestions;
  const destination = Array.isArray(suggestions) ? suggestions[0]?.destination : null;
  return DESTINATIONS[destination] ?? null;
}

function allowAlwaysLabel(ask) {
  const words = permissionDestination(ask);
  return words ? `Allow always ${words}` : "Allow always";
}

function askQuestions(ask) {
  const questions = ask?.input?.tool_input?.questions;
  return Array.isArray(questions) ? questions : [];
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
    if (ask.kind === "question") {
      if (/^[1-4]$/.test(event.keystroke)) {
        this.pickOption(ask, Number(event.keystroke) - 1, cx);
        cx.stop_propagation();
      } else if (event.keystroke === "enter" && this.allAnswered(ask)) {
        this.sendAnswers(ask, cx);
        cx.stop_propagation();
      }
      return;
    }
    if (event.keystroke === "enter") {
      this.answer(ask, ask.kind === "plan" ? { action: "approve" } : { action: "allow" }, cx);
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
    cx.notify();
  },

  openNote(action, cx) {
    this.noteAction = action;
    this.noteInput = InputState.new({
      placeholder: action === "deny" ? "Why not? (optional)" : "What should change? (optional)",
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

  questionAnswered(question) {
    const value = this.answers[question.question];
    if (Array.isArray(value)) return value.length > 0;
    return typeof value === "string" && value.length > 0;
  },

  allAnswered(ask) {
    const questions = askQuestions(ask);
    return questions.length > 0 && questions.every((question) => this.questionAnswered(question));
  },

  pickOption(ask, index, cx) {
    const question = askQuestions(ask).find((item) => !this.questionAnswered(item));
    if (!question) return;
    const option = question.options?.[index];
    if (!option) return;
    const label = String(option.label ?? "");
    if (!label) return;
    if (question.multiSelect) {
      const chosen = Array.isArray(this.answers[question.question]) ? this.answers[question.question] : [];
      this.answers[question.question] = chosen.includes(label)
        ? chosen.filter((item) => item !== label)
        : [...chosen, label];
    } else {
      this.answers[question.question] = label;
    }
    cx.notify();
  },

  toggleOption(question, label, cx) {
    if (question.multiSelect) {
      const chosen = Array.isArray(this.answers[question.question]) ? this.answers[question.question] : [];
      this.answers[question.question] = chosen.includes(label)
        ? chosen.filter((item) => item !== label)
        : [...chosen, label];
    } else {
      this.answers[question.question] = label;
    }
    cx.notify();
  },

  sendAnswers(ask, cx) {
    if (!this.allAnswered(ask)) return;
    this.answer(ask, { action: "answers", answers: this.answers }, cx);
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
      .text_size(10)
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
      .text_size(12.5)
      .text_color(t.muted)
      .cursor_pointer()
      .hover((style) => style.bg(t.hover).text_color(t.foreground))
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
      .text_size(12.5)
      .font_medium()
      .cursor_pointer()
      .hover((style) => style.bg(t.hover))
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
          .child(Input.new(this.noteInput).w_full().text_size(12.5).text_color(t.foreground)),
      )
      .child(this.cardButton("composer-note-cancel", "Cancel", (_e, cx) => this.closeNote(cx), t))
      .child(this.cardButton("composer-note-send", "Send", (_e, cx) => this.sendNote(cx), t, { filled: true }));
  },

  permissionCard(t, cx, ask) {
    const summary = permissionSummary(ask.input?.tool_name, ask.input?.tool_input);
    const suggestions = Array.isArray(ask.input?.permission_suggestions) ? ask.input.permission_suggestions : [];
    const head = v_flex()
      .w_full()
      .gap(4)
      .child(
        h_flex()
          .gap(6)
          .items_center()
          .child(this.cardTag(t, "Permission"))
          .child(div().text_size(13).font_medium().text_color(t.foreground).child(summary.title)),
      );
    if (summary.detail) {
      head.child(
        div()
          .text_size(12)
          .text_color(t.muted)
          .when(Boolean(summary.mono), (el) => el.font_family(settings()?.font_family ?? "Google Sans Code"))
          .child(summary.detail),
      );
    }
    if (ask.input?.cwd) head.child(div().text_size(11.5).text_color(t.dim).child(`Runs in ${ask.input.cwd}`));
    const buttons = h_flex().w_full().gap(6).items_center()
      .child(this.cardButton("composer-allow", "Allow", (_e, cx) => this.answer(ask, { action: "allow" }, cx), t, { filled: true }))
      .when(suggestions.length > 0, (el) => el.child(
        this.cardButton(
          "composer-allow-always",
          allowAlwaysLabel(ask),
          (_e, cx) => this.answer(ask, { action: "allow_always" }, cx),
          t,
          { tooltip: permissionDestination(ask) ?? "where Claude keeps its rules" },
        ),
      ))
      .child(div().flex_1())
      .child(this.textButton("composer-deny", "Deny", (_e, cx) => this.openNote("deny", cx), t));
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.blocked).bg(t.surface)
      .child(head)
      .when(this.noteOpen, (el) => el.child(this.noteField(t, cx)))
      .when(!this.noteOpen, (el) => el.child(buttons))
      .when(!this.noteOpen, (el) => el.child(div().text_size(11).text_color(t.dim).child("Enter to allow · Esc to deny")))
      .child(this.terminalAnswerLink(ask, t, cx));
  },

  planCard(t, cx, ask) {
    const plan = planText(ask);
    const buttons = h_flex().w_full().gap(6).items_center()
      .child(this.cardButton("composer-approve", "Approve", (_e, cx) => this.answer(ask, { action: "approve" }, cx), t, { filled: true }))
      .child(this.cardButton("composer-keep-planning", "Change the plan…", (_e, cx) => this.openNote("keep_planning", cx), t));
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.border).bg(t.surface)
      .child(
        h_flex()
          .gap(6)
          .items_center()
          .child(this.cardTag(t, "Plan"))
          .child(div().text_size(13).font_medium().text_color(t.foreground).child("Claude's plan is ready")),
      )
      .when(Boolean(plan), (el) => el.child(
        div()
          .w_full()
          .max_h(240)
          .overflow_y_scrollbar()
          .child(TextView.markdown("composer-plan-text", plan).text_size(12.5).text_color(t.muted).selectable(true)),
      ))
      .when(this.noteOpen, (el) => el.child(this.noteField(t, cx)))
      .when(!this.noteOpen, (el) => el.child(buttons))
      .child(this.terminalAnswerLink(ask, t, cx));
  },

  questionCard(t, cx, ask) {
    const questions = askQuestions(ask);
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.border).bg(t.surface)
      .child(h_flex().child(this.cardTag(t, "Question")))
      .children(questions.map((question, questionIndex) => {
        const answer = this.answers[question.question];
        const chosen = Array.isArray(answer) ? answer : typeof answer === "string" ? [answer] : [];
        return v_flex()
          .w_full()
          .gap(4)
          .child(div().text_size(12.5).font_medium().text_color(t.foreground).child(question.header || question.question))
          .when(Boolean(question.header), (el) => el.child(div().text_size(12).text_color(t.muted).child(question.question)))
          .children((question.options ?? []).map((option, optionIndex) => {
            const label = String(option.label ?? "");
            const picked = chosen.includes(label);
            return h_flex()
              .id(`composer-option-${questionIndex}-${optionIndex}`)
              .w_full()
              .gap(6)
              .items_center()
              .px(8)
              .py(5)
              .rounded(7)
              .border_1()
              .border_color(picked ? t.accent : t.input)
              .bg(picked ? t.selected : t.raised)
              .cursor_pointer()
              .hover((style) => style.bg(t.hover))
              .on_click((_e, cx) => this.toggleOption(question, label, cx))
              .child(div().text_size(12.5).font_medium().text_color(t.foreground).child(`${optionIndex + 1}. ${label}`))
              .when(
                Boolean(option.description) && !repeatsLabel(label, option.description),
                (el) => el.child(div().text_size(11.5).text_color(t.dim).child(option.description)),
              );
          }));
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
      .text_size(12.5)
      .font_medium()
      .child("Send answers");
    if (!enabled) return button.bg(t.hover).text_color(t.dim);
    return button
      .bg(t.accent_fill)
      .text_color(t.accent_foreground)
      .cursor_pointer()
      .hover((style) => style.opacity(0.88))
      .on_click((_e, cx) => this.sendAnswers(ask, cx));
  },

  terminalAnswerLink(ask, t, cx) {
    return h_flex()
      .id("composer-terminal-answer")
      .items_center()
      .cursor_pointer()
      .text_size(11.5)
      .text_color(t.dim)
      .hover((style) => style.text_color(t.foreground))
      .on_click((_e, cx) => this.answerInTerminal(ask, cx))
      .child("or answer in the terminal");
  },

  card(t, cx) {
    if (this.ask.kind === "plan") return this.planCard(t, cx, this.ask);
    if (this.ask.kind === "question") return this.questionCard(t, cx, this.ask);
    return this.permissionCard(t, cx, this.ask);
  },
};
