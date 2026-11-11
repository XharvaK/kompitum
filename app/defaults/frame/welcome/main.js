// First run: pick the lead, the folder it starts in, and (for Claude Code) one permission.
// Records finished/skipped in frame.welcomed; app.welcome opens it again.

import { View, div, svg, image, PathBuilder, Background } from "gpui-kit";
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
  window as appWindow,
  pickFolder,
  openLink,
  terminals,
  spaces,
  leadSetup,
} from "kompitum";

const W = (name) => `mods/workspace-chrome/${name}.svg`;
// A folder with one of these names, directly in the home folder, is a projects folder.
const PROJECT_FOLDERS = ["projects", "code", "src", "dev"];

function errorText(error) {
  return String(error?.message ?? error ?? "").trim();
}

const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

function mono() {
  return settings()?.font_family ?? "Google Sans Code";
}

// Windows paths come with either slash and any case: compare them without that.
const norm = (p) => String(p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
const sameFolder = (a, b) => Boolean(a) && Boolean(b) && norm(a) === norm(b);

function isProjectsFolder(folder) {
  const path = norm(folder);
  const cut = path.lastIndexOf("/");
  return cut >= 0 && PROJECT_FOLDERS.includes(path.slice(cut + 1)) && path.slice(0, cut) === norm(home());
}

// The list's bottom edge fades into the window instead of cutting a row off.
function fadeBottom(t) {
  const color = String(t.popover ?? t.background ?? "");
  const clear = /^#[0-9a-fA-F]{6}$/.test(color) ? `${color}00` : color;
  const rect = PathBuilder.fill().move_to("0%", "0%").line_to("100%", "0%").line_to("100%", "100%").line_to("0%", "100%").close().build();
  return window
    .paint_path(rect, Background.linear_gradient(180, Background.stop(clear, 0), Background.stop(color, 1)))
    .absolute()
    .left_0()
    .right_0()
    .bottom_0()
    .h(24);
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
    this.projects = null; // a projects folder the step knows: its own row
    this.other = null; // a folder picked with "Another folder…", unless it is a row already
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
    this.setFolder(lead.folder || home());
    this.chosen = lead.program || null;
  }

  // Shows the folder in the row it belongs to: a projects folder, the home folder, or another.
  setFolder(folder) {
    this.folder = folder;
    if (isProjectsFolder(folder)) this.projects = folder;
    this.other = isProjectsFolder(folder) || sameFolder(folder, home()) ? null : folder;
  }

  async loadAgents(fresh) {
    const list = (await knownAgents({ fresh }).catch(() => [])) ?? [];
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

  // Claude Code gets a third step for its permission; any other lead ends on the second.
  steps() {
    return this.chosen === "claude" ? 3 : 2;
  }

  next(cx) {
    this.step = Math.min(this.steps(), this.step + 1);
    cx.notify();
  }

  back(cx) {
    this.step = Math.max(1, Math.min(this.step, this.steps()) - 1);
    cx.notify();
  }

  installArgv(agent) {
    const argv = agent.install?.[appWindow()?.platform];
    return Array.isArray(argv) && argv.length ? argv : null;
  }

  startRun(kind, agent, argv, cx) {
    if (!argv?.length) return;
    const name = kind === "Install" ? `Install ${agent.name}` : `Sign in to ${agent.name}`;
    this.chosen = agent.key;
    this.runs[agent.key] = { kind, name, started: Date.now() };
    this.notes[agent.key] = null;
    run("terminal.open", { argv, place: "tab", name, stay: false, machine: "" });
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
      // Its tab closes when the program ends: a run seen once, or older than a
      // few seconds, that can't be found any more has ended.
      const gone = !terminal && (Boolean(running.terminal) || Date.now() - running.started > 3000);
      if (!terminal && !gone) continue;
      const ended = gone || terminal.exit_code != null || (terminal.state && terminal.state !== "live");
      if (!ended) continue;
      delete this.runs[key];
      const code = terminal?.exit_code;
      this.notes[key] = code != null && code !== 0 ? `${running.kind} ended with code ${code}.` : null;
      cx.spawn(async (cx) => {
        await this.loadAgents(true);
        const agent = this.agents.find((one) => one.key === key);
        if (running.kind === "Install" && agent && !agent.installed && !this.notes[key]) {
          this.notes[key] = `The install ended, but Kompitum can't find ${agent.name} yet.`;
        }
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
      this.setFolder(folder);
      cx.notify();
    });
  }

  // The folder rows: a projects folder when the step knows one, the home folder, and another.
  folderRows() {
    return [
      ...(this.projects ? [{ id: "w-folder-projects", label: "Projects folder", path: this.projects }] : []),
      { id: "w-folder-user", label: "User folder", path: home() },
      { id: "w-choose-folder", label: "Another folder…", path: this.other },
    ];
  }

  // Pieces ---------------------------------------------------------------

  para(t, text) {
    return div().w_full().text_size(13.5).line_height("20px").text_color(t.muted).child(text);
  }

  // A step's heading: the title, then the sub-text when it has one.
  heading(t, text, sub) {
    return v_flex()
      .w_full()
      .gap(6)
      .child(div().text_size(20).font_semibold().text_color(t.foreground).child(text))
      .when(Boolean(sub), (el) => el.child(this.para(t, sub)));
  }

  primaryButton(t, id, label, enabled, onClick) {
    return h_flex()
      .id(id)
      .h(32)
      .px(16)
      .rounded(8)
      .items_center()
      .text_size(13)
      .font_medium()
      .when(enabled, (el) =>
        el.cursor_pointer().bg(t.accent_fill).text_color(t.accent_foreground).hover((s) => s.opacity(0.88)).on_click(onClick),
      )
      .when(!enabled, (el) => el.bg(t.hover).text_color(t.dim))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(label);
  }

  // A quiet text button on an agent's row: it picks that row too.
  rowButton(t, id, label, onClick) {
    return div()
      .id(id)
      .px(8)
      .py(3)
      .rounded(6)
      .cursor_pointer()
      .text_size(12.5)
      .text_color(t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
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

  radio(t, on) {
    return div()
      .w(16)
      .h(16)
      .rounded(8)
      .border_1()
      .border_color(on ? t.accent : t.input)
      .flex_none()
      .flex()
      .items_center()
      .justify_center()
      .when(on, (el) => el.child(div().w(8).h(8).rounded(4).bg(t.accent)));
  }

  errorLine(t, text) {
    return div().text_size(12.5).line_height("18px").text_color(t.blocked).tooltip(text).child(firstLine(text) || text);
  }

  agentIcon(t, agent) {
    if (!agent.icon) return div().w(20).h(20).flex_none();
    return agent.tint
      ? svg(agent.icon).size(18).flex_none().text_color(t.muted)
      : image(agent.icon).w(18).h(18).flex_none();
  }

  // What the app has observed about a program, and nothing more.
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
    if (!agent.signed_in?.length) {
      return div().text_size(12.5).text_color(t.dim).child("Installed");
    }
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
    return div().text_size(12.5).text_color(t.dim).child("Checking…");
  }

  agentRow(t, agent, cx) {
    const chosen = this.chosen === agent.key;
    const running = Boolean(this.runs[agent.key]);
    const argv = this.installArgv(agent);
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
          .child(this.radio(t, chosen))
          .child(this.agentIcon(t, agent))
          .child(div().flex_1().min_w(0).truncate().text_size(14).text_color(t.foreground).child(agent.name))
          .child(this.stateOf(t, agent, cx)),
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

  // Step 1 ---------------------------------------------------------------

  stepOne(t, cx) {
    const rows = [...this.agents].sort((a, b) => (a.key === "claude" ? -1 : b.key === "claude" ? 1 : 0));
    return v_flex()
      .w_full()
      .gap(18)
      .child(
        this.heading(
          t,
          "Who do you want to talk to?",
          "This is your lead. It plans the work and hands parts of it to workers.",
        ),
      )
      .child(
        v_flex()
          .w_full()
          .gap(8)
          .when(rows.length === 0, (el) =>
            el.child(div().py(12).text_size(12.5).text_color(t.dim).child("Reading agents…")),
          )
          .children(rows.map((agent) => this.agentRow(t, agent, cx))),
      );
  }

  // Step 2 ---------------------------------------------------------------

  folderRow(t, row, onClick) {
    const on = sameFolder(this.folder, row.path);
    return h_flex()
      .id(row.id)
      .w_full()
      .h(52)
      .px(12)
      .gap(10)
      .items_center()
      .rounded(10)
      .border_1()
      .border_color(on ? t.accent : t.border)
      .bg(on ? t.selected : t.background)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_click(onClick)
      .child(this.radio(t, on))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .child(div().truncate().text_size(14).text_color(t.foreground).child(row.label))
          .when(Boolean(row.path), (el) => el.child(div().truncate().text_size(12).text_color(t.dim).child(row.path))),
      );
  }

  stepTwo(t, cx) {
    return v_flex()
      .w_full()
      .gap(18)
      .child(
        this.heading(
          t,
          "Where is your work?",
          "Your lead starts in this folder and can reach every project inside it.",
        ),
      )
      .child(
        v_flex()
          .w_full()
          .gap(12)
          .child(
            v_flex()
              .w_full()
              .gap(8)
              .children(
                this.folderRows().map((row) =>
                  this.folderRow(t, row, (_e, cx) => {
                    if (row.id === "w-choose-folder") return this.chooseFolder(cx);
                    this.folder = row.path;
                    cx.notify();
                  }),
                ),
              ),
          )
          .child(div().text_size(12).text_color(t.dim).child("You can change this in Settings.")),
      );
  }

  // Step 3 ---------------------------------------------------------------

  stepThree(t, cx) {
    return v_flex()
      .w_full()
      .gap(18)
      .child(this.heading(t, "One permission"))
      .child(
        v_flex()
          .w_full()
          .gap(10)
          .child(
            h_flex()
              .w_full()
              .h(64)
              .px(16)
              .gap(12)
              .items_center()
              .rounded(10)
              .border_1()
              .border_color(t.border)
              .bg(t.raised)
              .child(
                v_flex()
                  .flex_1()
                  .min_w(0)
                  .gap(2)
                  .child(div().text_size(14).text_color(t.foreground).child("Let Claude Code use Kompitum"))
                  .child(
                    div()
                      .text_size(13)
                      .line_height("18px")
                      .text_color(t.muted)
                      .child(
                        "Adds Kompitum's tools and skill to Claude Code, for all your projects. You can remove them in Settings › Agents.",
                      ),
                  ),
              )
              .child(
                this.toggle(t, "w-claude-setup", this.useClaude, (_e, cx) => {
                  this.useClaude = !this.useClaude;
                  cx.notify();
                }),
              ),
          )
          .when(Boolean(this.setupError), (el) => el.child(this.errorLine(t, this.setupError))),
      );
  }

  // ----------------------------------------------------------------------

  render(cx) {
    const t = theme();
    // The step shown: a step that is gone (Claude Code not the lead) shows the last one.
    // Not written back: the lead may not have loaded yet.
    const steps = this.steps();
    const step = Math.min(this.step, steps);
    const page = [this.stepOne, this.stepTwo, this.stepThree][step - 1];
    const last = step === steps;
    const enabled = this.canContinue() && !this.busy;

    const header = h_flex()
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
      .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Welcome to Kompitum"))
      .child(div().flex_1())
      .child(
        div()
          .id("w-close")
          .w(28)
          .h(28)
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
      );

    const progress = h_flex()
      .flex_none()
      .h(36)
      .gap(8)
      .items_center()
      .justify_center()
      .children(
        Array.from({ length: steps }, (_, i) =>
          div()
            .w(6)
            .h(6)
            .rounded(3)
            .bg(i + 1 === step ? t.accent : t.border),
        ),
      );

    // A bar that always shows: a step taller than the window should look it.
    const body = div()
      .relative()
      .flex_1()
      .min_h(0)
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(
        div()
          .id("w-body")
          .size_full()
          .overflow_y_scroll()
          .child(
            div()
              .px(28)
              .pt(20)
              .pb(20)
              .child(
                v_flex()
                  .w_full()
                  .gap(12)
                  .child(page.call(this, t, cx))
                  .when(Boolean(this.startError), (el) => el.child(this.errorLine(t, this.startError))),
              ),
          ),
      )
      .child(fadeBottom(t))
      .child(Scrollbar.vertical("w-body").mode("always").absolute().inset_0());

    const footer = h_flex()
      .flex_none()
      .h(60)
      .px(20)
      .items_center()
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .border_t(1)
      .border_color(t.border)
      .when(step > 1, (el) => el.child(this.rowButton(t, "w-back", "Back", (_e, cx) => this.back(cx))))
      .child(div().flex_1())
      .child(
        last
          ? this.primaryButton(t, "w-start", "Start", enabled, (_e, cx) => this.start(cx))
          : this.primaryButton(t, "w-next", "Continue", enabled, (_e, cx) => this.next(cx)),
      );

    return v_flex()
      .id("welcome")
      .track_focus(this.handle)
      .size_full()
      .items_center()
      .justify_center()
      .on_key_down((e, cx) => {
        const k = e.key;
        const m = e.modifiers;
        if (k === "escape") return this.done("skipped");
        if (m.control || m.alt || m.platform || k !== "enter") return;
        cx.stop_propagation();
        if (!enabled) return;
        if (last) this.start(cx);
        else this.next(cx);
      })
      .child(
        v_flex()
          .w(640)
          .h(560)
          .flex_none()
          .rounded(14)
          .overflow_hidden()
          .bg(t.popover)
          .child(header)
          .child(progress)
          .child(body)
          .child(footer),
      );
  }
}
