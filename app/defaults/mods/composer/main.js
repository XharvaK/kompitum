// The composer under the lead's terminal: one box that sends only what the
// Owner typed, when they press Enter or click Send. Files and folders dropped
// on it, and images pasted into it, wait as chips above the text.
//
// While Claude Code asks (a permission, a plan or a question) its card takes
// the box's place, and the Owner's click or key answers the hook.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState, InputState, Input, TextView, Popover, Slider, SliderState, SliderTrack, SliderIndicator, SliderThumb } from "gpui-base";
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
  conversation as conversationFrom,
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

const EFFORT_WORDS = { low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max" };

function cap(text) {
  const value = String(text ?? "");
  return value ? value.charAt(0).toUpperCase() + value.slice(1) : value;
}

function effortWord(effort) {
  if (!effort) return null;
  return EFFORT_WORDS[effort] ?? cap(effort);
}

function effortIds(entry) {
  return (entry?.efforts ?? [])
    .map((level) => (typeof level === "string" ? level : level?.id))
    .filter(Boolean);
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
    this.modelOpen = false;
    this.pickerView = "models";
    this.effortSlider = null;
    this.effortSliderMax = null;
    this.effortPreview = null;
    this.effortTouched = false;
    this.saveEffortOnRelease = true;
    this.firstModel = null;
    this.items = [];
    this.seenVersion = null;
    this.stickToBottom = true;
    this.openTools = {};
    this.openToolGroups = {};
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
    // The window hands over the keyboard ("focus") when the lead asks or the composer should type.
    cx.spawn(async (cx) => {
      for (;;) {
        await nextCommand();
        if (this.ask) this.handle.focus();
        else this.text.focus();
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
      if (this.needsFirstPick()) {
        this.saveEffortOnRelease = false;
        this.ensureEffortSlider(this.leadProgram());
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

  needsFirstPick() {
    if (this.leadLive()) return false;
    if (this.person?.lead?.model) return false;
    return (this.leadProgram().models ?? []).length > 0;
  }

  leadLine() {
    const lead = this.person?.lead ?? {};
    const entry = this.leadProgram();
    const parts = [leadName(entry, entry.key)];
    if (lead.model) parts.push(modelLabel(entry, lead.model));
    const effort = effortWord(lead.effort);
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
  async writeLead(patch) {
    const lead = { ...(this.person?.lead ?? {}), ...patch };
    for (const [key, value] of Object.entries(patch)) {
      if (value == null) delete lead[key];
    }
    this.person = { ...(this.person ?? {}), lead };
    const path = `${appHome()}/settings.json`;
    const raw = await readTextFile(path);
    const person = raw ? JSON.parse(raw) : {};
    const next = { ...(person.lead ?? {}), ...patch };
    for (const [key, value] of Object.entries(patch)) {
      if (value == null) delete next[key];
    }
    person.lead = next;
    await writeTextFile(path, `${JSON.stringify(person, null, 2)}\n`, raw);
    this.person = person;
  }

  saveLead(patch, cx) {
    cx.spawn(async (cx) => {
      try {
        await this.writeLead(patch);
      } catch (error) {
        this.error = `Couldn't save the lead's model: ${String(error?.message ?? error)}`;
      }
      cx.notify();
    });
  }

  startFirstPick(cx) {
    if (!this.firstModel) return;
    const patch = { model: this.firstModel };
    if (this.effortTouched && this.effortPreview) patch.effort = this.effortPreview;
    cx.spawn(async (cx) => {
      try {
        await this.writeLead(patch);
        run("lead.open");
      } catch (error) {
        this.error = `Couldn't save the lead's model: ${String(error?.message ?? error)}`;
      }
      cx.notify();
    });
  }

  ensureEffortSlider(entry) {
    const levels = effortIds(entry);
    const max = Math.max(0, levels.length - 1);
    const saved = this.person?.lead?.effort ?? null;
    const index = saved && levels.includes(saved) ? levels.indexOf(saved) : 0;
    if (this.effortSlider && this.effortSliderMax === max) return;
    this.effortSlider = SliderState.new({ min: 0, max, step: 1, value: index });
    this.effortSliderMax = max;
    this.effortSlider.on("change", (value, cx) => {
      const levels = effortIds(this.leadProgram());
      this.effortTouched = true;
      this.effortPreview = levels[Number(value)] ?? null;
      cx.notify();
    });
    this.effortSlider.on("release", (value, cx) => {
      const levels = effortIds(this.leadProgram());
      const effort = levels[Number(value)] ?? null;
      this.effortTouched = true;
      this.effortPreview = effort;
      if (this.saveEffortOnRelease && effort) this.saveLead({ effort }, cx);
      cx.notify();
    });
  }

  pickModel(id, cx) {
    this.saveLead({ model: id }, cx);
    const entry = this.leadProgram();
    if (effortIds(entry).length > 0) {
      this.pickerView = "effort";
      this.ensureEffortSlider(entry);
    } else {
      this.modelOpen = false;
    }
    cx.notify();
  }

  resetEffort(cx) {
    this.saveLead({ effort: null }, cx);
    this.effortTouched = false;
    this.effortPreview = null;
    this.effortSlider?.set_value(0);
    cx.notify();
  }

  // Dots sit on the track; those already on the fill use the accent at half strength.
  effortDots(t, count, filledThrough) {
    return h_flex()
      .absolute()
      .inset_0()
      .items_center()
      .justify_between()
      .children(Array.from({ length: count }, (_, index) => {
        const onFill = filledThrough != null && index <= filledThrough;
        return div()
          .w(4)
          .h(4)
          .rounded(2)
          .bg(onFill ? t.accent_foreground : t.dim)
          .when(onFill, (el) => el.opacity(0.5));
      }));
  }

  effortSliderEl(t, entry) {
    const levels = effortIds(entry);
    if (!this.effortSlider || levels.length === 0) return null;
    const preview = this.effortPreview ?? this.person?.lead?.effort ?? null;
    const filledThrough = this.effortTouched && preview && levels.includes(preview)
      ? levels.indexOf(preview)
      : null;
    let indicator = SliderIndicator.new(this.effortSlider)
      .absolute()
      .top(0)
      .left(0)
      .w_full()
      .h(30)
      .rounded(15);
    if (this.effortTouched) {
      indicator = indicator
        .range_style((fill) => fill.rounded(15).bg(t.accent))
        .child(
          SliderThumb.new(this.effortSlider)
            .size(24)
            .rounded(12)
            .bg(t.accent_foreground)
            .border_1()
            .border_color(t.border)
            .mt(3)
            .ml(-12),
        );
    }
    return Slider.new(this.effortSlider)
      .id("composer-effort-slider")
      .w_full()
      .h(30)
      .mt(10)
      .child(
        SliderTrack.new(this.effortSlider)
          .relative()
          .w_full()
          .h(30)
          .rounded(15)
          .bg(t.hover)
          .child(this.effortDots(t, levels.length, filledThrough))
          .child(indicator),
      );
  }

  modelsView(t, entry, lead) {
    const models = entry?.models ?? [];
    return v_flex()
      .child(div().px(8).pt(4).pb(2).text_size(11.5).text_color(t.dim).child("Model"))
      .children(models.map((model) =>
        h_flex()
          .id(`composer-model-${model.id}`)
          .h(32)
          .px(8)
          .items_center()
          .rounded(7)
          .cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => this.pickModel(model.id, cx))
          .child(div().flex_1().text_size(13).text_color(t.foreground).child(model.label ?? model.id))
          .when(model.id === lead.model, (el) =>
            el.child(svg("frame/settings/check.svg").size(14).text_color(t.accent)))))
      .child(div().px(8).pt(6).pb(2).text_size(11.5).text_color(t.dim)
        .child("For the lead's next start. Type /model to switch now."));
  }

  effortView(t, entry, lead) {
    const preview = this.effortPreview ?? lead.effort ?? null;
    const word = effortWord(preview);
    return v_flex()
      .id("composer-effort")
      .relative()
      .w_full()
      .items_center()
      .child(
        div()
          .text_size(15)
          .font_semibold()
          .text_color(word ? t.accent : t.muted)
          .child(word ?? "Its own default"),
      )
      .when(Boolean(lead.effort), (el) => el.child(
        h_flex()
          .id("composer-effort-reset")
          .absolute()
          .top(0)
          .right(0)
          .w(24)
          .h(24)
          .items_center()
          .justify_center()
          .rounded(6)
          .cursor_pointer()
          .text_color(t.muted)
          .hover((s) => s.bg(t.hover).text_color(t.foreground))
          .tooltip("Use its own default")
          .on_click((_e, cx) => this.resetEffort(cx))
          .child(svg("mods/composer/rotate-ccw.svg").size(14)),
      ))
      .child(
        h_flex()
          .id("composer-effort-model")
          .gap(4)
          .items_center()
          .cursor_pointer()
          .text_size(12.5)
          .text_color(t.muted)
          .hover((s) => s.text_color(t.foreground))
          .on_click((_e, cx) => {
            this.pickerView = "models";
            cx.notify();
          })
          .child(lead.model ? modelLabel(entry, lead.model) : "")
          .child("›"),
      )
      .child(this.effortSliderEl(t, entry));
  }

  // The lead's model and reasoning: models first, then Codex-style effort.
  modelChip(t) {
    const entry = this.leadProgram();
    const lead = this.person?.lead ?? {};
    const model = lead.model ? modelLabel(entry, lead.model) : null;
    const level = effortWord(lead.effort);
    let trigger = h_flex()
      .id("composer-model")
      .h(28)
      .px(8)
      .gap(5)
      .items_center()
      .rounded(7)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .tooltip("The lead's model and reasoning, for its next start");
    if (model) {
      trigger = trigger.child(div().text_size(12.5).font_medium().text_color(t.foreground).child(model));
      if (level) trigger = trigger.child(div().text_size(12.5).text_color(t.dim).child(level));
    } else {
      trigger = trigger.child(div().text_size(12.5).text_color(t.muted).child("Choose a model"));
    }
    trigger = trigger.child(svg("mods/workspace-chrome/chevron-down.svg").size(11).text_color(t.dim));
    const content = v_flex()
      .w(260)
      .p(6)
      .rounded(12)
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .child(this.pickerView === "effort" && effortIds(entry).length > 0
        ? this.effortView(t, entry, lead)
        : this.modelsView(t, entry, lead));
    return Popover.new("composer-model-menu")
      .anchor("bottom_right")
      .open(Boolean(this.modelOpen))
      .on_open_change((open, cx) => {
        this.modelOpen = open;
        if (open) {
          this.pickerView = "models";
          this.effortPreview = lead.effort ?? null;
          this.effortTouched = Boolean(lead.effort);
          this.saveEffortOnRelease = true;
          this.ensureEffortSlider(entry);
        }
        cx.notify();
      })
      .trigger(trigger)
      .content(content);
  }

  // One floating box: files, the text (no placeholder), then "+", the lead's status, its model and Send.
  // While the lead asks, its card sits above the text in the same box.
  bar(t, cx) {
    const first = this.needsFirstPick();
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
          .when(first, (el) => el.child(this.firstPickCard(t, cx)))
          .when(!first && Boolean(this.ask), (el) =>
            el.child(v_flex().id("composer-asks").flex_1().min_h(0).overflow_y_scrollbar().child(this.card(t, cx))))
          .when(!first && !this.ask, (el) => el.child(div().flex_1().min_h(0).child(Textarea.new(this.text).w_full())))
          .child(
            h_flex()
              .w_full()
              .gap(6)
              .items_center()
              .child(plus)
              .when(Boolean(note), (el) => el.child(div().min_w(0).child(note)))
              .child(div().flex_1())
              .when(!first, (el) => el.child(this.modelChip(t)))
              .when(!first && !this.ask, (el) => el.child(send)),
          ),
      );
  }

  startLabel() {
    const lead = this.person?.lead ?? {};
    const entry = this.leadProgram();
    const model = lead.model ? ` (${modelLabel(entry, lead.model)})` : "";
    return `Start ${leadName(entry, entry.key)}${model}`;
  }

  firstPickCard(t, cx) {
    const entry = this.leadProgram();
    const models = entry?.models ?? [];
    const picked = this.firstModel;
    const preview = this.effortPreview ?? this.person?.lead?.effort ?? null;
    const word = effortWord(preview);
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
      .when(effortIds(entry).length > 0, (el) => el
        .child(
          div()
            .text_size(15)
            .font_semibold()
            .text_color(word ? t.accent : t.muted)
            .child(word ?? "Its own default"),
        )
        .child(this.effortSliderEl(t, entry)))
      .child(button);
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

  syncConversation() {
    const meta = leadOf()?.conversation;
    if (!meta) {
      if (this.seenVersion !== null) {
        this.items = [];
        this.seenVersion = null;
      }
      return;
    }
    if (meta.version === this.seenVersion) return;
    let start = this.items.length > 0 ? Math.max(0, this.items.length - 1) : 0;
    if ((meta.length ?? 0) < this.items.length) start = 0;
    let fresh = [];
    try {
      fresh = conversationFrom(start) ?? [];
    } catch {
      fresh = [];
    }
    if (!Array.isArray(fresh)) fresh = [];
    this.items = this.items.slice(0, start).concat(fresh);
    this.seenVersion = meta.version;
  }

  terminalShown() {
    return leadOf()?.terminal === "shown";
  }

  terminalToggle(t) {
    const shown = this.terminalShown();
    return h_flex()
      .id("lead-terminal")
      .w(28)
      .h(28)
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(7)
      .text_color(shown ? t.foreground : t.muted)
      .when(shown, (el) => el.bg(t.selected))
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      .tooltip(shown ? "Back to the conversation" : "Show Claude's terminal")
      .on_click(() => run("lead.terminal", { terminal: this.terminal }))
      .child(svg("mods/workspace-chrome/terminal.svg").size(16));
  }

  revealNote(t) {
    const lead = leadOf();
    if (lead?.terminal !== "shown") return null;
    if (lead?.reveal !== "auto" && lead?.reveal !== "login") return null;
    const why = typeof lead.reveal_why === "string" && lead.reveal_why.trim() ? lead.reveal_why : null;
    if (!why) return null;
    return div()
      .id("composer-reveal-note")
      .px(20)
      .pb(8)
      .text_size(12)
      .text_color(t.dim)
      .child(why);
  }

  conversationGroups() {
    const groups = [];
    let i = 0;
    while (i < this.items.length) {
      const item = this.items[i];
      if (item?.kind === "tool" && item.state !== "running") {
        let j = i;
        while (j < this.items.length && this.items[j]?.kind === "tool" && this.items[j].state !== "running") j += 1;
        if (j - i >= 2) {
          groups.push({ type: "tools", items: this.items.slice(i, j), start: i });
          i = j;
          continue;
        }
      }
      groups.push({ type: "item", item, start: i });
      i += 1;
    }
    return groups;
  }

  ownerItem(t, item) {
    return h_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .justify_end()
      .child(
        div()
          .px(12)
          .py(8)
          .rounded(14)
          .bg(t.selected)
          .text_size(13.5)
          .text_color(t.foreground)
          .selectable(true)
          .child(item.text ?? ""),
      );
  }

  assistantItem(t, item) {
    return v_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .gap(6)
      .child(
        TextView.markdown(`composer-md-${item.id}`, item.text ?? "")
          .text_size(13.5)
          .text_color(t.foreground)
          .selectable(true),
      )
      .when(!item.done, (el) =>
        el.child(
          div()
            .id(`composer-streaming-${item.id}`)
            .mt(2)
            .size(6)
            .rounded(3)
            .bg(t.accent)
            .loop("opacity", { from: 0.35, to: 1, period: 900 }),
        ),
      );
  }

  toolIcon(t, state) {
    if (state === "running") return svg("mods/workspace-chrome/spinner.svg").size(14).text_color(t.muted);
    if (state === "failed") return svg("mods/workspace-chrome/x.svg").size(14).text_color(t.blocked);
    return svg("mods/workspace-chrome/check.svg").size(14).text_color(t.muted);
  }

  toolItem(t, item, cx) {
    const open = Boolean(this.openTools[item.id]);
    const result = typeof item.result === "string" && item.result ? item.result : "";
    return v_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .gap(6)
      .child(
        h_flex()
          .id(`composer-tool-${item.id}`)
          .gap(6)
          .items_center()
          .cursor_pointer()
          .on_click(() => {
            this.openTools[item.id] = !open;
            cx.notify();
          })
          .child(this.toolIcon(t, item.state))
          .child(div().text_size(12.5).text_color(t.muted).child(item.summary ?? item.name ?? "A tool")),
      )
      .when(open && result, (el) =>
        el.child(
          div()
            .id(`composer-tool-result-${item.id}`)
            .px(10)
            .py(8)
            .rounded(8)
            .bg(t.surface)
            .text_size(12)
            .font_family(settings()?.font_family ?? "Google Sans Code")
            .text_color(t.dim)
            .child(result.split(/\r?\n/).slice(0, 12).join("\n")),
        ),
      );
  }

  noticeItem(t, item) {
    return div()
      .id(`composer-item-${item.id}`)
      .w_full()
      .text_center()
      .text_size(12)
      .text_color(t.dim)
      .child(item.text ?? "");
  }

  toolsGroup(t, group, cx) {
    const open = Boolean(this.openToolGroups[group.start]);
    if (!open) {
      return h_flex()
        .id("composer-tools-used")
        .gap(6)
        .items_center()
        .cursor_pointer()
        .on_click(() => {
          this.openToolGroups[group.start] = true;
          cx.notify();
        })
        .child(svg("mods/workspace-chrome/check.svg").size(14).text_color(t.muted))
        .child(div().text_size(12.5).text_color(t.muted).child(`Used ${group.items.length} tools`));
    }
    return v_flex()
      .id("composer-tools-used")
      .w_full()
      .gap(14)
      .children(group.items.map((item) => this.toolItem(t, item, cx)));
  }

  drawItem(t, item, cx) {
    if (item?.kind === "owner") return this.ownerItem(t, item);
    if (item?.kind === "assistant") return this.assistantItem(t, item);
    if (item?.kind === "tool") return this.toolItem(t, item, cx);
    if (item?.kind === "notice") return this.noticeItem(t, item);
    return null;
  }

  conversationEmpty(t) {
    const entry = this.leadProgram();
    const name = leadName(entry, entry.key);
    return v_flex()
      .id("composer-conversation-empty")
      .flex_1()
      .min_h(0)
      .items_center()
      .justify_center()
      .gap(6)
      .child(div().text_size(14).text_color(t.foreground).child(name))
      .child(div().text_size(13).text_color(t.dim).child("Ask the lead anything."));
  }

  conversationPanel(t, cx) {
    const list = v_flex()
      .id("composer-conversation")
      .flex_1()
      .min_h(0)
      .px(20)
      .pt(16)
      .pb(16)
      .gap(14)
      .overflow_y_scrollbar()
      .when(this.stickToBottom, (el) => el.justify_end())
      .on_scroll_wheel((event, cx) => {
        if (event.delta.y < 0 && this.stickToBottom) {
          this.stickToBottom = false;
          cx.notify();
        }
      });
    if (this.items.length === 0) {
      list.child(this.conversationEmpty(t));
    } else {
      for (const group of this.conversationGroups()) {
        if (group.type === "tools") list.child(this.toolsGroup(t, group, cx));
        else list.child(this.drawItem(t, group.item, cx));
      }
    }
    return div()
      .id("composer-panel")
      .flex_1()
      .min_h(0)
      .relative()
      .child(list)
      .child(div().absolute().top(12).right(16).child(this.terminalToggle(t)))
      .when(!this.stickToBottom, (el) =>
        el.child(
          h_flex()
            .id("composer-jump-latest")
            .absolute()
            .bottom(8)
            .h(26)
            .px(12)
            .items_center()
            .rounded(13)
            .border_1()
            .border_color(t.border)
            .bg(t.raised)
            .text_size(12)
            .text_color(t.foreground)
            .cursor_pointer()
            .on_click(() => {
              this.stickToBottom = true;
              cx.notify();
            })
            .child("Jump to latest"),
        ),
      );
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
    const root = v_flex()
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
      });
    this.syncConversation();
    if (this.needsFirstPick()) {
      return root.child(this.bar(t, cx));
    }
    if (this.terminalShown()) {
      return root
        .child(
          h_flex()
            .id("composer-shown-chrome")
            .w_full()
            .px(16)
            .pt(10)
            .justify_end()
            .child(this.terminalToggle(t)),
        )
        .when(Boolean(this.revealNote(t)), (el) => el.child(this.revealNote(t)))
        .child(this.bar(t, cx));
    }
    return root.child(this.conversationPanel(t, cx)).child(this.bar(t, cx));
  }

  // The pane grows with the text, up to six lines; the window holds the height (lead.fit).
  fit(cx) {
    if (!this.terminal || this.ask) return;
    if (this.terminalShown() !== true) return;
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
    if (this.needsFirstPick() || this.leadLive()) return this.compose(t, cx);
    return this.empty(t);
  }
}
