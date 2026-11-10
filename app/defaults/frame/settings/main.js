// App-owned settings mod. Writes changes immediately; no Save step.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, Scrollbar, InputState, Input } from "gpui-base";
import { kom, run, theme, updates, shared, appHome, readTextFile, writeTextFile, playSound, props, closeOverlay, window, setShared } from "kompitum";
import { loadVault, vaultSection } from "./vault.js";
import { connectPage, initConnect, loadConnect } from "./connect.js";
import { agentsPage, loadAgents, ensureAgents } from "./agents.js";
import { appearancePage } from "./appearance.js";
import { generalPage } from "./general.js";

const I = (name) => `frame/settings/${name}.svg`;

const SECTIONS = [
  { id: "general", title: "General", icon: "settings" },
  { id: "appearance", title: "Appearance", icon: "palette" },
  { id: "agents", title: "Agents", icon: "bot" },
  { id: "vault", title: "Vault", icon: "mail" },
  { id: "connect", title: "Connect", icon: "link" },
  { id: "notifications", title: "Notifications", icon: "bell" },
  { id: "machines", title: "Machines", icon: "server" },
  { id: "keyboard", title: "Keyboard", icon: "keyboard" },
  { id: "about", title: "About", icon: "info" },
];

const macos = () => window()?.platform === "macos";

// Keyboard holds only macOS's Option key, so other systems never show it.
function liveSections() {
  return macos() ? SECTIONS : SECTIONS.filter((s) => s.id !== "keyboard");
}

// What the search field finds: one entry per row the sections draw, so search
// never reads the drawn tree. `key` matches the row's highlight key.
const SEARCH = [
  { section: "general", key: "folder", title: "Home folder", words: "lead starts projects user drive working directory" },
  { section: "appearance", key: "theme", title: "Theme", words: "light dark system appearance" },
  { section: "appearance", key: "layout", title: "Window", words: "layout reset desk window shape back" },
  { section: "appearance", key: "zoom", title: "Zoom", words: "size scale bigger smaller interface" },
  { section: "appearance", key: "font", title: "Terminal font", words: "typeface monospace ligatures" },
  { section: "appearance", key: "ui_font", title: "Interface font", words: "typeface panels" },
  { section: "agents", key: "lead", title: "CLI", words: "lead agent program claude command code opencode grok" },
  { section: "agents", key: "lead_model", title: "Model", words: "provider models list other" },
  { section: "agents", key: "lead_model", title: "Reasoning", words: "effort levels low medium high xhigh max" },
  { section: "agents", key: "folder", title: "Folder", words: "lead folder project directory picker" },
  { section: "agents", key: "workers", title: "Workers", words: "order add remove drag subagents claude sonnet haiku" },
  { section: "agents", key: "clis", title: "CLI versions", words: "update version releases" },
  { section: "vault", key: "Folder", title: "Vault folder", words: "notes git mailbox" },
  { section: "vault", key: "Needs you", title: "Notes that need you", words: "mailbox notes requests questions" },
  { section: "connect", key: "connect", title: "Connect", words: "agent systems join prompt grok hermes openclaw manager" },
  { section: "notifications", key: "sounds", title: "Play sounds", words: "sound audio notification" },
  { section: "notifications", key: "blocked", title: "An agent needs you", words: "sound blocked attention" },
  { section: "notifications", key: "done", title: "An agent finished", words: "sound done finished" },
  { section: "notifications", key: "Notes between agents", title: "Notes between agents", words: "pop-ups toast vault mail" },
  { section: "notifications", key: "Quiet while a game is fullscreen", title: "Quiet while a game is fullscreen", words: "game fullscreen mute do not disturb" },
  { section: "machines", key: "this", title: "This computer", words: "local version kom" },
  { section: "machines", key: "add", title: "Add a machine", words: "ssh remote host" },
  { section: "keyboard", key: "option", title: "Option key", words: "alt macos accents keyboard" },
  { section: "about", key: "welcome", title: "Welcome", words: "show again onboarding first start setup" },
  { section: "about", key: "app", title: "Kompitum", words: "version update what's new release" },
  { section: "about", key: "core", title: "kompitum-core", words: "version update core terminals" },
];

// Sections that were named differently before; old links still open their page.
const RENAMED_SECTIONS = { look: "appearance", alerts: "notifications" };

export default class Settings extends View {
  init(_props, cx) {
    const asked = props()?.section;
    const wanted = RENAMED_SECTIONS[asked] ?? asked;
    this.section = SECTIONS.some((s) => s.id === wanted) ? wanted : "appearance";
    this.picking = null; // "font" | "ui_font" while its list is open
    this.themeOpen = false;
    ensureAgents(this);
    this.filter = "";
    this.search = "";
    this.highlight = null; // { section, key } while a search result's row is lit
    this.searchInput = InputState.new({ placeholder: "Search or ask" });
    this.searchInput.on("change", (_e, cx) => { this.search = this.searchInput.value(); cx.notify(); });
    this.searchInput.on("submit", (_e, cx) => {
      const first = this.searchMatches()[0];
      if (first) this.openMatch(first, cx);
    });
    this.fontInput = InputState.new({ placeholder: "Type to search fonts" });
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
    this.handle = cx.focus_handle();
    this.handle.focus();
    cx.spawn(async (cx) => {
      await this.load();
      if (this.section === "machines") await this.loadMachines();
      if (this.section === "vault") loadVault(this, cx);
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
      if (!this.asked) return;
      if (updates().checking) this.asked = Date.now();
      else if (Date.now() - this.asked >= 3000) this.asked = null;
      cx.notify();
    });
    cx.timer.every(3000, (cx) => {
      if (this.section !== "machines") return;
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
  change(edit, cx) {
    cx.spawn(async (cx) => {
      for (let attempt = 0; attempt < 3; attempt++) {
        await this.load();
        const next = JSON.parse(JSON.stringify(this.person));
        edit(next);
        try {
          await writeTextFile(this.file(), JSON.stringify(next, null, 2) + "\n", this.raw);
          this.person = next;
          break;
        } catch {}
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
      const text = `${e.title} ${e.words}`.toLowerCase();
      return words.every((w) => text.includes(w));
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
    this.themeOpen = false;
    this.machineEdit = null;
    const at = Date.now();
    this.highlight = { section: match.section, key: match.key, at };
    this.search = "";
    this.searchInput.set_value("");
    this.handle.focus();
    if (match.section === "machines") cx.spawn(async (cx) => { await this.loadMachines(); cx.notify(); });
    if (match.section === "agents") loadAgents(this, cx, true);
    if (match.section === "vault") loadVault(this, cx);
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

  // Opening Keyboard on a system without it lands on General.
  sectionNow() {
    return this.section === "keyboard" && !macos() ? "general" : this.section;
  }

  // Pieces ---------------------------------------------------------------

  heading(t, text) {
    return div().pt(6).pb(8).text_size(12).font_semibold().text_color(t.dim).child(text);
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
          .when(Boolean(about), (el) => el.child(div().text_size(12.5).line_height("18px").text_color(t.dim).child(about))),
      )
      .child(div().flex_none().child(control));
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
      .text_size(13)
      .text_color(t.foreground)
      .hover((s) => s.bg(t.hover))
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
      .text_size(13)
      .text_color(t.muted)
      .hover((s) => s.text_color(t.foreground))
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
      .text_size(13)
      .font_semibold()
      .hover((s) => s.opacity(0.9))
      .on_click(onClick)
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
      .on_click(onClick)
      .child(div().w(16).h(16).rounded(8).bg(on ? t.accent_foreground : t.muted));
  }

  // Look -----------------------------------------------------------------

  keyboard(t) {
    const choices = [
      ["auto", "Automatic (Alt on US keyboards)"],
      ["on", "Alt"],
      ["off", "Types characters"],
      ["left", "Left Option is Alt"],
      ["right", "Right Option is Alt"],
    ];
    return v_flex()
      .w_full()
      .child(this.heading(t, "Option key"))
      .when(window()?.platform === "macos", (el) =>
        el.child(div().pb(12).text_size(13).text_color(t.dim).child("Use Option for terminal shortcuts or to type accents and special characters.")),
      )
      .when(window()?.platform !== "macos", (el) =>
        el.child(div().pb(12).text_size(12.5).line_height("18px").text_color(t.dim).child("Option-key settings are for macOS.")),
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
          .hover((s) => s.bg(t.hover))
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
      if (r.code !== 0) throw new Error(r.stderr || r.stdout || "Couldn't read machines.");
      const list = JSON.parse(r.stdout).machines;
      if (!Array.isArray(list)) throw new Error("Couldn't read machines.");
      this.machineList = list.filter((m) => !m.temporary);
      this.machineError = null;
    } catch (e) {
      this.machineError = String(e);
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
        if (r.code !== 0) throw new Error(payload?.error?.message || (r.stderr || r.stdout || "It didn't work.").trim());
        const message = action === "update"
          ? `Updated ${machine}${payload?.version ? ` to kom ${payload.version}` : ""}; its terminals kept running.`
          : action === "rename" ? `Renamed ${machine} to ${name.trim()}.` : `Removed ${machine}; its terminals keep running there.`;
        if (action === "update") this.machineResults[machine] = { message, ok: true };
        else this.machineNotice = message;
      } catch (e) {
        this.machineResults[machine] = { message: String(e), ok: false };
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
    if (busy) controls.child(svg("mods/workspace-chrome/spinner.svg").size(12).text_color(t.working).spin(900)).child(div().text_size(12).text_color(t.muted).child(busy === "update" ? "Updating…" : busy === "rename" ? "Renaming…" : "Removing…"));
    else {
      if (this.olderCore(m.version, localVersion)) controls.child(this.button(t, `st-m-update-${m.name}`, "Update", (_e, cx) => this.machineAction(m.name, "update", cx)));
      for (const [action, label] of [["rename", "Rename"], ["rm", "Remove"]]) {
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
          .child(div().text_size(12.5).text_color(t.dim).child(`${m.target ?? ""} · ${state} · ${m.version ? `kom ${m.version}` : "kom version unknown"}`)))
        .child(controls.flex_none()));
    if (busy === "update") row.child(div().text_size(12.5).text_color(t.dim).child("Copying this computer's kompitum-core. Its terminals keep running."));
    if (edit && !busy) {
      const cancel = this.button(t, `st-m-cancel-${m.name}`, "Cancel", (_e, cx) => { this.machineEdit = null; this.handle.focus(); cx.notify(); });
      if (edit.action === "rm") row.child(v_flex().gap(8)
        .child(div().text_size(12.5).text_color(t.muted).child(`Remove ${m.name}? Its terminals keep running there.`))
        .child(h_flex().gap(6).child(cancel).child(this.button(t, `st-m-confirm-${m.name}`, "Remove machine", (_e, cx) => this.machineAction(m.name, "rm", cx)))));
      else row.child(v_flex().gap(6)
        .child(div().text_size(12).text_color(t.dim).child("New name (addresses using the old name stop working)"))
        .child(h_flex().gap(6).items_center()
          .child(Input.new(this.renameInput).flex_1().min_w(0).h(32).px(10).rounded(7).border_1().border_color(this.renameFocused ? t.accent : t.input).bg(t.background).text_size(13).text_color(t.foreground))
          .child(cancel).child(this.button(t, `st-m-save-${m.name}`, "Save", (_e, cx) => this.machineAction(m.name, "rename", cx, edit.name)))));
    }
    if (result) row.child(div().text_size(12.5).text_color(result.ok ? t.ansi2 : t.blocked).child(result.message));
    return row;
  }

  machines(t) {
    const version = updates().core.version;
    return v_flex().w_full()
      .child(div().pb(8).text_size(12.5).line_height("18px").text_color(t.dim).child("Your computers, reached over ssh. Their terminals keep running when you disconnect."))
      .child(this.row(t, "This computer", `Connected · ${version ? `kom ${version}` : "kom version unknown"}`, div().w(7).h(7).rounded(4).bg(t.ansi2), "this"))
      .children((this.machineList ?? []).map((m) => this.machineRow(t, m, version)))
      .when(this.machineList === null && !this.machineError, (el) => el.child(h_flex().py(14).gap(8).items_center().child(svg("mods/workspace-chrome/spinner.svg").size(12).text_color(t.working).spin(900)).child(div().text_size(12.5).text_color(t.dim).child("Reading machines…"))))
      .when(this.machineList?.length === 0, (el) => el.child(div().py(14).text_size(12.5).text_color(t.dim).child("No other machines yet.")))
      .when(Boolean(this.machineError), (el) => el.child(div().py(10).text_size(12.5).text_color(t.blocked).child(this.machineError)))
      .when(Boolean(this.machineNotice), (el) => el.child(div().py(10).text_size(12.5).text_color(t.ansi2).child(this.machineNotice)))
      .child(this.row(t, "Add a machine", "Another computer, reached over ssh.", this.button(t, "st-m-add", "Add", () => run("mod.open", { name: "machine-add" })), "add"));
  }

  // Alerts ---------------------------------------------------------------

  alerts(t) {
    const sounds = this.person.sounds ?? {};
    const on = sounds.enabled !== false;
    const between = this.person.notes_between_agents !== false;
    const quiet = this.person.quiet_while_fullscreen !== false;
    const file = (f) => (f ? f.split(/[\\/]/).pop() : "Kompitum's sound");
    return v_flex()
      .w_full()
      .child(this.heading(t, "Sounds"))
      .child(
        this.row(
          t,
          "Play sounds",
          "When an agent needs you, and when one finishes while you're not looking. A note shows in the corner either way.",
          this.toggle(t, "st-sounds", on, (_e, cx) => this.change((p) => (p.sounds = { ...(p.sounds ?? {}), enabled: !on }), cx)),
          "sounds",
        ),
      )
      .child(this.row(t, "An agent needs you", file(sounds.blocked), this.button(t, "st-play-blocked", "Play", () => playSound("blocked")), "blocked"))
      .child(this.row(t, "An agent finished", file(sounds.done), this.button(t, "st-play-done", "Play", () => playSound("done")), "done"))
      .child(div().h(18))
      .child(this.heading(t, "Pop-ups"))
      .child(this.row(t, "Notes between agents", "Show a note for a few seconds when agents write to each other.", this.toggle(t, "st-notes-between-agents", between, (_e, cx) => this.change((p) => (p.notes_between_agents = !between), cx))))
      .child(this.row(t, "Quiet while a game is fullscreen", "No sounds and no passing notes while another app fills the screen. Notes that need you still wait for you.", this.toggle(t, "st-quiet-fullscreen", quiet, (_e, cx) => this.change((p) => (p.quiet_while_fullscreen = !quiet), cx))))
      .child(div().pt(12).text_size(12.5).line_height("18px").text_color(t.dim).child("Want your own sounds? Ask your agent to set sounds.done and sounds.blocked in settings.json to an MP3."));
  }

  // About ----------------------------------------------------------------

  about(t) {
    const u = updates();
    const at = (secs) => {
      const d = new Date(secs * 1000);
      return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    };
    const checked = u.checked ? ` Checked at ${at(u.checked)}.` : "";
    const result = u.checking ? "Checking\u2026" : u.checked
      ? `Up to date.${checked}`
      : "No successful check. Check your connection and try again.";
    // Pressed: "Checking\u2026" while it looks, then "Up to date" for a moment
    // (a find changes the row instead). One per row: an element shows once.
    const asked = this.asked && Date.now() - this.asked < 3000;
    const check = (id) =>
      !u.checks
        ? null
        : u.checking && this.asked
          ? div().px(12).py(5).text_size(13).text_color(t.dim).child("Checking\u2026")
          : asked && u.checked && !u.checking
            ? div().px(12).py(5).text_size(13).text_color(t.ansi2).child("\u2713 Up to date")
            : this.button(t, id, "Check now", (_e, cx) => {
                this.asked = Date.now();
                run("app.check_updates");
                cx.notify();
              });
    const primary = (id, label, f) =>
      div().id(id).px(12).py(5).rounded(7).cursor_pointer().bg(t.accent_fill).text_color(t.accent_foreground).text_size(13).font_semibold().hover((s) => s.opacity(0.9)).on_click(f).child(label);
    const source = !u.app.updates;
    const app = source
      ? [null, null]
      : u.app.ready
        ? [`${u.app.ready} is ready. Restart to switch now; it also switches when you quit.`, primary("st-app-update", "Restart to update", () => run("app.update"))]
        : [result, check("st-check-app")];
    const c = u.core;
    const core = c.managed
      ? [`Managed by ${c.managed}: update it there.`, null]
      : c.updating
        ? ["Updating\u2026 your terminals keep running.", null]
        : c.error
          ? [`The update failed: ${c.error}`, primary("st-core-retry", "Try again", () => run("app.update_core"))]
          : c.ready
            ? [`${c.ready} is ready. Your terminals keep running through it.`, primary("st-core-update", "Update kompitum-core", () => run("app.update_core"))]
            : u.checks
              ? [result, check("st-check-core")]
              : ["Not checked: a build from source looks for no updates.", null];
    return v_flex()
      .w_full()
      .child(
        v_flex()
          .w_full()
          .py(12)
          .gap(9)
          .border_b(1)
          .border_color(t.border)
          .child(div().text_size(14).text_color(t.foreground).child(`Kompitum ${u.app.version} · core ${c.version ?? ""}`))
          .child(div().text_size(12.5).line_height("18px").text_color(t.dim)
            .child(source ? "Built from source; updates come from your checkout." : "Checks for new versions of the app and its core every hour."))
          .child(
            h_flex()
              .w_full()
              .gap(8)
              .items_center()
              .flex_wrap()
              .when(this.lit("app"), (el) => el.bg(t.selected).rounded(7))
              .child(div().text_size(12.5).text_color(t.dim).child("App"))
              .when(Boolean(app[0]), (el) => el.child(div().flex_1().min_w(0).text_size(12.5).line_height("18px").text_color(t.muted).child(app[0])))
              .child(div().flex_1())
              .child(this.button(t, "st-whats-new", "What's new", () => { closeOverlay(props()?.overlay); run("app.whats_new"); }))
              .when(Boolean(app[1]), (el) => el.child(app[1])),
          )
          .child(
            h_flex()
              .w_full()
              .gap(8)
              .items_center()
              .flex_wrap()
              .when(this.lit("core"), (el) => el.bg(t.selected).rounded(7))
              .child(div().text_size(12.5).text_color(t.dim).child("Core"))
              .child(div().flex_1().min_w(0).text_size(12.5).line_height("18px").text_color(t.muted).child(core[0]))
              .when(Boolean(core[1]), (el) => el.child(core[1])),
          ),
      )
      .child(
        this.row(
          t,
          "Welcome",
          null,
          this.button(t, "st-general-welcome", "Show again", () => {
            closeOverlay(props().overlay);
            run("app.welcome");
          }),
          "welcome",
        ),
      )
      .child(div().pt(12).text_size(12.5).line_height("18px").text_color(t.dim).child("Kompitum sends nothing: no usage data, no crash reports. Send feedback opens an issue on GitHub."))
      .child(div().text_size(12.5).line_height("18px").text_color(t.dim).child("Built on herdr (Apache 2.0)."));
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
            .text_size(13)
            .text_color(t.foreground),
        ),
      )
      .when(Boolean(query), (el) =>
        el.children(
          matches.length
            ? matches.map((m, i) => {
                const target = SECTIONS.find((s) => s.id === m.section);
                return h_flex()
                  .id(`st-search-${i}`)
                  .px(10)
                  .py(7)
                  .rounded(7)
                  .cursor_pointer()
                  .hover((st) => st.bg(t.raised))
                  .on_click((_e, cx) => this.openMatch(m, cx))
                  .child(div().text_size(12.5).text_color(t.muted).child(`${target?.title ?? m.section} › ${m.title}`));
              })
            : [
                div().px(10).py(6).text_size(12.5).line_height("18px").text_color(t.dim).child(`No setting matches "${query}".`),
                h_flex()
                  .id("st-ask-lead")
                  .px(10)
                  .py(7)
                  .gap(7)
                  .items_center()
                  .rounded(7)
                  .cursor_pointer()
                  .hover((st) => st.bg(t.raised))
                  .on_click((_e, cx) => this.askLead(cx))
                  .child(svg(I("bot")).size(14).text_color(t.accent))
                  .child(div().text_size(12.5).text_color(t.foreground).child("Ask the lead")),
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
            .bg(on ? t.raised : "#00000000")
            .hover((st) => st.bg(t.raised))
            .on_click((_e, cx) => {
              this.section = s.id;
              this.picking = null;
              this.themeOpen = false;
              this.machineEdit = null;
              this.highlight = null;
              this.handle.focus();
              if (s.id === "machines") cx.spawn(async (cx) => { await this.loadMachines(); cx.notify(); });
              if (s.id === "vault") loadVault(this, cx);
              if (s.id === "connect" && this.connect.vault === null) loadConnect(this, cx);
              if (s.id === "agents") loadAgents(this, cx);
              cx.notify();
            })
            .child(svg(I(s.icon)).size(15).text_color(on ? t.accent : t.dim))
            .child(div().flex_1().text_size(13.5).text_color(on ? t.foreground : t.muted).child(s.title))
            .when(s.id === "about" && waiting, (el) => el.child(div().w(7).h(7).rounded(4).bg(t.accent)));
        }),
      ));
    return v_flex()
      .id("settings")
      .track_focus(this.handle)
      .w_full()
      .h(620)
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
        if (k === "escape") {
          if (this.search) this.clearSearch(cx);
          else if (this.picking) this.picking = null;
          else if (this.themeOpen) this.themeOpen = false;
          else return this.close();
          this.handle.focus();
        } else return;
        cx.stop_propagation();
        cx.notify();
      })
      .child(
        h_flex()
          .flex_none()
          .h(44)
          .px(16)
          .items_center()
          .border_b(1)
          .border_color(t.border)
          .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Settings"))
          .child(div().flex_1())
          .child(
            div()
              .id("st-close")
              .w(26)
              .h(26)
              .rounded(7)
              .flex()
              .items_center()
              .justify_center()
              .text_color(t.muted)
              .cursor_pointer()
              .hover((s) => s.bg(t.hover).text_color(t.foreground))
              .on_click(() => this.close())
              .child(svg("mods/workspace-chrome/x.svg").size(14)),
          ),
      )
      .child(
        h_flex()
          .flex_1()
          .min_h(0)
          .items_start()
          .child(nav.h_full().child(div().flex_1()).child(this.textButton(t, "st-feedback", "Send feedback", () => { closeOverlay(props()?.overlay); run("app.feedback"); })))
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
                      .child(
                        section === "general"
                          ? generalPage(this, t, cx)
                          : section === "appearance"
                            ? appearancePage(this, t, cx)
                          : section === "agents"
                            ? agentsPage(this, t, cx)
                          : section === "keyboard"
                            ? this.keyboard(t)
                          : section === "machines"
                            ? this.machines(t)
                          : section === "vault"
                            ? vaultSection(this, t, cx)
                          : section === "connect"
                            ? connectPage(this, t, cx)
                          : section === "notifications"
                            ? this.alerts(t)
                            : this.about(t),
                      ),
                  ),
              )
              .child(Scrollbar.vertical("st-body").mode("always").absolute().inset_0()),
          ),
      );
  }
}
