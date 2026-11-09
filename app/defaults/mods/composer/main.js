// The composer under the lead's terminal: one box that sends only what the
// Owner typed, when they press Enter or click Send. Files and folders dropped
// on it, and images pasted into it, wait as chips above the text.
//
// While Claude Code asks (a permission, a plan or a question) its card takes
// the box's place, and the Owner's click or key answers the hook.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState, InputState, Input, TextView, Popover } from "gpui-base";
import {
  kom,
  run,
  theme,
  props,
  shared,
  setShared,
  terminals,
  knownAgents,
  appHome,
  readTextFile,
  writeTextFile,
  pickFiles,
  pasteImage,
  lead as leadOf,
  nextCommand,
  settings,
} from "kompitum";

const HISTORY_KEY = "composer.history";
const HISTORY_LIMIT = 100;
// The box's height without its text, one line of text, and the chips' row (window.rs sizes the pane).
const BOX_PX = 76;
const LINE_PX = 20;
const CHIPS_PX = 28;
function errorText(error) {
  if (error == null) return "The command failed.";
  if (typeof error === "string") return error;
  if (typeof error.message === "string" && error.message.trim()) return error.message.trim();
  for (const raw of [error.stderr, error.stdout]) {
    if (typeof raw !== "string" || !raw.trim()) continue;
    try {
      const said = JSON.parse(raw.trim());
      const message = said?.error?.message ?? said?.message ?? said?.error;
      if (typeof message === "string" && message.trim()) return message.trim();
    } catch {}
    return raw.trim();
  }
  if (typeof error.code === "number") return `The command failed with exit code ${error.code}.`;
  return String(error);
}

function planText(ask) {
  const plan = ask?.input?.tool_input?.plan;
  return typeof plan === "string" && plan.trim() ? plan : null;
}

function baseName(path) {
  return String(path ?? "").split(/[\\/]/).filter(Boolean).pop() ?? String(path ?? "");
}

function quotePath(path) {
  const text = String(path ?? "");
  return /[ \t]/.test(text) ? `"${text}"` : text;
}

function loadHistory() {
  try {
    const list = JSON.parse(localStorage.getItem(HISTORY_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter((text) => typeof text === "string").slice(0, HISTORY_LIMIT) : [];
  } catch {
    return [];
  }
}

function saveHistory(list) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(list.slice(0, HISTORY_LIMIT)));
  } catch {}
}

function modelLabel(entry, id) {
  return entry?.models?.find((model) => model.id === id)?.label ?? id;
}

function leadName(entry, key) {
  return key === "claude" ? "Claude" : entry?.name ?? key;
}

const EFFORT_LEVELS = ["low", "medium", "high", "xhigh", "max"];

// The catalog's own label for the chosen effort; a bare level says it is effort.
function effortWords(entry, effort) {
  if (!effort) return null;
  const found = (entry?.efforts ?? []).find((level) => {
    const id = typeof level === "string" ? level : level?.id;
    return id === effort || level?.label === effort;
  });
  const label = String(typeof found === "object" ? found?.label ?? effort : found ?? effort);
  return EFFORT_LEVELS.includes(label) ? `${label} effort` : label;
}

function leadStatus(state) {
  if (state === "working") return "Working";
  if (state === "asking") return "Waiting for you";
  if (state === "waiting") return "Your turn";
  return null;
}

function statusColor(t, state) {
  return state === "asking" ? t.working : state === "waiting" ? t.done : t.accent;
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

export default class Composer extends View {
  init(_props, cx) {
    const p = props() ?? {};
    this.terminal = typeof p.terminal === "string" ? p.terminal : null;
    this.person = {};
    this.agents = [];
    this.loading = false;
    this.sending = false;
    this.error = null;
    this.chips = [];
    this.history = loadHistory();
    this.historyAt = -1;
    this.draft = "";
    this.askAt = null;
    // The lead's state and its open ask, from Claude Code's hooks.
    this.state = null;
    this.ask = null;
    this.askId = null;
    this.noteOpen = false;
    this.noteInput = null;
    this.noteAction = null;
    this.answers = {};
    this.handle = cx.focus_handle();
    this.text = TextareaState.new({});
    this.text.set_auto_grow(1, 6);
    this.text.set_submit_on_enter(true);
    this.text.on("change", (_event, cx) => {
      if (this.text.value().length === 0) this.historyAt = -1;
      cx.notify();
      this.setHistory(this.text.value().length === 0 || this.historyAt >= 0, cx);
    });
    this.text.on("submit", (event, cx) => this.submit(event, cx));
    this.text.on("paste_other", (_event, cx) => this.pastedImage(cx));
    cx.bind_keys([
      { keystroke: "escape", action: "composer-escape", context: "Composer" },
      { keystroke: "escape", action: "composer-escape", context: "ComposerHistory" },
      { keystroke: "up", action: "composer-history-up", context: "ComposerHistory" },
      { keystroke: "down", action: "composer-history-down", context: "ComposerHistory" },
    ]);
    this.historyMode = true;
    this.text.set_key_context("ComposerHistory");
    this.takeAsk(cx);
    cx.spawn(async (cx) => {
      await this.load(cx);
      cx.notify();
    });
    cx.timer.every(5000, (cx) => {
      this.load(cx);
    });
    cx.timer.every(500, (cx) => {
      this.takeAsk(cx);
    });
    // The window hands over the keyboard ("focus") and what was typed on the lead's terminal ("type:<text>").
    cx.spawn(async (cx) => {
      for (;;) {
        const command = String((await nextCommand()) ?? "");
        if (this.ask) this.handle.focus();
        else this.text.focus();
        if (command.startsWith("type:") && !this.ask) {
          this.text.replace(command.slice("type:".length));
        }
        cx.notify();
      }
    });
  }

  async load(cx) {
    if (this.loading) return;
    this.loading = true;
    try {
      let person = {};
      try {
        const raw = await readTextFile(`${appHome()}/settings.json`);
        const parsed = raw ? JSON.parse(raw) : {};
        person = parsed && typeof parsed === "object" ? parsed : {};
      } catch {}
      this.person = person;
      try {
        this.agents = await knownAgents();
      } catch {
        this.agents = [];
      }
    } finally {
      this.loading = false;
      cx.notify();
    }
  }

  leadProgram() {
    const key = this.person?.lead?.program ?? "claude";
    return this.agents?.find((entry) => entry.key === key) ?? { key, name: key, models: [] };
  }

  leadLive() {
    return Boolean(
      this.terminal
        && terminals().some((terminal) => terminal.address === this.terminal && terminal.state === "live"),
    );
  }

  leadLine() {
    const lead = this.person?.lead ?? {};
    const entry = this.leadProgram();
    const parts = [leadName(entry, entry.key)];
    if (lead.model) parts.push(modelLabel(entry, lead.model));
    const effort = effortWords(entry, lead.effort);
    if (effort) parts.push(effort);
    return parts.join(" · ");
  }

  takeAsk(cx) {
    if (!this.leadLive()) return;
    const open = leadOf()?.ask ?? null;
    if (open?.id && open.id !== this.focusedAsk) {
      this.focusedAsk = open.id;
      this.handle.focus();
    } else if (!open && this.focusedAsk) {
      this.focusedAsk = null;
      this.text.focus();
    }
    const ask = shared("composer.ask");
    if (!ask || typeof ask.text !== "string" || !ask.text.trim()) return;
    if (this.askAt === ask.at) return;
    this.askAt = ask.at;
    setShared("composer.ask", null);
    this.text.set_value(ask.text);
    this.setHistory(true, cx);
    this.text.focus();
    cx.notify();
  }

  addChip(path) {
    const text = String(path ?? "");
    if (!text || this.chips.some((chip) => chip.path === text)) return;
    this.chips.push({ path: text });
  }

  removeChip(index, cx) {
    this.chips.splice(index, 1);
    cx.notify();
  }

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
  }

  escape(cx) {
    if (this.state === "asking") {
      if (this.ask) this.cardEscape(this.ask, cx);
      return;
    }
    if (this.text.value().length === 0 && this.leadLive()) this.stopTurn(cx);
  }

  cardEscape(ask, cx) {
    if (this.noteOpen) {
      this.closeNote(cx);
      return;
    }
    this.answer(ask, ask.kind === "plan" ? { action: "keep_planning" } : { action: "deny" }, cx);
  }

  // The keys reach the card while one shows. History's Up and Down are bound
  // on the textarea's own ComposerHistory context instead, so they outrank the
  // editor's caret keys only while the box is empty or browsing history.
  keyDown(event, cx) {
    if (this.state === "asking" && this.ask) this.cardKey(event, cx);
  }

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
  }

  answer(ask, answer, cx) {
    if (!ask) return;
    this.noteOpen = false;
    this.noteInput = null;
    run("lead.answer", { id: ask.id, answer });
    cx.notify();
  }

  openNote(action, cx) {
    this.noteAction = action;
    this.noteInput = InputState.new({
      placeholder: action === "deny" ? "Why not? (optional)" : "What should change? (optional)",
    });
    this.noteInput.on("submit", (_event, cx) => this.sendNote(cx));
    this.noteOpen = true;
    this.noteInput.focus();
    cx.notify();
  }

  closeNote(cx) {
    this.noteOpen = false;
    this.noteInput = null;
    this.noteAction = null;
    this.handle.focus();
    cx.notify();
  }

  sendNote(cx) {
    if (!this.ask || !this.noteInput) return;
    this.answer(this.ask, { action: this.noteAction, note: this.noteInput.value() }, cx);
  }

  questionAnswered(question) {
    const value = this.answers[question.question];
    if (Array.isArray(value)) return value.length > 0;
    return typeof value === "string" && value.length > 0;
  }

  allAnswered(ask) {
    const questions = askQuestions(ask);
    return questions.length > 0 && questions.every((question) => this.questionAnswered(question));
  }

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
  }

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
  }

  sendAnswers(ask, cx) {
    if (!this.allAnswered(ask)) return;
    this.answer(ask, { action: "answers", answers: this.answers }, cx);
  }

  answerInTerminal(ask, cx) {
    this.answer(ask, { action: "terminal" }, cx);
  }

  setHistory(value, cx) {
    if (this.historyMode === value) return;
    this.historyMode = value;
    if (this.text) this.text.set_key_context(value ? "ComposerHistory" : null);
    cx.notify();
  }

  historyKey(direction, cx) {
    // A card owns the keys while it shows; the draft under it stays put.
    if (this.ask) return;
    if (direction > 0) {
      if (this.historyAt < 0 && this.text.value().length > 0) return;
      if (this.historyAt >= this.history.length - 1) return;
    } else if (this.historyAt < 0) {
      return;
    }
    this.moveHistory(direction, cx);
  }

  moveHistory(delta, cx) {
    if (this.historyAt < 0) this.draft = this.text.value();
    const next = this.historyAt + delta;
    if (next < 0) {
      this.historyAt = -1;
      this.text.set_value(this.draft);
    } else if (next < this.history.length) {
      this.historyAt = next;
      this.text.set_value(this.history[next]);
    }
    this.setHistory(this.text.value().length === 0 || this.historyAt >= 0, cx);
    this.text.focus();
    cx.notify();
  }

  stopTurn(cx) {
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["keys", this.terminal, "esc"]);
        if (!r || r.code !== 0) throw new Error(errorText(r));
      } catch (error) {
        this.error = errorText(error);
      }
      cx.notify();
    });
  }

  submit(event, cx) {
    if (event?.shift) return;
    const value = this.text.value();
    if (value.endsWith("\\")) {
      this.text.set_value(`${value.slice(0, -1)}\n`);
      this.text.focus();
      cx.notify();
      return;
    }
    this.send(cx);
  }

  send(cx) {
    if (this.sending) return;
    if (this.state === "asking") return;
    const text = this.text.value();
    const paths = this.chips.map((chip) => chip.path);
    if (!text.trim() && paths.length === 0) return;
    if (!this.leadLive()) {
      this.error = "The lead isn't running";
      cx.notify();
      return;
    }
    const body = [text.trim() ? text : "", ...paths.map(quotePath)].filter(Boolean).join(" ");
    this.sending = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await kom(["send", this.terminal, body]);
        if (!r || r.code !== 0) throw new Error(errorText(r));
        this.text.set_value("");
        this.chips = [];
        this.error = null;
        this.historyAt = -1;
        this.draft = "";
        this.setHistory(true, cx);
        if (text.trim()) {
          this.history = [text, ...this.history.filter((message) => message !== text)].slice(0, HISTORY_LIMIT);
          saveHistory(this.history);
        }
      } catch (error) {
        // The words stay in the box; the refusal shows under it.
        this.error = errorText(error);
      } finally {
        this.sending = false;
        cx.notify();
      }
    });
  }

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
  }

  statusWord(t) {
    const word = leadStatus(this.state);
    if (!word) return null;
    // Working pulses; asking is the amber that wants the Owner; waiting is still.
    return div()
      .text_size(11.5)
      .text_color(statusColor(t, this.state))
      .child(word)
      .when(this.state === "working", (el) => el.loop("opacity", { from: 0.35, to: 1, period: 900 }));
  }

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
          .w(14)
          .h(14)
          .items_center()
          .justify_center()
          .rounded(4)
          .text_size(11)
          .text_color(t.dim)
          .cursor_pointer()
          .hover((style) => style.bg(t.hover).text_color(t.foreground))
          .on_click(() => this.removeChip(index, cx))
          .child("×"),
      );
  }

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
  }

  // The model chip writes the lead's model or reasoning for its next start, as Settings › Agents does.
  saveLead(patch, cx) {
    cx.spawn(async (cx) => {
      const path = `${appHome()}/settings.json`;
      try {
        const raw = await readTextFile(path);
        const person = raw ? JSON.parse(raw) : {};
        person.lead = { ...(person.lead ?? {}), ...patch };
        await writeTextFile(path, `${JSON.stringify(person, null, 2)}\n`, raw);
        this.person = person;
      } catch (error) {
        this.error = `Couldn't save the lead's model: ${String(error?.message ?? error)}`;
      }
      cx.notify();
    });
  }

  // The lead's model and reasoning, as one quiet chip that opens a list upward.
  modelChip(t) {
    const entry = this.leadProgram();
    const lead = this.person?.lead ?? {};
    const models = (entry?.models ?? []).map((model) => [model.id, model.label]);
    const efforts = (entry?.efforts ?? []).map((level) => [level, level.replace(/^./, (c) => c.toUpperCase())]);
    const label =
      [lead.model ? modelLabel(entry, lead.model) : null, effortWords(entry, lead.effort)].filter(Boolean).join(" · ") ||
      leadName(entry, entry.key);
    const group = (key, title, list, chosen) =>
      v_flex()
        .gap(1)
        .child(div().px(8).pt(6).pb(2).text_size(11).text_color(t.dim).child(title))
        .children(list.map(([value, text]) =>
          h_flex()
            .id(`composer-${key}-${value}`)
            .h(28)
            .px(8)
            .gap(8)
            .items_center()
            .rounded(6)
            .cursor_pointer()
            .hover((s) => s.bg(t.hover))
            .on_click((_e, cx) => {
              this.modelOpen = false;
              this.saveLead({ [key]: value }, cx);
            })
            .child(div().flex_1().text_size(13).text_color(t.foreground).child(text))
            .when(value === chosen, (el) => el.child(svg("frame/settings/check.svg").size(14).text_color(t.accent)))));
    const content = v_flex()
      .w(230)
      .p(4)
      .rounded(10)
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .when(models.length > 0, (el) => el.child(group("model", "Model", models, lead.model)))
      .when(efforts.length > 0, (el) => el.child(group("effort", "Reasoning", efforts, lead.effort)))
      .child(div().px(8).pt(6).pb(4).text_size(11.5).line_height("16px").text_color(t.dim)
        .child("For the lead's next start. Type /model to switch now."));
    const trigger = h_flex()
      .id("composer-model")
      .h(28)
      .px(8)
      .gap(5)
      .items_center()
      .rounded(7)
      .cursor_pointer()
      .text_size(12.5)
      .text_color(t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      .tooltip(`The lead: ${this.leadLine()}`)
      .child(label)
      .child(svg("mods/workspace-chrome/chevron-down.svg").size(11));
    return Popover.new("composer-model-menu")
      .anchor("bottom_right")
      .open(Boolean(this.modelOpen))
      .on_open_change((open, cx) => {
        this.modelOpen = open;
        cx.notify();
      })
      .trigger(trigger)
      .content(content);
  }

  // One floating box: files, the text (no placeholder), then "+", the lead's status, its model and Send.
  // While the lead asks, its card sits above the text in the same box.
  bar(t, cx) {
    const ready = !this.sending && Boolean(this.text?.value?.()?.trim()) || this.chips.length > 0;
    const send = h_flex()
      .id("composer-send")
      .w(30)
      .h(30)
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(15)
      .cursor_pointer()
      .tooltip(this.sending ? "Sending…" : "Send (Enter)")
      .when(ready, (el) => el.bg(t.foreground).hover((s) => s.opacity(0.85)))
      .when(!ready, (el) => el.border_1().border_color(t.input))
      .on_click(() => this.send(cx))
      .child(svg("mods/composer/arrow-up.svg").size(16).text_color(ready ? t.background : t.dim));
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
      .on_click(() => this.attach(cx))
      .child(svg("mods/composer/plus.svg").size(16));
    const typing = Boolean(leadOf()?.typing);
    const keys = h_flex()
      .id("composer-type")
      .w(28)
      .h(28)
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(14)
      .cursor_pointer()
      .text_color(typing ? t.accent : t.muted)
      .when(typing, (el) => el.bg(t.selected))
      .hover((s) => s.bg(t.hover).text_color(typing ? t.accent : t.foreground))
      .tooltip(typing ? "Typing in the terminal: click to type here again" : "Type in terminal (for logins and the CLI's own menus)")
      .on_click(() => run("lead.type", { terminal: this.terminal, on: !typing }))
      .child(svg("mods/composer/keyboard.svg").size(16));
    const note = this.error
      ? div().id("composer-error").min_w(0).truncate().text_size(11.5).text_color(t.blocked).tooltip(this.error).child(this.error)
      : this.state ? this.statusWord(t) : null;
    return div()
      .size_full()
      .p(10)
      .bg(t.background)
      .child(
        v_flex()
          .id("composer-box")
          .size_full()
          .px(12)
          .pt(10)
          .pb(8)
          .gap(6)
          .rounded(16)
          .border_1()
          .border_color(this.ask ? (this.ask.kind === "permission" ? t.blocked : t.input) : t.border)
          .bg(t.raised)
          .when(this.chips.length > 0, (el) =>
            el.child(h_flex().gap(6).flex_wrap().children(this.chips.map((chip, index) => this.chip(t, cx, chip, index)))))
          .when(Boolean(this.ask), (el) =>
            el.child(v_flex().id("composer-asks").flex_1().min_h(0).overflow_y_scrollbar().child(this.card(t, cx))))
          .when(!this.ask, (el) => el.child(div().flex_1().min_h(0).child(Textarea.new(this.text).w_full())))
          .child(
            h_flex()
              .w_full()
              .gap(6)
              .items_center()
              .child(plus)
              .when(Boolean(this.terminal), (el) => el.child(keys))
              .when(Boolean(note), (el) => el.child(div().min_w(0).child(note)))
              .child(div().flex_1())
              .child(this.modelChip(t))
              .when(!this.ask, (el) => el.child(send)),
          ),
      );
  }

  startLabel() {
    const lead = this.person?.lead ?? {};
    const entry = this.leadProgram();
    const model = lead.model ? ` (${modelLabel(entry, lead.model)})` : "";
    return `Start ${leadName(entry, entry.key)}${model}`;
  }

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
  }

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
  }

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
  }

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
  }

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
      .child(this.cardButton("composer-note-cancel", "Cancel", () => this.closeNote(cx), t))
      .child(this.cardButton("composer-note-send", "Send", () => this.sendNote(cx), t, { filled: true }));
  }

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
      .child(this.cardButton("composer-allow", "Allow", () => this.answer(ask, { action: "allow" }, cx), t, { filled: true }))
      .when(suggestions.length > 0, (el) => el.child(
        this.cardButton(
          "composer-allow-always",
          allowAlwaysLabel(ask),
          () => this.answer(ask, { action: "allow_always" }, cx),
          t,
          { tooltip: permissionDestination(ask) ?? "where Claude keeps its rules" },
        ),
      ))
      .child(div().flex_1())
      .child(this.textButton("composer-deny", "Deny", () => this.openNote("deny", cx), t));
    return v_flex().id("composer-card").w_full().gap(8).p(10).rounded(8).border_1().border_color(t.blocked).bg(t.surface)
      .child(head)
      .when(this.noteOpen, (el) => el.child(this.noteField(t, cx)))
      .when(!this.noteOpen, (el) => el.child(buttons))
      .when(!this.noteOpen, (el) => el.child(div().text_size(11).text_color(t.dim).child("Enter to allow · Esc to deny")))
      .child(this.terminalAnswerLink(ask, t, cx));
  }

  planCard(t, cx, ask) {
    const plan = planText(ask);
    const buttons = h_flex().w_full().gap(6).items_center()
      .child(this.cardButton("composer-approve", "Approve", () => this.answer(ask, { action: "approve" }, cx), t, { filled: true }))
      .child(this.cardButton("composer-keep-planning", "Change the plan…", () => this.openNote("keep_planning", cx), t));
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
  }

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
              .on_click(() => this.toggleOption(question, label, cx))
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
  }

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
      .on_click(() => this.sendAnswers(ask, cx));
  }

  terminalAnswerLink(ask, t, cx) {
    return h_flex()
      .id("composer-terminal-answer")
      .items_center()
      .cursor_pointer()
      .text_size(11.5)
      .text_color(t.dim)
      .hover((style) => style.text_color(t.foreground))
      .on_click(() => this.answerInTerminal(ask, cx))
      .child("or answer in the terminal");
  }

  card(t, cx) {
    if (this.ask.kind === "plan") return this.planCard(t, cx, this.ask);
    if (this.ask.kind === "question") return this.questionCard(t, cx, this.ask);
    return this.permissionCard(t, cx, this.ask);
  }

  compose(t, cx) {
    const lead = leadOf();
    this.state = lead?.state ?? null;
    this.ask = lead?.ask ?? null;
    const askId = this.ask?.id ?? null;
    if (askId !== this.askId) {
      this.askId = askId;
      this.noteOpen = false;
      this.noteInput = null;
      this.noteAction = null;
      this.answers = {};
      if (askId) cx.timer.after(0, () => this.handle.focus());
    }
    this.fit(cx);
    return v_flex()
      .id("composer")
      .size_full()
      .min_h(0)
      .bg(t.background)
      .track_focus(this.handle)
      .key_context(this.historyMode ? "ComposerHistory" : "Composer")
      .on_action("composer-escape", (_event, cx) => this.escape(cx))
      .on_action("composer-history-up", (_event, cx) => this.historyKey(1, cx))
      .on_action("composer-history-down", (_event, cx) => this.historyKey(-1, cx))
      .on_key_down((event, cx) => this.keyDown(event, cx))
      .on_drop((event, cx) => {
        for (const path of event.paths ?? []) this.addChip(path);
        cx.notify();
      })
      .on_size_change((size, cx) => {
        this.width = size.width;
        cx.notify();
      })
      .child(this.bar(t, cx));
  }

  // The pane grows with the text, up to six lines; the window holds the height (lead.fit).
  fit(cx) {
    if (!this.terminal || this.ask) return;
    const perLine = Math.max(12, Math.floor(((this.width ?? 640) - 46) / 7.3));
    const lines = String(this.text?.value?.() ?? "")
      .split(/\r?\n/)
      .reduce((n, line) => n + Math.max(1, Math.ceil(line.length / perLine)), 0);
    const px = BOX_PX + Math.min(6, Math.max(1, lines)) * LINE_PX + (this.chips.length ? CHIPS_PX : 0);
    if (px === this.fitPx) return;
    this.fitPx = px;
    cx.timer.after(0, () => run("lead.fit", { terminal: this.terminal, px }));
  }

  render(cx) {
    const t = theme();
    if (!this.leadLive()) return this.empty(t);
    return this.compose(t, cx);
  }
}
