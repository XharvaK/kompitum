// App-owned settings mod. Writes changes immediately; no Save step.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Scrollbar, InputState, Input, TextareaState } from "gpui-base";
import { kom, run, theme, updates, shared, appHome, readTextFile, writeTextFile, playSound, props, closeOverlay, window, setShared, sheet, text, notify } from "kompitum";
import { loadVault, vaultSection } from "./vault.js";
import { loadMemory, memoryPage, saveMemory, forgetMemory } from "./memory.js";
import { connectPage, initConnect, loadConnect } from "./connect.js";
import { agentsPage, loadAgents, ensureAgents } from "./agents.js";
import { appearancePage } from "./appearance.js";
import { dropdown } from "./ui.js";
import { exportSettings, importSettings, importSheet } from "./backup.js";
import { copyDiagnostics } from "./diagnostics.js";

const I = (name) => `frame/settings/${name}.svg`;

const SECTIONS = [
  { id: "appearance", icon: "palette" },
  { id: "agents", icon: "bot" },
  { id: "vault", icon: "mail" },
  { id: "memory", icon: "memory" },
  { id: "connect", icon: "link" },
  { id: "notifications", icon: "bell" },
  { id: "keyboard", icon: "keyboard" },
  { id: "about", icon: "info" },
];

// A section's word, read while drawing so a language change is seen.
function sectionTitle(id) {
  if (id === "appearance") return text("settings.section-appearance");
  if (id === "agents") return text("settings.section-agents");
  if (id === "vault") return text("settings.section-vault");
  if (id === "memory") return text("settings.section-memory");
  if (id === "connect") return text("settings.section-connect");
  if (id === "notifications") return text("settings.section-notifications");
  if (id === "keyboard") return text("settings.section-keyboard");
  if (id === "about") return text("settings.section-about");
  return "";
}

const macos = () => window()?.platform === "macos";
const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

// Keyboard holds only macOS's Option key, so other systems never show it.
function liveSections() {
  return macos() ? SECTIONS : SECTIONS.filter((s) => s.id !== "keyboard");
}

// What the search field finds: one entry per row the sections draw, so search
// never reads the drawn tree. `key` matches the row's highlight key.
const SEARCH = [
  { section: "appearance", key: "light-dark", title: () => text("settings.light-dark"), words: "light dark system appearance" },
  { section: "appearance", key: "theme", title: () => text("settings.theme"), words: "theme colors look" },
  { section: "appearance", key: "glass", title: () => text("settings.sidebar-glass"), words: "transparency blur mica desktop sidebar", only: "windows" },
  { section: "appearance", key: "motion", title: () => text("settings.reduce-motion"), words: "animation spinners still calm accessibility" },
  { section: "appearance", key: "layout", title: () => text("settings.window"), words: "layout reset desk window shape back" },
  { section: "appearance", key: "zoom", title: () => text("settings.zoom"), words: "size scale bigger smaller interface" },
  { section: "appearance", key: "font", title: () => text("settings.terminal-font"), words: "typeface monospace ligatures" },
  { section: "appearance", key: "ui_font", title: () => text("settings.interface-font"), words: "typeface panels" },
  { section: "agents", key: "lead", title: () => text("settings.cli"), words: "lead agent program claude command code opencode grok" },
  { section: "agents", key: "lead_model", title: () => text("settings.model"), words: "provider models list other" },
  { section: "agents", key: "lead_model", title: () => text("settings.reasoning"), words: "effort levels low medium high xhigh max" },
  { section: "agents", key: "folder", title: () => text("settings.folder"), words: "lead folder project directory picker" },
  { section: "agents", key: "workers", title: () => text("settings.workers"), words: "order add remove drag subagents claude sonnet haiku" },
  { section: "agents", key: "clis", title: () => text("settings.cli-versions"), words: "update version releases" },
  { section: "vault", key: "Folder", title: () => text("settings.vault-folder"), words: "notes git mailbox" },
  { section: "vault", key: "Needs you", title: () => text("settings.notes-need-you"), words: "mailbox notes requests questions" },
  { section: "memory", key: "memory", title: () => text("settings.memory"), words: "remember forget notes claude lead" },
  { section: "connect", key: "connect", title: () => text("settings.connect"), words: "agent systems join prompt grok hermes openclaw manager machines" },
  { section: "notifications", key: "sounds", title: () => text("settings.play-sounds"), words: "sound audio notification" },
  { section: "notifications", key: "blocked", title: () => text("settings.agent-needs-you"), words: "sound blocked attention" },
  { section: "notifications", key: "done", title: () => text("settings.agent-finished"), words: "sound done finished" },
  { section: "notifications", key: "Notes between agents", title: () => text("settings.notes-between-agents"), words: "pop-ups toast vault mail" },
  { section: "notifications", key: "Quiet while a game is fullscreen", title: () => text("settings.quiet-fullscreen"), words: "game fullscreen mute do not disturb" },
  { section: "notifications", key: "Windows notifications", title: () => text("settings.windows-notifications"), words: "system toast needs you in front", only: "windows" },
  { section: "notifications", key: "Summon Kompitum", title: () => text("settings.summon-kompitum"), words: "keyboard shortcut hotkey bring forward composer", only: "windows" },
  { section: "connect", key: "this", title: () => text("settings.this-computer"), words: "local version kom machines" },
  { section: "connect", key: "add", title: () => text("settings.add-machine"), words: "ssh remote host machines" },
  { section: "keyboard", key: "option", title: () => text("settings.option-key"), words: "alt macos accents keyboard" },
  { section: "about", key: "welcome", title: () => text("settings.welcome"), words: "show again onboarding first start setup" },
  { section: "about", key: "app", title: () => text("settings.kompitum"), words: "version update what's new release" },
  { section: "about", key: "core", title: () => text("settings.kompitum-core"), words: "version update core terminals" },
];

// Sections that were named differently or merged before; old links still open their page.
const RENAMED_SECTIONS = { look: "appearance", alerts: "notifications", general: "agents", machines: "connect" };

export default class Settings extends View {
  init(_props, cx) {
    const asked = props()?.section;
    const wanted = RENAMED_SECTIONS[asked] ?? asked;
    this.section = SECTIONS.some((s) => s.id === wanted) ? wanted : "appearance";
    this.picking = null; // "font" | "ui_font" while its list is open
    ensureAgents(this);
    this.filter = "";
    this.search = "";
    this.highlight = null; // { section, key } while a search result's row is lit
    this.advancedOpen = {}; // which pages' Advanced folds the person opened
    this.searchInput = InputState.new({ placeholder: text("settings.search-ask-placeholder") });
    this.searchInput.on("change", (_e, cx) => { this.search = this.searchInput.value(); cx.notify(); });
    this.searchInput.on("submit", (_e, cx) => {
      const first = this.searchMatches()[0];
      if (first) this.openMatch(first, cx);
    });
    this.fontInput = InputState.new({ placeholder: text("settings.font-search-placeholder") });
    this.fontInput.on("change", (_e, cx) => { this.filter = this.fontInput.value(); cx.notify(); });
    this.renameInput = InputState.new();
    this.renameInput.on("focus", (_e, cx) => { this.renameFocused = true; cx.notify(); });
    this.renameInput.on("blur", (_e, cx) => { this.renameFocused = false; cx.notify(); });
    this.renameInput.on("change", (_e, cx) => { if (this.machineEdit?.action === "rename") this.machineEdit.name = this.renameInput.value(); cx.notify(); });
    this.renameInput.on("submit", (_e, cx) => {
      const edit = this.machineEdit;
      if (edit?.action === "rename") this.machineAction(edit.machine, "rename", cx, edit.name);
    });
    this.person = {};
    this.raw = null;
    this.machineList = null;
    this.machineLoading = false;
    this.machineError = null;
    this.machineBusy = {};
    this.machineResults = {};
    this.machineEdit = null;
    this.machineNotice = null;
    this.vault = null;
    this.vaultError = null;
    this.vaultLoading = false;
    this.memory = null;
    this.memoryError = null;
    this.memoryLoading = false;
    this.memoryOpenPath = null;
    this.memoryEdit = null;
    this.memoryForgetPath = null;
    this.memoryWhyText = "";
    this.memoryRowError = null;
    this.memoryForgottenOpen = false;
    this.memoryTitle = InputState.new({ placeholder: text("settings.title-placeholder") });
    this.memoryTitle.on("change", (_e, cx) => {
      if (this.memoryEdit) this.memoryEdit.title = this.memoryTitle.value();
      cx.notify();
    });
    this.memoryBody = TextareaState.new({ rows: 6 });
    this.memoryBody.on("change", (_e, cx) => {
      if (this.memoryEdit) this.memoryEdit.body = this.memoryBody.value();
      cx.notify();
    });
    this.memoryWhy = InputState.new({ placeholder: text("settings.why-placeholder") });
    this.memoryWhy.on("change", (_e, cx) => {
      this.memoryWhyText = this.memoryWhy.value();
      cx.notify();
    });
    this.memorySave = (cx) => saveMemory(this, cx);
    this.memoryForget = (cx) => forgetMemory(this, cx);
    this.summonKeyMenu = {};
    this.summonKeyPick = (value, cx) => this.change((p) => { p.summon_key = value; }, cx);
    this.exportNote = null;
    this.importNote = null;
    this.importError = null;
    this.importSheet = null;
    this.copiedAt = null;
    this.saveError = null;
    this.diagnosticError = null;
    this.openExport = (cx) => exportSettings(this, cx);
    this.openImport = (cx) => importSettings(this, cx);
    this.copyReport = (cx) => copyDiagnostics(this, cx);
    this.handle = cx.focus_handle();
    this.handle.focus();
    cx.spawn(async (cx) => {
      await this.load();
      if (this.section === "connect") await this.loadMachines();
      if (this.section === "vault") loadVault(this, cx);
      if (this.section === "memory") loadMemory(this, cx);
      if (this.section === "agents") loadAgents(this, cx);
      cx.notify();
    });
    // While the Agents page is open, its usage, workers and lead stay current.
    cx.timer.every(5000, (cx) => {
      if (this.section !== "agents") return;
      loadAgents(this, cx);
    });
    // After "Check now": follow the look (the answer comes from the app), then
    // "Up to date" for a moment.
    cx.timer.every(300, (cx) => {
      if (this.copiedAt && Date.now() - this.copiedAt >= 2000) {
        this.copiedAt = null;
        cx.notify();
      }
      if (!this.asked) return;
      if (updates().checking) this.asked = Date.now();
      else if (Date.now() - this.asked >= 3000) this.asked = null;
      cx.notify();
    });
    cx.timer.every(3000, (cx) => {
      if (this.section !== "connect") return;
      cx.spawn(async (cx) => {
        await this.loadMachines();
        cx.notify();
      });
    });
    // "Up to date · 2 min ago" keeps its distance from now while the page is open.
    cx.timer.every(30000, (cx) => {
      if (this.section === "vault" && this.vault && !this.vaultLoading) cx.notify();
    });
    initConnect(this, cx);
  }

  file() {
    return `${appHome()}/settings.json`;
  }

  async load() {
    this.raw = await readTextFile(this.file());
    try {
      this.person = this.raw ? JSON.parse(this.raw) : {};
    } catch {
      this.person = {};
    }
  }

  // Changes settings.json the way an agent would: read, change, write back
  // only if nobody changed it meanwhile (else read again and redo).
  change(edit, cx, after = null) {
    cx.spawn(async (cx) => {
      let saved = false;
      let reason = null;
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          await this.load();
          const next = JSON.parse(JSON.stringify(this.person));
          edit(next);
          await writeTextFile(this.file(), JSON.stringify(next, null, 2) + "\n", this.raw);
          this.person = next;
          saved = true;
          break;
        } catch (error) {
          reason = String(error?.message ?? error);
        }
      }
      if (saved) {
        this.saveError = null;
        if (after) after(cx);
      } else {
        this.saveError = reason;
        notify({ title: text("settings.save-failed", { reason }), level: "error" });
      }
      cx.notify();
    });
  }

  close() {
    closeOverlay(props().overlay);
  }

  // Search ---------------------------------------------------------------

  searchMatches() {
    const words = this.search.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return [];
    const live = new Set(liveSections().map((s) => s.id));
    return SEARCH.filter((e) => live.has(e.section)).filter((e) => {
      if (e.only && window()?.platform !== e.only) return false;
      const line = `${e.title()} ${e.words}`.toLowerCase();
      return words.every((w) => line.includes(w));
    });
  }

  clearSearch(cx) {
    this.search = "";
    this.searchInput.set_value("");
    this.searchInput.focus();
    cx.notify();
  }

  // No setting matches: hand the words to the lead's composer, unsent.
  askLead(cx) {
    const text = this.search.trim();
    if (!text) return;
    setShared("composer.ask", { text, at: Date.now() });
    closeOverlay(props().overlay);
    run("lead.open");
  }

  openMatch(match, cx) {
    this.section = match.section;
    this.picking = null;
    this.machineEdit = null;
    const at = Date.now();
    this.highlight = { section: match.section, key: match.key, at };
    this.search = "";
    this.searchInput.set_value("");
    this.handle.focus();
    if (match.section === "connect") cx.spawn(async (cx) => { await this.loadMachines(); cx.notify(); });
    if (match.section === "agents") loadAgents(this, cx, true);
    if (match.section === "vault") loadVault(this, cx);
    if (match.section === "memory") loadMemory(this, cx);
    if (match.section === "connect" && this.connect.vault === null) loadConnect(this, cx);
    cx.timer.after(1500, (cx) => {
      if (this.highlight?.at === at) {
        this.highlight = null;
        cx.notify();
      }
    });
    cx.notify();
  }

  lit(key) {
    return Boolean(this.highlight) && this.highlight.section === this.sectionNow() && this.highlight.key === key;
  }

  // Opening Keyboard on a system without it lands on Agents.
  sectionNow() {
    return this.section === "keyboard" && !macos() ? "agents" : this.section;
  }

  // Pieces ---------------------------------------------------------------

  heading(t, text) {
    return div().pt(6).pb(8).text_size(13).font_semibold().text_color(t.dim).child(text);
  }

  // One setting: what it is on the left, the control on the right. `key` names
  // the row for the search field's highlight (default: its title).
  row(t, title, about, control, key = title) {
    const lit = this.lit(key);
    return h_flex()
      .w_full()
      .gap(16)
      .py(12)
      .items_center()
      .border_b(1)
      .border_color(t.border)
      .when(lit, (el) => el.bg(t.selected).rounded(7))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(3)
          .when(Boolean(title), (el) => el.child(div().text_size(14).text_color(t.foreground).child(title)))
          .when(Boolean(about), (el) => el.child(div().text_size(13).line_height("18px").text_color(t.dim).child(about))),
      )
      .child(div().flex_none().child(control));
  }

  // The rarely used rows of a page, folded under "Advanced". The fold opens by
  // itself while a search result for one of its rows is lit (`keys`).
  advanced(t, key, rows, keys) {
    const hit = Boolean(this.highlight) && this.highlight.section === this.sectionNow() && keys.includes(this.highlight.key);
    const open = Boolean(this.advancedOpen[key]) || hit;
    return v_flex()
      .w_full()
      .child(
        h_flex()
          .id(`st-adv-${key}`)
          .w_full()
          .py(12)
          .gap(6)
          .items_center()
          .cursor_pointer()
          .hover((s) => s.text_color(t.foreground))
          .tab_stop(true)
          .focus_ring(t.focus, 7)
          .on_click((_e, cx) => {
            this.advancedOpen[key] = !this.advancedOpen[key];
            cx.notify();
          })
          .child(div().text_size(14).text_color(t.muted).hover((s) => s.text_color(t.foreground)).child(text("settings.advanced")))
          .child(div().text_size(14).text_color(t.muted).hover((s) => s.text_color(t.foreground)).child(open ? "⌄" : "›")),
      )
      .when(open, (el) => el.child(v_flex().w_full().children(rows)));
  }

  button(t, id, label, onClick) {
    return div()
      .id(id)
      .px(12)
      .py(5)
      .rounded(7)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .cursor_pointer()
      .text_size(14)
      .text_color(t.foreground)
      .hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
      .tab_stop(true)
      .focus_ring(t.focus, 7)
      .on_click(onClick)
      .child(label);
  }

  // A quiet link for an action that must not compete with the page's controls.
  textButton(t, id, label, onClick) {
    return div()
      .id(id)
      .px(6)
      .py(4)
      .rounded(6)
      .cursor_pointer()
      .text_size(14)
      .text_color(t.muted)
      .hover((s) => s.text_color(t.foreground))
      .tab_stop(true)
      .focus_ring(t.focus, 6)
      .on_click(onClick)
      .child(label);
  }

  // One primary action, filled the way the theme fills an on switch.
  primaryButton(t, id, label, onClick) {
    return div()
      .id(id)
      .px(12)
      .py(5)
      .rounded(7)
      .cursor_pointer()
      .bg(t.accent_fill)
      .text_color(t.accent_foreground)
      .text_size(14)
      .font_semibold()
      .relative()
      .tab_stop(true)
      .focus_ring(t.focus, 7)
      .on_click(onClick)
      .child(div().absolute().inset_0().rounded(7).hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected)))
      .child(label);
  }

  toggle(t, id, on, onClick) {
    return div()
      .id(id)
      .w(36)
      .h(20)
      .rounded(10)
      .p(2)
      .cursor_pointer()
      .bg(on ? t.accent_fill : t.hover)
      .flex()
      .when(on, (el) => el.justify_end())
      .tab_stop(true)
      .focus_ring(t.focus, 10)
      .on_click(onClick)
      .child(div().w(16).h(16).rounded(8).bg(on ? t.accent_foreground : t.muted));
  }

  // Look -----------------------------------------------------------------

  keyboard(t) {
    const choices = [
      ["auto", text("settings.option-auto")],
      ["on", text("settings.option-alt")],
      ["off", text("settings.option-types-characters")],
      ["left", text("settings.option-left-alt")],
      ["right", text("settings.option-right-alt")],
    ];
    return v_flex()
      .w_full()
      .child(this.heading(t, text("settings.option-key")))
      .when(window()?.platform === "macos", (el) =>
        el.child(div().pb(12).text_size(14).text_color(t.dim).child(text("settings.option-about"))),
      )
      .when(window()?.platform !== "macos", (el) =>
        el.child(div().pb(12).text_size(13).line_height("18px").text_color(t.dim).child(text("settings.option-mac-only"))),
      )
      .when(window()?.platform === "macos", (el) =>
        el.children(choices.map(([value, label]) => {
        const on = (this.person.option_as_alt ?? "auto") === value;
        return h_flex()
          .id(`st-option-${value}`)
          .gap(10)
          .px(12)
          .py(10)
          .rounded(7)
          .items_center()
          .cursor_pointer()
          .when(this.lit("option"), (el) => el.bg(t.selected))
          .hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))
          .tab_stop(true)
          .focus_ring(t.focus, 7)
          .on_click((_e, cx) => this.change((p) => { p.option_as_alt = value; }, cx))
          .child(div().text_color(on ? t.accent : t.muted).child(on ? "◉" : "○"))
          .child(div().text_size(14).text_color(t.foreground).child(label));
        })));
  }

  // Machines -------------------------------------------------------------

  async loadMachines() {
    if (this.machineLoading) return;
    this.machineLoading = true;
    try {
      const r = await kom(["machine", "ls", "--json"]);
      if (r.code !== 0) throw new Error(r.stderr || r.stdout || text("settings.machines-read-failed"));
      const list = JSON.parse(r.stdout).machines;
      if (!Array.isArray(list)) throw new Error(text("settings.machines-read-failed"));
      this.machineList = list.filter((m) => !m.temporary);
      this.machineError = null;
    } catch (e) {
      this.machineError = String(e?.message ?? e);
    } finally {
      this.machineLoading = false;
    }
  }

  machineAction(machine, action, cx, name) {
    if (this.machineBusy[machine]) return;
    if (action === "rename" && (!name?.trim() || name.trim() === machine)) return;
    this.machineBusy[machine] = action;
    delete this.machineResults[machine];
    this.machineEdit = null;
    this.handle.focus();
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const args = ["machine", action, machine];
        if (action === "rename") args.push(name.trim());
        const r = await kom(args);
        let payload = null;
        try { payload = JSON.parse(r.stdout || r.stderr); } catch {}
        if (r.code !== 0) throw new Error(payload?.error?.message || (r.stderr || r.stdout || text("settings.it-didnt-work")).trim());
        const message = action === "update"
          ? payload?.version
            ? text("settings.machine-updated-version", { machine, version: payload.version })
            : text("settings.machine-updated", { machine })
          : action === "rename"
            ? text("settings.machine-renamed", { machine, name: name.trim() })
            : text("settings.machine-removed", { machine });
        if (action === "update") this.machineResults[machine] = { message, ok: true };
        else this.machineNotice = message;
      } catch (e) {
        this.machineResults[machine] = { message: String(e?.message ?? e), ok: false };
      } finally {
        delete this.machineBusy[machine];
      }
      await this.loadMachines();
      cx.notify();
    });
  }

  olderCore(remote, local) {
    const parse = (v) => /^(\d+)\.(\d+)\.(\d+)(?:-([^+]+))?(?:\+.*)?$/.exec(v ?? "");
    const a = parse(remote), b = parse(local);
    if (!a || !b) return false;
    for (let i = 1; i <= 3; i++) {
      if (+a[i] !== +b[i]) return +a[i] < +b[i];
    }
    if (!a[4] || !b[4]) return Boolean(a[4] && !b[4]);
    const x = a[4].split("."), y = b[4].split(".");
    for (let i = 0; i < Math.max(x.length, y.length); i++) {
      if (x[i] === y[i]) continue;
      if (x[i] === undefined || y[i] === undefined) return x[i] === undefined;
      const xn = /^\d+$/.test(x[i]), yn = /^\d+$/.test(y[i]);
      return xn && yn ? +x[i] < +y[i] : xn !== yn ? xn : x[i] < y[i];
    }
    return false;
  }

  machineRow(t, m, localVersion) {
    const busy = this.machineBusy[m.name];
    const edit = this.machineEdit?.machine === m.name ? this.machineEdit : null;
    const state = String(m.state ?? "unknown").replace(/[_-]/g, " ");
    const color = ["connected", "online"].includes(m.state) ? t.ansi2 : ["connecting", "reconnecting"].includes(m.state) ? t.working : t.blocked;
    const result = this.machineResults[m.name];
    const controls = h_flex().gap(6).items_center();
    if (busy) controls.child(svg("mods/workspace-chrome/spinner.svg").size(12).text_color(t.working).spin(900)).child(div().text_size(13).text_color(t.muted).child(busy === "update" ? text("settings.updating") : busy === "rename" ? text("settings.renaming") : text("settings.removing")));
    else {
      if (this.olderCore(m.version, localVersion)) controls.child(this.button(t, `st-m-update-${m.name}`, text("settings.update"), (_e, cx) => this.machineAction(m.name, "update", cx)));
      for (const [action, label] of [["rename", text("settings.rename")], ["rm", text("settings.remove")]]) {
        controls.child(this.button(t, `st-m-${action}-${m.name}`, label, (_e, cx) => {
          this.machineEdit = { machine: m.name, action, name: m.name };
          if (action === "rename") {
            this.renameInput.set_value(m.name);
            this.renameInput.set_selected_range({ start: 0, end: encodeURIComponent(m.name).replace(/%[0-9A-F]{2}/g, "_").length });
            this.renameFocused = true;
            this.renameInput.focus();
          }
          cx.notify();
        }));
      }
    }
    const row = v_flex().w_full().py(12).gap(8).border_b(1).border_color(t.border)
      .child(h_flex().gap(10).items_center()
        .child(div().w(7).h(7).flex_none().rounded(4).bg(color))
        .child(v_flex().flex_1().min_w(0).gap(3)
          .child(div().text_size(14).text_color(t.foreground).child(m.name))
          .child(div().text_size(13).text_color(t.dim).child(m.version
            ? text("settings.machine-line", { target: m.target ?? "", state, version: m.version })
            : text("settings.machine-line-no-version", { target: m.target ?? "", state }))))
        .child(controls.flex_none()));
    if (busy === "update") row.child(div().text_size(13).text_color(t.dim).child(text("settings.copying-core")));
    if (edit && !busy) {
      const cancel = this.button(t, `st-m-cancel-${m.name}`, text("settings.cancel"), (_e, cx) => { this.machineEdit = null; this.handle.focus(); cx.notify(); });
      if (edit.action === "rm") row.child(v_flex().gap(8)
        .child(div().text_size(13).text_color(t.muted).child(text("settings.remove-machine-confirm", { machine: m.name })))
        .child(h_flex().gap(6).child(cancel).child(this.button(t, `st-m-confirm-${m.name}`, text("settings.remove-machine"), (_e, cx) => this.machineAction(m.name, "rm", cx)))));
      else row.child(v_flex().gap(6)
        .child(div().text_size(13).text_color(t.dim).child(text("settings.new-name-hint")))
        .child(h_flex().gap(6).items_center()
          .child(Input.new(this.renameInput).flex_1().min_w(0).h(32).px(10).rounded(7).border_1().border_color(this.renameFocused ? t.accent : t.input).bg(t.background).text_size(14).text_color(t.foreground))
          .child(cancel).child(this.button(t, `st-m-save-${m.name}`, text("settings.save"), (_e, cx) => this.machineAction(m.name, "rename", cx, edit.name)))));
    }
    if (result) row.child(div().text_size(13).text_color(result.ok ? t.ansi2 : t.blocked).tooltip(result.ok ? null : result.message).child(result.ok ? result.message : firstLine(result.message) || result.message));
    return row;
  }

  machines(t) {
    const version = updates().core.version;
    return v_flex().w_full()
      .child(this.heading(t, text("settings.heading-machines")))
      .child(div().pb(8).text_size(13).line_height("18px").text_color(t.dim).child(text("settings.machines-about")))
      .child(this.row(t, text("settings.this-computer"), text("settings.connected"), div().w(7).h(7).rounded(4).bg(t.ansi2), "this"))
      .children((this.machineList ?? []).map((m) => this.machineRow(t, m, version)))
      .when(this.machineList === null && !this.machineError, (el) => el.child(h_flex().py(14).gap(8).items_center().child(svg("mods/workspace-chrome/spinner.svg").size(12).text_color(t.working).spin(900)).child(div().text_size(13).text_color(t.dim).child(text("settings.reading-machines")))))
      .when(this.machineList?.length === 0, (el) => el.child(div().py(14).text_size(13).text_color(t.dim).child(text("settings.no-machines"))))
      .when(Boolean(this.machineError), (el) => el.child(div().py(10).text_size(13).text_color(t.blocked).tooltip(this.machineError).child(firstLine(this.machineError) || this.machineError)))
      .when(Boolean(this.machineNotice), (el) => el.child(div().py(10).text_size(13).text_color(t.ansi2).child(this.machineNotice)))
      .child(this.row(t, text("settings.add-machine"), text("settings.add-machine-about"), this.button(t, "st-m-add", text("settings.add"), () => run("mod.open", { name: "machine-add" })), "add"));
  }

  // Alerts ---------------------------------------------------------------

  alerts(t) {
    const sounds = this.person.sounds ?? {};
    const on = sounds.enabled !== false;
    const between = this.person.notes_between_agents !== false;
    const quiet = this.person.quiet_while_fullscreen !== false;
    const notifications = this.person.system_notifications !== false;
    const summon = this.person.summon_key ?? "ctrl+alt+k";
    const taken = Boolean(window()?.summon_taken);
    const file = (f) => (f ? f.split(/[\\/]/).pop() : text("settings.kompitum-sound"));
    const summonMenu = dropdown(t, "st-summon-key", {
      items: [
        { value: "ctrl+alt+k", label: "Ctrl+Alt+K" },
        { value: "ctrl+alt+space", label: "Ctrl+Alt+Space" },
        { value: "off", label: text("settings.off") },
      ],
      value: summon,
      onPick: (value, cx) => this.summonKeyPick(value, cx),
      state: this.summonKeyMenu,
      width: 200,
    });
    const summonAbout = taken
      ? div().text_size(13).line_height("18px").text_color(t.blocked).child(text("settings.summon-taken"))
      : text("settings.summon-about");
    return v_flex()
      .w_full()
      .child(this.heading(t, text("settings.heading-sounds")))
      .child(
        this.row(
          t,
          text("settings.play-sounds"),
          text("settings.play-sounds-about"),
          this.toggle(t, "st-sounds", on, (_e, cx) => this.change((p) => (p.sounds = { ...(p.sounds ?? {}), enabled: !on }), cx)),
          "sounds",
        ),
      )
      .child(this.row(t, text("settings.agent-needs-you"), file(sounds.blocked), this.button(t, "st-play-blocked", text("settings.play"), () => playSound("blocked")), "blocked"))
      .child(this.row(t, text("settings.agent-finished"), file(sounds.done), this.button(t, "st-play-done", text("settings.play"), () => playSound("done")), "done"))
      .child(div().h(18))
      .child(this.heading(t, text("settings.heading-popups")))
      .child(this.row(t, text("settings.notes-between-agents"), text("settings.notes-between-agents-about"), this.toggle(t, "st-notes-between-agents", between, (_e, cx) => this.change((p) => (p.notes_between_agents = !between), cx))))
      .child(this.row(t, text("settings.quiet-fullscreen"), text("settings.quiet-fullscreen-about"), this.toggle(t, "st-quiet-fullscreen", quiet, (_e, cx) => this.change((p) => (p.quiet_while_fullscreen = !quiet), cx))))
      .when(window()?.platform === "windows", (el) =>
        el
          .child(this.row(t, text("settings.windows-notifications"), text("settings.windows-notifications-about"), this.toggle(t, "st-system-notifications", notifications, (_e, cx) => this.change((p) => (p.system_notifications = !notifications), cx))))
          .child(this.row(t, text("settings.summon-kompitum"), summonAbout, summonMenu)),
      )
      .child(div().pt(12).text_size(13).line_height("18px").text_color(t.dim).child(text("settings.sounds-own")));
  }

  // About ----------------------------------------------------------------

  about(t, cx) {
    if (this.importSheet) return importSheet(this, t, cx);
    const u = updates();
    const at = (secs) => {
      const d = new Date(secs * 1000);
      return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    };
    const checkedAt = u.checked ? at(u.checked) : null;
    const result = u.checking
      ? text("settings.checking")
      : u.checked
        ? checkedAt ? text("settings.up-to-date-checked", { time: checkedAt }) : text("settings.up-to-date")
        : text("settings.no-check");
    // Pressed: "Checking\u2026" while it looks, then "Up to date" for a moment
    // (a find changes the row instead). One per row: an element shows once.
    const asked = this.asked && Date.now() - this.asked < 3000;
    const check = (id) =>
      !u.checks
        ? null
        : u.checking && this.asked
          ? div().px(12).py(5).text_size(14).text_color(t.dim).child(text("settings.checking"))
          : asked && u.checked && !u.checking
            ? div().px(12).py(5).text_size(14).text_color(t.ansi2).child(text("settings.up-to-date-check"))
            : this.button(t, id, text("settings.check-now"), (_e, cx) => {
                this.asked = Date.now();
                run("app.check_updates");
                cx.notify();
              });
    const primary = (id, label, f) =>
      div().id(id).px(12).py(5).rounded(7).cursor_pointer().bg(t.accent_fill).text_color(t.accent_foreground).text_size(14).font_semibold().relative().tab_stop(true).focus_ring(t.focus, 7).on_click(f).child(div().absolute().inset_0().rounded(7).hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))).child(label);
    const source = !u.app.updates;
    const app = source
      ? [null, null]
      : u.app.ready
        ? [text("settings.app-ready", { version: u.app.ready }), primary("st-app-update", text("settings.restart-update"), () => run("app.update"))]
        : [result, check("st-check-app")];
    const c = u.core;
    const core = c.managed
      ? [text("settings.core-managed", { manager: c.managed }), null]
      : c.updating
        ? [text("settings.core-updating"), null]
        : c.error
          ? [text("settings.core-failed", { error: c.error }), primary("st-core-retry", text("settings.try-again"), () => run("app.update_core"))]
          : c.ready
            ? [text("settings.core-ready", { version: c.ready }), primary("st-core-update", text("settings.core-update"), () => run("app.update_core"))]
            : u.checks
              ? [result, check("st-check-core")]
              : [text("settings.built-from-source"), null];
    return v_flex()
      .w_full()
      .child(
        v_flex()
          .w_full()
          .py(12)
          .gap(9)
          .border_b(1)
          .border_color(t.border)
          .child(div().text_size(14).text_color(t.foreground).child(text("settings.version-line", { app: u.app.version, core: c.version ?? "" })))
          .child(div().text_size(13).line_height("18px").text_color(t.dim)
            .child(source ? text("settings.built-from-checkout") : text("settings.update-check")))
          .child(
            h_flex()
              .w_full()
              .gap(8)
              .items_center()
              .flex_wrap()
              .when(this.lit("app"), (el) => el.bg(t.selected).rounded(7))
              .child(div().text_size(13).text_color(t.dim).child(text("settings.app-word")))
              .when(Boolean(app[0]), (el) => el.child(div().flex_1().min_w(0).text_size(13).line_height("18px").text_color(t.muted).child(app[0])))
              .child(div().flex_1())
              .when(Boolean(u.app.version), (el) => el.child(div().text_size(13).text_color(t.muted).child(u.app.version)))
              .child(this.button(t, "st-whats-new", text("settings.whats-new"), () => { closeOverlay(props()?.overlay); run("app.whats_new"); }))
              .when(Boolean(app[1]), (el) => el.child(app[1])),
          )
          .child(
            h_flex()
              .w_full()
              .gap(8)
              .items_center()
              .flex_wrap()
              .when(this.lit("core"), (el) => el.bg(t.selected).rounded(7))
              .child(div().text_size(13).text_color(t.dim).child(text("settings.core-word")))
              .child(div().flex_1().min_w(0).text_size(13).line_height("18px").text_color(t.muted).tooltip(core[0]).child(firstLine(core[0]) || core[0]))
              .when(Boolean(core[1]), (el) => el.child(core[1])),
          ),
      )
      .child(
        this.row(
          t,
          text("settings.welcome"),
          null,
          this.button(t, "st-general-welcome", text("settings.show-again"), () => {
            closeOverlay(props().overlay);
            run("app.welcome");
          }),
          "welcome",
        ),
      )
      .child(
        this.row(
          t,
          text("settings.export-settings"),
          text("settings.export-about"),
          this.button(t, "st-export", text("settings.export-settings"), (_e, cx) => exportSettings(this, cx)),
        ),
      )
      .child(
        this.row(
          t,
          text("settings.import-settings"),
          text("settings.import-about"),
          this.button(t, "st-import", text("settings.import-settings"), (_e, cx) => importSettings(this, cx)),
        ),
      )
      .when(Boolean(this.exportNote), (el) =>
        el.child(div().id("st-export-note").py(6).text_size(13).text_color(t.dim).child(this.exportNote)))
      .when(Boolean(this.importError), (el) =>
        el.child(div().id("st-import-error").py(6).text_size(13).text_color(t.blocked).child(this.importError)))
      .when(Boolean(this.importNote), (el) =>
        el.child(div().id("st-import-note").py(6).text_size(13).text_color(t.dim).child(this.importNote)))
      .child(
        this.row(
          t,
          text("settings.copy-diagnostics"),
          text("settings.copy-diagnostics-about"),
          h_flex()
            .gap(8)
            .items_center()
            .child(this.button(t, "st-copy-diagnostics", text("settings.copy-diagnostics"), (_e, cx) => copyDiagnostics(this, cx)))
            .when(Boolean(this.copiedAt), (el) => el.child(div().id("st-copied").text_size(13).text_color(t.ansi2).child(text("settings.copied")))),
        ),
      )
      .when(Boolean(this.diagnosticError), (el) =>
        el.child(div().id("st-diagnostic-error").py(6).text_size(13).text_color(t.blocked).child(this.diagnosticError)))
      .child(div().pt(12).text_size(13).line_height("18px").text_color(t.dim).child(text("settings.privacy-note")))
      .child(div().text_size(13).line_height("18px").text_color(t.dim).child(text("settings.built-on-herdr")));
  }


  // ----------------------------------------------------------------------

  render(cx) {
    const t = theme();
    const u = updates();
    const section = this.sectionNow();
    const waiting = Boolean(u.app.ready || (u.core.ready && !u.core.managed));
    const query = this.search.trim();
    const matches = query ? this.searchMatches() : [];
    const nav = v_flex()
      .flex_none()
      .w(190)
      .p(10)
      .gap(2)
      .border_r(1)
      .border_color(t.border)
      .child(
        div().px(6).py(4).pb(10).child(
          Input.new(this.searchInput)
            .w_full()
            .h(30)
            .px(9)
            .rounded(7)
            .border_1()
            .border_color(t.input)
            .bg(t.background)
            .text_size(14)
            .text_color(t.foreground),
        ),
      )
      .when(Boolean(query), (el) =>
        el.children(
          matches.length
            ? matches.map((m, i) => {
                const where = sectionTitle(m.section) || m.section;
                // The setting's name over its section: two short lines that truncate fit the column, where one
                // long "Section › Setting" ran out over the page. The tooltip keeps the whole path.
                return v_flex()
                  .id(`st-search-${i}`)
                  .min_w(0)
                  .px(10)
                  .py(6)
                  .gap(2)
                  .rounded(7)
                  .cursor_pointer()
                  .hover((st) => st.bg(t.hover).text_color(t.foreground)).active((st) => st.bg(t.selected))
                  .tab_stop(true)
                  .focus_ring(t.focus, 7)
                  .tooltip(`${where} › ${m.title()}`)
                  .on_click((_e, cx) => this.openMatch(m, cx))
                  .child(div().truncate().text_size(14).text_color(t.foreground).child(m.title()))
                  .child(div().truncate().text_size(12).text_color(t.dim).child(where));
              })
            : [
                div().px(10).py(6).text_size(13).line_height("18px").text_color(t.dim).child(text("settings.no-match", { query })),
                h_flex()
                  .id("st-ask-lead")
                  .px(10)
                  .py(7)
                  .gap(7)
                  .items_center()
                  .rounded(7)
                  .cursor_pointer()
                  .hover((st) => st.bg(t.hover).text_color(t.foreground)).active((st) => st.bg(t.selected))
                  .tab_stop(true)
                  .focus_ring(t.focus, 7)
                  .on_click((_e, cx) => this.askLead(cx))
                  .child(svg(I("bot")).size(14).text_color(t.accent))
                  .child(div().text_size(13).text_color(t.foreground).child(text("settings.ask-lead"))),
              ],
        ),
      )
      .when(!query, (el) => el.children(
        liveSections().map((s) => {
          const on = s.id === section;
          return h_flex()
            .id(`st-s-${s.id}`)
            .gap(10)
            .px(10)
            .py(7)
            .items_center()
            .rounded(7)
            .cursor_pointer()
            .bg(on ? t.selected : "#00000000")
            .hover((st) => st.bg(t.hover).text_color(t.foreground)).active((st) => st.bg(t.selected))
            .tab_stop(true)
            .focus_ring(t.focus, 7)
            .on_click((_e, cx) => {
              this.section = s.id;
              this.picking = null;
              this.machineEdit = null;
              this.highlight = null;
              this.handle.focus();
              if (s.id === "connect") cx.spawn(async (cx) => { await this.loadMachines(); cx.notify(); });
              if (s.id === "vault") loadVault(this, cx);
              if (s.id === "memory") loadMemory(this, cx);
              if (s.id === "connect" && this.connect.vault === null) loadConnect(this, cx);
              if (s.id === "agents") loadAgents(this, cx);
              cx.notify();
            })
            .child(svg(I(s.icon)).size(15).text_color(on ? t.accent : t.dim))
            .child(div().flex_1().text_size(14).text_color(on ? t.foreground : t.muted).child(sectionTitle(s.id)))
            .when(s.id === "about" && waiting, (el) => el.child(div().w(7).h(7).rounded(4).bg(t.accent)));
        }),
      ));
    const content = v_flex()
      .id("settings")
      .track_focus(this.handle)
      .w_full()
      .min_h(0)
      .on_key_down((e, cx) => {
        const k = e.key;
        if (this.machineEdit) {
          if (k === "escape") this.machineEdit = null;
          else return;
          this.handle.focus();
          cx.stop_propagation();
          cx.notify();
          return;
        }
        if (k !== "escape") return;
        if (this.search) this.clearSearch(cx);
        else if (this.picking) this.picking = null;
        // Nothing open inside: Esc reaches the app's one close, on the Overlay key context.
        else return;
        this.handle.focus();
        cx.stop_propagation();
        cx.notify();
      })
      .child(
        h_flex()
          .flex_1()
          .min_h(0)
          .items_start()
          .child(nav.h_full().child(div().flex_1()).child(this.textButton(t, "st-feedback", text("settings.send-feedback"), () => { closeOverlay(props()?.overlay); run("app.feedback"); })))
          .child(
            // A bar that always shows: the pages are taller than the dialog and
            // people should see that they scroll.
            div()
              .relative()
              .flex_1()
              .min_w(0)
              .h_full()
              .child(
                div()
                  .id("st-body")
                  .size_full()
                  .overflow_y_scroll()
                  // Padding inside, not on the scroll area: there it counts as
                  // content and a page that fits still shows a bar.
                  .child(
                    div()
                      .px(28)
                      .py(22)
                      .when(Boolean(this.saveError), (el) =>
                        el.child(
                          div()
                            .id("st-save-error")
                            .pb(10)
                            .text_size(13)
                            .line_height("18px")
                            .text_color(t.blocked)
                            .child(text("settings.save-failed", { reason: this.saveError })),
                        ))
                      .child(
                        section === "appearance"
                          ? appearancePage(this, t, cx)
                          : section === "agents"
                            ? agentsPage(this, t, cx)
                          : section === "keyboard"
                            ? this.keyboard(t)
                          : section === "vault"
                            ? vaultSection(this, t, cx)
                          : section === "memory"
                            ? memoryPage(this, t, cx)
                          : section === "connect"
                            ? connectPage(this, t, cx)
                          : section === "notifications"
                            ? this.alerts(t)
                            : this.about(t, cx),
                      ),
                  ),
              )
              .child(Scrollbar.vertical("st-body").mode("always").absolute().inset_0()),
          ),
      );
    const where = query ? text("settings.search") : sectionTitle(section);
    return sheet({ title: text("settings.sheet-title", { where }), width: 880, onClose: () => this.close() }, content);
  }
}
