// Add a machine: the hosts in ~/.ssh/config to pick from (or type one),
// each added with one click (`kom machine add`). When the machine needs
// something first (kompitum-core installed there, a password, a new host
// key), the picker offers to finish it in a terminal where the Owner can
// answer. Machines that need no answers are added quietly. Opened as a dialog
// (`mod.open machine-add`); Settings › Machines and the welcome open it.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { kom, exec, home, readTextFile, theme, props, closeOverlay, run } from "kompitum";

const I = (name) => `mods/workspace-chrome/${name}.svg`;

// The program that adds a machine: on this computer (`machine: ""`) the window
// runs the core the app runs for `kom`, wherever the terminal's PATH points.
const KOM = "kom";
const firstLine = (v) => String(v ?? "").split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";

// An Include's files, as ssh finds them: ~ is home, a relative path is
// under ~/.ssh, and a pattern matches files (through sh; none where there's no sh).
async function includeFiles(value) {
  const paths = (value.match(/"[^"]*"|\S+/g) ?? [])
    .map((p) => p.replace(/^"|"$/g, ""))
    .map((p) => (p.startsWith("~/") ? `${home()}/${p.slice(2)}` : p.startsWith("/") || /^[a-z]:[\\/]/i.test(p) ? p : `${home()}/.ssh/${p}`));
  const files = [];
  for (const p of paths) {
    if (!/[*?[]/.test(p)) {
      files.push(p);
      continue;
    }
    const r = await exec("sh", ["-c", 'for f in $1; do [ -f "$f" ] && printf "%s\\n" "$f"; done', "sh", p]).catch(() => null);
    files.push(...(r?.stdout ?? "").split("\n").filter(Boolean));
  }
  return files;
}

// Host entries people can pick: named ones, not patterns, from the file
// and the files it includes (ssh stops 16 deep).
async function parseConfig(text, hosts = [], depth = 0) {
  let cur = null;
  for (const raw of (text ?? "").split("\n")) {
    const m = raw.trim().match(/^([^\s#=]+)\s*=?\s*(.*)$/);
    if (!m) continue;
    const [key, value] = [m[1].toLowerCase(), m[2].trim()];
    if (key === "host" || key === "match") {
      const names = key === "host" ? value.split(/\s+/).filter((h) => h && !/[*?!]/.test(h) && !hosts.some((x) => x.alias === h || x.aliases.includes(h))) : [];
      cur = names.length ? { alias: names[0], aliases: names.slice(1), hostname: null, user: null } : null;
      if (cur) hosts.push(cur);
    } else if (key === "include" && depth < 16) {
      for (const f of await includeFiles(value)) await parseConfig(await readTextFile(f).catch(() => null), hosts, depth + 1);
    } else if (cur && key === "hostname") cur.hostname = value;
    else if (cur && key === "user") cur.user = value;
  }
  return hosts;
}

// Whether adding stopped for something only a person at a terminal can answer.
function needsAnswer(why) {
  return (
    ["kom_missing", "incompatible", "login_needed", "auth_failed", "host_key"].includes(why.kind) ||
    /password|passphrase|host key|permission denied|login/i.test(why.message ?? "")
  );
}

// One ssh target as one shell word, for the command the button runs.
function shellQuote(target) {
  return /^[a-z0-9_@%+=:,./-]+$/i.test(target) ? target : "'" + target.replace(/'/g, "'\\''") + "'";
}

export default class MachineAdd extends View {
  init(_props, cx) {
    this.hosts = null;
    this.added = [];
    this.state = {}; // target -> "adding" | pending guidance | error text
    this.pending = new Set();
    this.refreshing = false;
    this.typed = "";
    this.input = InputState.new({ placeholder: "user@host" });
    this.input.on("change", (_e, cx) => { this.typed = this.input.value(); cx.notify(); });
    this.input.on("submit", (_e, cx) => this.add(this.typed, cx));
    this.inputFocused = true;
    this.input.on("focus", (_e, cx) => { this.inputFocused = true; cx.notify(); });
    this.input.on("blur", (_e, cx) => { this.inputFocused = false; cx.notify(); });
    this.input.focus();
    cx.timer.after(600, () => this.input.focus());
    this.macs = {}; // target -> { name, core } for an added Mac
    this.opened = {};
    this.handle = cx.focus_handle();
    cx.spawn(async (cx) => {
      this.hosts = await parseConfig(await readTextFile(`${home()}/.ssh/config`).catch(() => null));
      await this.refresh();
      cx.notify();
    });
    cx.timer.every(3000, (cx) => {
      if (!this.pending.size || this.refreshing) return;
      this.refreshing = true;
      cx.spawn(async (cx) => {
        try {
          await this.refresh();
          cx.notify();
        } finally {
          this.refreshing = false;
        }
      });
    });
  }

  async refresh() {
    const r = await kom(["machine", "ls", "--json"]).catch(() => null);
    try {
      this.added = r?.code === 0 ? (JSON.parse(r.stdout).machines ?? []).filter((m) => !m.temporary) : [];
    } catch {
      this.added = [];
    }
    for (const target of this.pending) {
      if (!this.isAdded(target)) continue;
      this.pending.delete(target);
      delete this.state[target];
      if (this.typed.trim() === target) {
        this.input.set_value("");
        this.typed = "";
      }
      await this.checkMac(target);
    }
  }

  isAdded(target) {
    return this.added.some((m) => m.target === target || m.name === target || m.host === target);
  }

  add(target, cx) {
    target = target.trim();
    if (!target || this.state[target] === "adding") return;
    this.pending.delete(target);
    this.state[target] = "adding";
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await kom(["machine", "add", target, "--json"]).catch((e) => ({ code: 1, stderr: String(e), stdout: "" }));
      let why = null;
      try {
        why = r.code === 0 ? null : (JSON.parse(r.stdout || r.stderr).error ?? { message: (r.stderr || r.stdout).trim() });
      } catch {
        why = { message: (r.stderr || r.stdout || "it didn't answer").trim() };
      }
      if (!why) {
        delete this.state[target];
        this.input.set_value("");
        this.typed = "";
        await this.refresh();
        await this.checkMac(target);
      } else if (needsAnswer(why)) {
        // Kept as the sentence the old picker printed (the JS tests and the
        // tooltip read it); the render shows the short line and the button.
        const quoted = shellQuote(target);
        this.pending.add(target);
        this.state[target] = `Couldn't add ${target} here: it needs you to answer something (a password, a new host key, or installing kompitum-core there). Run \`kom machine add ${quoted}\` in a terminal. \`kom machine --help\` shows the options.`;
      } else this.state[target] = why.message || "it didn't work";
      await this.refresh();
      cx.notify();
    });
  }

  // macOS asks on its own screen before anything reads Desktop, Documents
  // or Downloads, and the asking program waits: say so once, with the way
  // to allow it for good.
  async checkMac(target) {
    const m = this.added.find((x) => x.target === target || x.name === target);
    if (!m) return;
    const r = await exec("sh", ["-c", 'uname; command -v kompitum-core || echo "$HOME/.local/bin/kompitum-core"'], { on: m.name }).catch(() => null);
    const [os, core] = (r?.stdout ?? "").trim().split("\n");
    if (os === "Darwin") this.macs[target] = { name: m.name, core: core ?? "~/.local/bin/kompitum-core" };
  }

  macCard(t, target) {
    const mac = this.macs[target];
    return v_flex()
      .mx(10)
      .mb(8)
      .p(12)
      .gap(8)
      .rounded(10)
      .bg(t.blocked + "14")
      .border_1()
      .border_color(t.blocked + "40")
      .child(div().text_size(12.8).font_semibold().text_color(t.foreground).child(`${mac.name} is a Mac: allow its folders once`))
      .child(
        div()
          .text_size(12.5)
          .line_height("18px")
          .text_color(t.muted)
          .child("macOS asks on the Mac's own screen before an agent reads Desktop, Documents or Downloads, and waits until someone answers. Give kompitum-core Full Disk Access there and it never asks again: in the list, press + and pick this file:"),
      )
      .child(div().px(8).py(4).rounded(6).bg(t.background).border_1().border_color(t.border).text_size(12).text_color(t.foreground).child(mac.core))
      .child(
        h_flex()
          .gap(8)
          .items_center()
          .child(
            div()
              .id(`ma-mac-${target}`)
              .h(28)
              .px(12)
              .flex()
              .items_center()
              .rounded(7)
              .bg(t.solid)
              .text_color(t.solid_foreground)
              .text_size(12)
              .font_medium()
              .cursor_pointer()
              .hover((s) => s.opacity(0.88))
              .on_click((_e, cx) => {
                exec("open", ["x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"], { on: mac.name }).catch(() => null);
                this.opened[target] = true;
                cx.notify();
              })
              .child(`Open the setting on ${mac.name}`),
          )
          .when(Boolean(this.opened[target]), (el) => el.child(div().text_size(12).text_color(t.dim).child("It's open on the Mac's screen."))),
      );
  }

  close() {
    closeOverlay(props().overlay);
  }

  key(e, cx) {
    if (e.key === "escape") { this.close(); cx.stop_propagation(); }
  }

  // "It needs you to answer something": one short line and the way to do it
  // where the questions can be asked.
  answer(t, target) {
    return v_flex()
      .gap(8)
      .child(
        div()
          .id(`ma-message-${target}`)
          .text_size(12)
          .line_height("16px")
          .text_color(t.muted)
          .child(`${target} needs an answer from you: a password, a new host key, or installing Kompitum there.`),
      )
      .child(
        div()
          .id(`ma-finish-${target}`)
          .h(26)
          .px(10)
          .flex()
          .items_center()
          .rounded(7)
          .bg(t.solid)
          .text_color(t.solid_foreground)
          .text_size(12)
          .font_medium()
          .cursor_pointer()
          .hover((s) => s.opacity(0.88))
          .tooltip(`Runs \`kom machine add ${shellQuote(target)}\` in a new tab.`)
          .on_click((_e, cx) => {
            run("terminal.open", {
              argv: [KOM, "machine", "add", target],
              place: "tab",
              stay: true,
              machine: "",
              name: `Add ${target}`,
            });
            this.close();
            cx.stop_propagation();
          })
          .child("Finish in a terminal"),
      );
  }

  // Anything else that went wrong: the first meaningful line, the whole text on hover.
  error(t, target) {
    const text = String(this.state[target]);
    return div()
      .id(`ma-message-${target}`)
      .text_size(12)
      .line_height("16px")
      .max_h(32)
      .overflow_hidden()
      .text_color(t.blocked)
      .tooltip(text)
      .child(firstLine(text) || text);
  }

  reply(t, target) {
    const st = this.state[target];
    if (!st || st === "adding") return null;
    return this.pending.has(target) ? this.answer(t, target) : this.error(t, target);
  }

  row(t, target, label, sub) {
    const st = this.state[target];
    const done = this.isAdded(target);
    const answer = this.pending.has(target);
    const right = done
      ? h_flex().gap(5).items_center().text_size(12).text_color(t.ansi2).child(svg(I("circle-check")).size(13)).child("Added")
      : st === "adding"
        ? div().id(`ma-add-${target}`).h(26).px(8).flex().items_center().rounded(6).text_size(12).text_color(t.dim).child("Adding…")
        : answer
          ? div()
          : div()
              .id(`ma-add-${target}`)
              .h(26)
              .px(8)
              .flex()
              .items_center()
              .rounded(6)
              .text_size(12)
              .font_medium()
              .text_color(t.foreground)
              .cursor_pointer()
              .hover((s) => s.bg(t.hover))
              .on_click((_e, cx) => this.add(target, cx))
              .child("Add");
    const message = this.reply(t, target);
    return v_flex()
      .flex_none()
      .child(
        h_flex()
          .id(`ma-row-${target}`)
          .h(40)
          .px(10)
          .gap(10)
          .items_center()
          .rounded(8)
          .child(svg(I("server")).size(15).text_color(t.muted))
          .child(div().flex_none().text_size(13.5).text_color(t.foreground).child(label))
          .child(div().flex_1().min_w(0).truncate().text_size(12).text_color(t.dim).child(sub ?? ""))
          .child(right),
      )
      .when(Boolean(message), (el) => el.child(div().px(35).pb(8).child(message)))
      .when(Boolean(this.macs[target]), (el) => el.child(this.macCard(t, target)));
  }

  render() {
    const t = theme();
    const hosts = this.hosts ?? [];
    const typed = this.typed.trim();
    const typedMessage = typed ? this.reply(t, typed) : null;
    return v_flex()
      .id("machine-add")
      .track_focus(this.handle)
      .on_key_down((e, cx) => this.key(e, cx))
      .w_full()
      .min_h(0)
      .p(18)
      .bg(t.popover)
      .text_color(t.foreground)
      .child(h_flex().items_center().child(div().flex_1().text_size(15).font_semibold().child("Add a machine")).child(div().text_size(11).text_color(t.dim).child("Esc")))
      .child(
        div()
          .pt(6)
          .text_size(12.8)
          .line_height("19px")
          .text_color(t.muted)
          .child("Kompitum reaches it over SSH. Agents there keep working while this computer sleeps."),
      )
      .child(div().pt(16).pb(6).text_size(11.5).font_medium().text_color(t.dim).child(hosts.length ? "From your ssh config" : "Your ssh config lists no hosts"))
      .child(
        v_flex()
          .id("ma-hosts")
          .max_h(280)
          .min_h(0)
          .flex_shrink(1)
          .overflow_y_scroll()
          .children(
            this.hosts === null
              ? [div().px(10).py(8).text_size(12.5).text_color(t.dim).child("Reading ~/.ssh/config…")]
              : hosts.map((h) => this.row(t, h.alias, h.alias, [h.user, h.hostname].filter(Boolean).join("@") || (h.aliases.length ? `also ${h.aliases.join(", ")}` : ""))),
          ),
      )
      .child(div().pt(14).pb(6).text_size(11.5).font_medium().text_color(t.dim).child("Or type its address"))
      .child(
        h_flex()
          .gap(8)
          .child(
            Input.new(this.input)
              .flex_1()
              .h(34)
              .px(10)
              .gap(6)
              .items_center()
              .rounded(8)
              .border_1()
              .border_color(this.inputFocused ? t.accent : t.input)
              .bg(t.background)
              .text_size(13)
              .text_color(t.foreground),
          )
          .child(
            div()
              .id("ma-add-typed")
              .h(34)
              .px(14)
              .flex()
              .items_center()
              .rounded(8)
              .bg(t.solid)
              .text_color(t.solid_foreground)
              .text_size(12.5)
              .font_medium()
              .when(!typed, (el) => el.opacity(0.4))
              .when(Boolean(typed), (el) => el.cursor_pointer().on_click((_e, cx) => this.add(this.typed, cx)))
              .child("Add"),
          ),
      )
      .when(Boolean(typedMessage), (el) => el.child(div().pt(8).child(typedMessage)));
  }
}
