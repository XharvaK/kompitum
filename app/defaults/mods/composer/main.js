// The composer under the lead's terminal: one box that sends only what the
// Owner typed, when they press Enter or click Send. Files and folders dropped
// on it, and images pasted into it, wait as chips above the text.
//
// While Claude Code asks (a permission, a plan or a question) its card takes
// the box's place, and the Owner's click or key answers the hook.

import { View } from "gpui-kit";
import { v_flex, TextareaState, VirtualListScrollHandle } from "gpui-base";
import { kom, run, theme, props, shared, setShared, terminals, knownAgents, appHome, readTextFile, writeTextFile, pickFolder, lead as leadOf, nextCommand } from "kompitum";
import { errorText, CARD_WAIT_MS, COMPACT_FIRST_TOKENS } from "./util.js";
import cards from "./cards.js";
import model, { leadUsage } from "./model.js";
import conversation from "./conversation.js";
import box from "./box.js";

const HISTORY_KEY = "composer.history";

const HISTORY_LIMIT = 100;

// The box's height without its text, one line of text, and the chips' row (window.rs sizes the pane).
const BOX_PX = 76;

const LINE_PX = 20;

const CHIPS_PX = 28;

// The pane's height while the lead asks before it starts: the one row of startingRow (window.rs clamps to 44 to 320).
const STARTING_PX = 44;

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
    this.askedFolder = false;
    this.history = loadHistory();
    this.historyAt = -1;
    this.draft = "";
    this.draftShared = null;
    this.usageOpen = false;
    this.askAt = null;
    // The lead's state and its open ask, from Claude Code's hooks.
    this.state = null;
    this.ask = null;
    this.askId = null;
    // When the open card appeared (0 with none): its Enter and digit keys wait for it.
    this.askSince = 0;
    this.noteOpen = false;
    this.noteInput = null;
    this.noteAction = null;
    this.answers = {};
    this.modelOpen = false;
    // Compact first: the pill's state, and the send that waits for the lead's compaction (`at` in ms, `started` once it runs).
    this.compactFirst = false;
    this.compactThenSend = null;
    this.effortSlider = null;
    this.effortSliderMax = null;
    this.effortPreview = null;
    this.effortTouched = false;
    this.saveEffortOnRelease = true;
    this.firstModel = null;
    this.items = [];
    this.seenVersion = null;
    this.stickToBottom = true;
    // The conversation's scroll position, held here so it can be moved to the end. Made in init, never in render.
    this.scroll = VirtualListScrollHandle.new();
    this.openTools = {};
    this.openToolGroups = {};
    this.handle = cx.focus_handle();
    this.text = TextareaState.new({});
    this.text.set_auto_grow(1, 6);
    this.text.set_submit_on_enter(true);
    this.text.on("change", (_event, cx) => {
      if (this.text.value().length === 0) this.historyAt = -1;
      this.syncDraft();
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
    this.syncDraft();
    cx.spawn(async (cx) => {
      await this.load(cx);
      cx.notify();
    });
    cx.timer.every(5000, (cx) => {
      this.load(cx);
    });
    cx.timer.every(500, (cx) => {
      this.takeAsk(cx);
      // A send waiting for the lead's compaction is noticed here.
      if (this.compactThenSend) cx.notify();
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
      this.askFolder(cx);
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

  leadReady() {
    return Boolean(leadOf()?.started);
  }

  // The lead has no folder yet: ask for one once. A cancelled pick saves nothing and starts nothing.
  askFolder(cx) {
    if (this.askedFolder || this.person?.lead?.folder) return;
    this.askedFolder = true;
    cx.spawn(async (cx) => {
      try {
        const folder = await pickFolder({ prompt: "Choose the folder the lead works in" });
        if (!folder) return;
        await this.writeLead({ folder });
        run("lead.open");
      } catch (error) {
        this.error = `Couldn't choose the lead's folder: ${String(error?.message ?? error)}`;
      }
      cx.notify();
    });
  }

  needsFirstPick() {
    if (this.leadLive()) return false;
    if (this.person?.lead?.model) return false;
    return (this.leadProgram().models ?? []).length > 0;
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
    this.syncDraft();
    this.setHistory(true, cx);
    this.text.focus();
    cx.notify();
  }

  // The box holds text or a file that the Owner has not sent.
  hasDraft() {
    return this.text.value().length > 0 || this.chips.length > 0;
  }

  // Enter answers a card only once it has shown for a moment, and while the box holds no draft.
  enterArmed() {
    return !this.hasDraft() && Date.now() - this.askSince >= CARD_WAIT_MS;
  }

  // Other code reads the shared flag "lead.draft": true while the box holds text or a file. It is set only on a change.
  syncDraft() {
    const waiting = this.hasDraft();
    if (waiting === this.draftShared) return;
    this.draftShared = waiting;
    setShared("lead.draft", waiting);
  }

  escape(cx) {
    if (this.state === "asking") {
      if (this.ask) this.cardEscape(this.ask, cx);
      return;
    }
    if (this.text.value().length === 0 && this.leadLive()) this.stopTurn(cx);
  }

  // The keys reach the card while one shows. History's Up and Down are bound
  // on the textarea's own ComposerHistory context instead, so they outrank the
  // editor's caret keys only while the box is empty or browsing history.
  keyDown(event, cx) {
    if (this.state === "asking" && this.ask) this.cardKey(event, cx);
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
    if (this.compactThenSend) return;
    if (this.sending) return;
    if (this.state === "asking") return;
    if (leadOf()?.compacting) return;
    if (!this.leadReady()) return;
    const text = this.text.value();
    const paths = this.chips.map((chip) => chip.path);
    if (!text.trim() && paths.length === 0) return;
    if (!this.leadLive()) {
      this.error = "The lead isn't running";
      cx.notify();
      return;
    }
    // Compact first: the lead compacts now and the text stays in the box; send runs again once it has tidied up.
    if (this.compactFirst && this.state === "waiting") {
      this.compactFirst = false;
      this.compactThenSend = { at: Date.now(), started: false };
      run("lead.compact", { terminal: this.terminal });
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
        this.syncDraft();
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

  // Compact first, settled at each draw: the send waits for the lead's compaction (started, then sent), gives up after
  // 15 s with the text kept, and the pill goes when the box empties or the context drops under the threshold.
  settleCompactFirst(cx) {
    const wait = this.compactThenSend;
    if (wait) {
      if (leadOf()?.compacting === true) {
        wait.started = true;
      } else if (wait.started) {
        this.compactThenSend = null;
        cx.timer.after(0, (cx) => this.send(cx));
      } else if (Date.now() - wait.at > 15000) {
        this.compactThenSend = null;
        this.error = "Couldn't tidy up first. Your message is still here.";
      }
    }
    if (this.compactFirst && (!this.hasDraft() || !((leadUsage()?.context_tokens ?? 0) >= COMPACT_FIRST_TOKENS))) {
      this.compactFirst = false;
    }
  }

  compose(t, cx) {
    const lead = leadOf();
    this.state = lead?.state ?? null;
    this.ask = lead?.ask ?? null;
    const askId = this.ask?.id ?? null;
    if (askId !== this.askId) {
      this.askId = askId;
      this.askSince = askId ? Date.now() : 0;
      this.noteOpen = false;
      this.noteInput = null;
      this.noteAction = null;
      this.answers = {};
      if (askId) cx.timer.after(0, () => this.handle.focus());
      // A card takes the box's place, so the box grows and the list above it shrinks: keep the newest item in view.
      if (this.stickToBottom) {
        cx.timer.after(0, (cx) => { this.scroll.scroll_to_end(); cx.notify(); });
        cx.timer.after(120, (cx) => { this.scroll.scroll_to_end(); cx.notify(); });
      }
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
    this.settleCompactFirst(cx);
    if (this.needsFirstPick()) {
      return root.child(this.bar(t, cx));
    }
    if (this.terminalShown()) {
      if (!this.leadReady()) return root.child(this.startingRow(t));
      return root
        .when(Boolean(this.revealNote(t)), (el) => el.child(this.revealNote(t)))
        .child(this.bar(t, cx));
    }
    if (this.items.length === 0 && this.leadReady() && !this.ask) return root.child(this.startView(t, cx));
    return root.child(this.conversationPanel(t, cx)).child(this.bar(t, cx, true));
  }

  // The pane grows with the text, up to six lines; the window holds the height (lead.fit).
  fit(cx) {
    if (!this.terminal || this.ask) return;
    if (this.terminalShown() !== true) return;
    // Before the lead starts, its terminal may be asking something: the pane is one row for that.
    let px = STARTING_PX;
    if (this.leadReady()) {
      const perLine = Math.max(12, Math.floor(((this.width ?? 640) - 46) / 7.3));
      const lines = String(this.text?.value?.() ?? "")
        .split(/\r?\n/)
        .reduce((n, line) => n + Math.max(1, Math.ceil(line.length / perLine)), 0);
      px = BOX_PX + Math.min(6, Math.max(1, lines)) * LINE_PX + (this.chips.length ? CHIPS_PX : 0);
    }
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

Object.assign(Composer.prototype, cards, model, conversation, box);
