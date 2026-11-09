// First run in three steps: pick the lead, pick its home folder, connect agents.
// Records finished/skipped in frame.welcomed; app.welcome opens it again.

import { View, div, svg, image } from "gpui-kit";
import { v_flex, h_flex, Scrollbar } from "gpui-base";
import {
  knownAgents,
  run,
  theme,
  props,
  closeOverlay,
  setShared,
  kom,
  home,
  appHome,
  readTextFile,
  writeTextFile,
  settings,
  window,
  pickFolder,
  openLink,
  terminals,
  spaces,
  leadSetup,
} from "kompitum";

const W = (name) => `mods/workspace-chrome/${name}.svg`;
const STEPS = 3;
const STEP_NAMES = ["Pick your lead", "Pick a home folder", "Connect agents"];

function errorText(error) {
  return String(error?.message ?? error ?? "").trim();
}

function mono() {
  return settings()?.font_family ?? "Google Sans Code";
}

export default class Welcome extends View {
  init(_props, cx) {
    this.step = 1;
    this.agents = [];
    this.chosen = null;
    this.signedIn = {}; // key -> true/false; never the check's output
    this.runs = {}; // key -> { kind, name } while its tab runs
    this.notes = {}; // key -> what an ended run said
    this.useClaude = true;
    this.setupError = null;
    this.startError = null;
    this.folder = home();
    this.person = {};
    this.raw = null;
    this.busy = false;
    this.handle = cx.focus_handle();
    this.handle.focus();
    // It opens while the window is still starting, which takes the keyboard
    // back for its first pane: take it again once that's done.
    cx.timer.after(600, () => this.handle.focus());
    cx.spawn(async (cx) => {
      await this.loadSettings();
      await this.loadAgents();
      cx.notify();
    });
    // Install and sign-in tabs end while the window is open: follow them.
    cx.timer.every(1000, (cx) => {
      if (Object.keys(this.runs).length) this.followRuns(cx);
    });
  }

  file() {
    return `${appHome()}/settings.json`;
  }

  async loadSettings() {
    this.raw = await readTextFile(this.file()).catch(() => null);
    let person = {};
    try {
      person = this.raw ? JSON.parse(this.raw) : {};
    } catch {
      person = {};
    }
    this.person = person;
    const lead = person.lead ?? {};
    this.folder = lead.folder || home();
    this.chosen = lead.program || null;
  }

  async loadAgents() {
    const list = (await knownAgents().catch(() => [])) ?? [];
    this.agents = list.filter((agent) => agent.flags);
    if (!this.chosen || !this.agents.some((agent) => agent.key === this.chosen)) {
      this.chosen = this.defaultChoice();
    }
    await this.checkSignedIn();
  }

  // `lead.program`, else Claude Code when installed, else the first installed one.
  defaultChoice() {
    const fromSettings = this.person?.lead?.program;
    if (fromSettings && this.agents.some((agent) => agent.key === fromSettings)) return fromSettings;
    const claude = this.agents.find((agent) => agent.key === "claude");
    if (claude?.installed) return claude.key;
    return (this.agents.find((agent) => agent.installed) ?? this.agents[0])?.key ?? null;
  }

  // The exit code only: its output can name the account.
  async checkSignedIn() {
    const checks = [];
    for (const agent of this.agents) {
      if (!agent.installed || !agent.signed_in?.length) {
        delete this.signedIn[agent.key];
        continue;
      }
      checks.push(
        (async () => {
          try {
            const result = await kom(["exec", "--", ...agent.signed_in]);
            this.signedIn[agent.key] = result.code === 0;
          } catch {
            this.signedIn[agent.key] = false;
          }
        })(),
      );
    }
    await Promise.all(checks);
  }

  chosenAgent() {
    return this.agents.find((agent) => agent.key === this.chosen) ?? null;
  }

  canContinue() {
    return Boolean(this.chosenAgent()?.installed);
  }

  installArgv(agent) {
    const argv = agent.install?.[window()?.platform];
    return Array.isArray(argv) && argv.length ? argv : null;
  }

  startRun(kind, agent, argv, cx) {
    if (!argv?.length) return;
    const name = kind === "Install" ? `Install ${agent.name}` : `Sign in to ${agent.name}`;
    this.chosen = agent.key;
    this.runs[agent.key] = { kind, name };
    this.notes[agent.key] = null;
    run("terminal.open", { argv, place: "tab", name });
    cx.notify();
  }

  // Follow each run in terminals() by the name it was given: the core's own
  // name when it has one, else the tab the app named and its terminals' label.
  runTerminal(running, live) {
    if (running.terminal) {
      const known = live.find((one) => one.address === running.terminal);
      if (known) return known;
    }
    const byName = live.find((one) => one.name === running.name);
    if (byName) {
      running.terminal = byName.address;
      return byName;
    }
    const tab = spaces()
      .flatMap((space) => space.tabs ?? [])
      .find((one) => one.name === running.name);
    if (!tab) return null;
    const byTab = live.find((one) => one.labels?.["kompitum.tab"] === tab.id);
    if (byTab) running.terminal = byTab.address;
    return byTab ?? null;
  }

  followRuns(cx) {
    const live = terminals();
    for (const [key, running] of Object.entries(this.runs)) {
      const terminal = this.runTerminal(running, live);
      if (!terminal) continue;
      const ended = terminal.exit_code != null || (terminal.state && terminal.state !== "live");
      if (!ended) continue;
      delete this.runs[key];
      const code = terminal.exit_code;
      this.notes[key] =
        code != null && code !== 0
          ? `${running.kind} ended with code ${code}. Its tab shows why.`
          : null;
      cx.spawn(async (cx) => {
        await this.loadAgents();
        cx.notify();
      });
    }
    cx.notify();
  }

  done(outcome) {
    setShared("frame.welcomed", outcome);
    closeOverlay(props().overlay);
  }

  // Reads, edits and writes like Settings' change(), keeping other keys.
  async saveLead() {
    for (let attempt = 0; attempt < 3; attempt++) {
      this.raw = await readTextFile(this.file()).catch(() => null);
      let person = {};
      try {
        person = this.raw ? JSON.parse(this.raw) : {};
      } catch {
        person = {};
      }
      const next = JSON.parse(JSON.stringify(person));
      const lead = { ...(next.lead ?? {}), program: this.chosen };
      if (this.folder) lead.folder = this.folder;
      else delete lead.folder;
      next.lead = lead;
      try {
        await writeTextFile(this.file(), JSON.stringify(next, null, 2) + "\n", this.raw);
        this.person = next;
        return;
      } catch {}
    }
    throw new Error("Couldn't write settings.json.");
  }

  async start(cx) {
    if (this.busy || !this.canContinue()) return;
    this.busy = true;
    this.startError = null;
    this.setupError = null;
    cx.notify();
    try {
      await this.saveLead();
    } catch (error) {
      this.startError = errorText(error) || "Couldn't write settings.json.";
      this.busy = false;
      cx.notify();
      return;
    }
    if (this.chosen === "claude" && this.useClaude) {
      try {
        await leadSetup();
      } catch (error) {
        // Its words stay under the toggle, and the next Start goes on without it.
        this.setupError = errorText(error) || "lead.setup failed.";
        this.useClaude = false;
        this.busy = false;
        cx.notify();
        return;
      }
    }
    this.done("finished");
    run("lead.open");
  }

  chooseFolder(cx) {
    cx.spawn(async (cx) => {
      const folder = await pickFolder({ prompt: "Choose" }).catch(() => null);
      if (!folder) return; // a cancelled pick keeps the old choice
      this.folder = folder;
      cx.notify();
    });
  }

  // Pieces ---------------------------------------------------------------

  para(t, text) {
    return div().w_full().text_size(13).line_height("20px").text_color(t.muted).child(text);
  }

  title(t, text, sub) {
    return v_flex()
      .gap(6)
      .child(div().text_size(18).font_semibold().text_color(t.foreground).child(text))
      .when(Boolean(sub), (el) => el.child(this.para(t, sub)));
  }

  button(t, id, label, primary, enabled, onClick, icon) {
    return h_flex()
      .id(id)
      .h(34)
      .px(14)
      .gap(8)
      .items_center()
      .rounded(9)
      .text_size(13)
      .font_medium()
      .when(enabled, (el) => el.cursor_pointer())
      .when(enabled && primary, (el) => el.bg(t.accent_fill).text_color(t.accent_foreground).hover((s) => s.opacity(0.88)))
      .when(enabled && !primary, (el) => el.text_color(t.muted).hover((s) => s.text_color(t.foreground).bg(t.hover)))
      .when(!enabled && primary, (el) => el.bg(t.hover).text_color(t.dim))
      .when(!enabled && !primary, (el) => el.text_color(t.dim))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .when(enabled, (el) => el.on_click(enabled ? onClick : undefined))
      .child(label)
      .when(Boolean(icon), (el) => el.child(svg(W(icon)).size(14)));
  }

  // A small button on an agent's row: it picks that row too.
  rowButton(t, id, label, onClick) {
    return div()
      .id(id)
      .px(10)
      .py(4)
      .rounded(7)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .cursor_pointer()
      .text_size(12.5)
      .text_color(t.foreground)
      .hover((s) => s.bg(t.hover))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
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
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click(onClick)
      .child(div().w(16).h(16).rounded(8).bg(on ? t.accent_foreground : t.muted));
  }

  agentIcon(t, agent) {
    if (!agent.icon) return div().w(20).h(20).flex_none();
    return agent.tint
      ? svg(agent.icon).size(18).flex_none().text_color(t.muted)
      : image(agent.icon).w(18).h(18).flex_none();
  }

  stateOf(t, agent, cx) {
    if (this.runs[agent.key]) {
      return div().text_size(12.5).text_color(t.dim).child("Running in the tab behind this window.");
    }
    if (!agent.installed) {
      const argv = this.installArgv(agent);
      if (argv) {
        return this.rowButton(t, `w-install-${agent.key}`, "Install", (_e, cx) =>
          this.startRun("Install", agent, argv, cx),
        );
      }
      return this.rowButton(t, `w-get-${agent.key}`, `Get ${agent.name}`, () => {
        if (agent.install?.page) openLink(agent.install.page);
      });
    }
    if (agent.signed_in?.length) {
      if (this.signedIn[agent.key] === true) {
        return h_flex()
          .gap(6)
          .items_center()
          .child(svg(W("check")).size(13).text_color(t.ansi2))
          .child(div().text_size(12.5).text_color(t.muted).child("Signed in"));
      }
      if (this.signedIn[agent.key] === false) {
        return h_flex()
          .gap(8)
          .items_center()
          .child(div().text_size(12.5).text_color(t.muted).child("Not signed in"))
          .child(
            this.rowButton(t, `w-signin-${agent.key}`, "Sign in", (_e, cx) =>
              this.startRun("Sign in", agent, agent.login, cx),
            ),
          );
      }
      return div().text_size(12.5).text_color(t.dim).child("Checking\u2026");
    }
    return this.rowButton(t, `w-signin-${agent.key}`, "Sign in", (_e, cx) =>
      this.startRun("Sign in", agent, agent.login, cx),
    );
  }

  agentRow(t, agent, cx) {
    const chosen = this.chosen === agent.key;
    const running = Boolean(this.runs[agent.key]);
    const argv = this.installArgv(agent);
    const noCheck = agent.installed && !agent.signed_in?.length;
    return v_flex()
      .w_full()
      .gap(6)
      .child(
        h_flex()
          .id(`w-agent-${agent.key}`)
          .w_full()
          .h(52)
          .px(12)
          .gap(10)
          .items_center()
          .rounded(10)
          .border_1()
          .border_color(chosen ? t.accent : t.border)
          .bg(chosen ? t.selected : t.background)
          .cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => {
            this.chosen = agent.key;
            cx.notify();
          })
          .child(
            div()
              .w(16)
              .h(16)
              .rounded(8)
              .border_1()
              .border_color(chosen ? t.accent : t.input)
              .flex_none()
              .flex()
              .items_center()
              .justify_center()
              .when(chosen, (el) => el.child(div().w(8).h(8).rounded(4).bg(t.accent))),
          )
          .child(this.agentIcon(t, agent))
          .child(div().flex_1().min_w(0).truncate().text_size(14).text_color(t.foreground).child(agent.name))
          .child(this.stateOf(t, agent, cx)),
      )
      .when(noCheck, (el) =>
        el.child(
          div()
            .pl(38)
            .text_size(12)
            .text_color(t.dim)
            .child(`Kompitum can't check whether ${agent.name} is signed in.`),
        ),
      )
      .when(chosen && !agent.installed && !running && Boolean(argv), (el) =>
        el.child(
          div()
            .pl(38)
            .font_family(mono())
            .text_size(12)
            .text_color(t.dim)
            .child(argv.join(" ")),
        ),
      )
      .when(Boolean(this.notes[agent.key]), (el) =>
        el.child(div().pl(38).text_size(12.5).line_height("18px").text_color(t.blocked).child(this.notes[agent.key])),
      );
  }

  stepLine(t) {
    return h_flex()
      .gap(8)
      .items_center()
      .child(div().text_size(12).font_semibold().text_color(t.dim).child(`${this.step} of ${STEPS}`))
      .child(div().text_size(12).text_color(t.dim).child(STEP_NAMES[this.step - 1]));
  }

  // Step 1 ---------------------------------------------------------------

  stepOne(t, cx) {
    const rows = [...this.agents].sort((a, b) => (a.key === "claude" ? -1 : b.key === "claude" ? 1 : 0));
    const platform = window()?.platform;
    return v_flex()
      .gap(16)
      .child(this.stepLine(t))
      .child(
        this.title(
          t,
          "Pick your lead",
          "The lead is the agent you talk to. It plans the work and hands parts of it to workers.",
        ),
      )
      .child(
        v_flex()
          .gap(8)
          .when(rows.length === 0, (el) =>
            el.child(div().py(12).text_size(12.5).text_color(t.dim).child("Reading agents\u2026")),
          )
          .children(rows.map((agent) => this.agentRow(t, agent, cx))),
      )
      .child(
        this.para(
          t,
          platform === "windows"
            ? "Closing the window keeps Kompitum in the tray. Your agents keep running either way."
            : "Your agents keep running when you close the window.",
        ),
      );
  }

  // Step 2 ---------------------------------------------------------------

  stepTwo(t, cx) {
    return v_flex()
      .gap(16)
      .child(this.stepLine(t))
      .child(
        this.title(
          t,
          "Pick a home folder",
          "Kompitum is one ongoing conversation. Your lead always starts in this folder and reaches every project inside it, so you never switch folders or start over.",
        ),
      )
      .child(this.para(t, "Pick the folder that holds your work: your projects folder, your user folder, or a whole drive."))
      .child(
        h_flex()
          .w_full()
          .gap(10)
          .items_center()
          .child(
            div()
              .flex_1()
              .min_w(0)
              .truncate()
              .px(12)
              .py(8)
              .rounded(8)
              .border_1()
              .border_color(t.input)
              .bg(t.background)
              .text_size(13)
              .text_color(t.foreground)
              .child(this.folder),
          )
          .child(this.button(t, "w-choose-folder", "Choose\u2026", false, true, (_e, cx) => this.chooseFolder(cx))),
      )
      .child(this.para(t, "You can change it later in Settings \u203a General. A new folder starts a new conversation."));
  }

  // Step 3 ---------------------------------------------------------------

  stepThree(t, cx) {
    return v_flex()
      .gap(16)
      .child(this.stepLine(t))
      .child(
        h_flex()
          .gap(10)
          .items_center()
          .child(div().flex_1().text_size(18).font_semibold().text_color(t.foreground).child("Connect agents"))
          .child(
            div()
              .px(7)
              .py(1)
              .rounded(5)
              .border_1()
              .border_color(t.input)
              .text_size(10.5)
              .font_semibold()
              .text_color(t.dim)
              .child("OPTIONAL"),
          ),
      )
      .child(this.para(t, "Other agent systems on this computer can send notes to your lead and take work from it."))
      .child(
        h_flex()
          .child(this.button(t, "w-open-connect", "Open Connect", false, true, () => run("app.settings", { section: "connect" }))),
      )
      .when(this.chosen === "claude", (el) =>
        el.child(
          v_flex()
            .gap(6)
            .p(12)
            .rounded(10)
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .child(
              h_flex()
                .gap(12)
                .items_center()
                .child(this.toggle(t, "w-claude-setup", this.useClaude, (_e, cx) => {
                  this.useClaude = !this.useClaude;
                  cx.notify();
                }))
                .child(div().flex_1().text_size(13.5).font_semibold().text_color(t.foreground).child("Let Claude Code use Kompitum")),
            )
            .child(
              div()
                .text_size(12.5)
                .line_height("18px")
                .text_color(t.dim)
                .child("Adds Kompitum's tools to Claude Code for all your projects, and teaches it the kom skill."),
            )
            .when(Boolean(this.setupError), (el) =>
              el.child(div().text_size(12.5).line_height("18px").text_color(t.blocked).child(this.setupError)),
            ),
        ),
      )
      .when(Boolean(this.startError), (el) =>
        el.child(div().text_size(12.5).line_height("18px").text_color(t.blocked).child(this.startError)),
      );
  }

  // ----------------------------------------------------------------------

  render(cx) {
    const t = theme();
    const page = [this.stepOne, this.stepTwo, this.stepThree][this.step - 1];
    const last = this.step === STEPS;
    const enabled = this.canContinue() && !this.busy;
    return v_flex()
      .id("welcome")
      .track_focus(this.handle)
      .size_full()
      .bg(t.popover)
      .on_key_down((e, cx) => {
        const k = e.key;
        const m = e.modifiers;
        if (k === "escape") return this.done("skipped");
        if (m.control || m.alt || m.platform) return;
        if (k === "enter" || k === "right") {
          if (last) {
            if (k === "enter") this.start(cx);
            cx.stop_propagation();
            return;
          }
          if (k === "enter" && !enabled) {
            cx.stop_propagation();
            return;
          }
          this.step = Math.min(STEPS, this.step + 1);
          cx.notify();
        } else if (k === "left") {
          this.step = Math.max(1, this.step - 1);
          cx.notify();
        } else return;
        cx.stop_propagation();
      })
      .child(
        h_flex()
          .flex_none()
          .h(52)
          // Only the header drags the window, and only it shows the grab hand.
          .cursor_grab()
          .pl(20)
          .pr(12)
          .gap(10)
          .items_center()
          .border_b(1)
          .border_color(t.border)
          .child(image("frame/welcome/kompitum.png").w(24).h(24).rounded(6).flex_none())
          .child(div().text_size(14).font_semibold().text_color(t.foreground).child("Welcome to Kompitum"))
          .child(div().px(6).py(0.5).rounded(5).border_1().border_color(t.input).text_color(t.muted).text_size(10.5).font_semibold().child("BETA"))
          .child(div().flex_1())
          .child(div().text_size(11.5).text_color(t.dim).child("Drag to move"))
          .child(
            div()
              .id("w-close")
              .w(26)
              .h(26)
              .rounded(7)
              .flex()
              .items_center()
              .justify_center()
              .text_color(t.muted)
              .cursor_pointer()
              .hover((s) => s.bg(t.hover).text_color(t.foreground))
              .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
              .on_click(() => this.done("skipped"))
              .child(svg(W("x")).size(14)),
          ),
      )
      // A bar that always shows: a step taller than the window should look it.
      .child(
        div()
          .relative()
          .flex_1()
          .min_h(0)
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .child(
            div()
              .id("w-body")
              .size_full()
              .overflow_y_scroll()
              // Padding inside, not on the scroll area: there it counts as
              // content and a step that fits still shows a bar.
              .child(div().px(22).pt(22).pb(20).child(page.call(this, t, cx))),
          )
          .child(Scrollbar.vertical("w-body").mode("always").absolute().inset_0()),
      )
      .child(
        h_flex()
          .flex_none()
          .px(16)
          .py(12)
          .gap(6)
          .items_center()
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .border_t(1)
          .border_color(t.border)
          .child(div().flex_1())
          .child(
            this.button(t, "w-back", "Back", false, this.step > 1, (_e, cx) => {
              this.step = Math.max(1, this.step - 1);
              cx.notify();
            }),
          )
          .child(
            last
              ? this.button(t, "w-start", "Start", true, enabled, (_e, cx) => this.start(cx))
              : this.button(t, "w-next", "Continue", true, enabled, (_e, cx) => {
                  this.step = Math.min(STEPS, this.step + 1);
                  cx.notify();
                }),
          ),
      );
  }
}
