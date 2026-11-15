// The desktop's icons, down the left edge of the teal: My Computer, the
// Recycle Bin, every space as a folder (the shown one selected, its agents'
// state as a badge), the games, Network Neighborhood. One click opens (Win98's
// "single-click to open" web style); right-click for each icon's menu, or the
// desktop's.

import { div } from "gpui-kit";
import { v_flex } from "gpui-base";
import { spaces, run, text } from "kompitum";
import { C, unit, root, icon, openMenu, openDialog, perform, sfx } from "./kit.js";
import { closeSpaceAsk } from "./taskbar.js";

export default class Desktop {
  init() {
    this.sel = null;
  }

  item(u, key, ico, text, o = {}) {
    const on = o.selected ?? this.sel === key;
    const badge = o.badge;
    return v_flex()
      .id(`dk-${key}`)
      .flex_none()
      .w(u(76))
      .pt(u(4))
      .pb(u(6))
      .items_center()
      .gap(u(3))
      .on_mouse_down("left", (_e, cx) => {
        cx.stop_propagation();
        this.sel = key;
        cx.notify();
      })
      .on_click((_e, cx) => {
        o.open?.();
        cx.notify();
      })
      .on_mouse_down("right", (_e, cx) => {
        cx.stop_propagation();
        this.sel = key;
        if (o.menu) openMenu(o.menu());
        cx.notify();
      })
      .child(
        div()
          .relative()
          .child(icon(u, ico, 32))
          .when(on, (el) => el.child(div().absolute().top(0).left(0).size_full().bg("#00008060")))
          .when(Boolean(badge), (el) =>
            el.child(
              div()
                .absolute()
                .left(u(-2))
                .bottom(u(-2))
                .child(icon(u, badge, 16))
                .when(badge === "warn-badge", (b) => b.loop("opacity", { from: 0, to: 1, period: 900 })),
            ),
          ),
      )
      .child(
        div()
          .max_w(u(74))
          .px(u(1))
          .text_center()
          .text_color(C.white)
          .border(u(1))
          .border_color(on ? "#ffff00" : "#00000000")
          .when(on, (el) => el.bg(C.navy))
          .child(text),
      );
  }

  render(_cx) {
    const u = unit();
    const all = spaces();
    const badgeOf = (s) => (s.status === "blocked" ? "warn-badge" : s.status === "working" ? "hourglass" : s.status === "done" ? "check-badge" : null);
    const items = [
      this.item(u, "computer", "computer", text("win98.my-computer"), { open: () => openDialog("mycomputer", {}) }),
      this.item(u, "bin", "bin", text("win98.recycle-bin"), { open: () => openDialog("bin", {}) }),
      ...all.map((s) =>
        this.item(u, s.id, s.machine ? "netfolder" : s.active ? "folder-open" : "folder", s.name, {
          selected: s.active,
          badge: badgeOf(s),
          open: () => run("space.select", { id: s.id }),
          menu: () => [
            { label: text("win98.open"), bold: true, act: { run: ["space.select", { id: s.id }] } },
            { label: text("win98.new-tab-menu"), act: { dialog: { kind: "prompt", title: text("win98.new-tab"), text: text("win98.new-tab-in", { space: s.name }), icon: "dos", command: { name: "tab.new", args: { space: s.id } } } } },
            { sep: true },
            { label: text("win98.rename"), act: { dialog: { kind: "prompt", title: text("win98.rename-title"), text: text("win98.rename-space-text"), icon: "folder", value: s.name, command: { name: "space.rename", args: { id: s.id } } } } },
            { label: text("win98.delete"), act: { dialog: { kind: "confirm", ...closeSpaceAsk(s) } } },
            { sep: true },
            {
              label: text("win98.properties"),
              act: {
                dialog: {
                  kind: "message",
                  title: text("win98.properties-title", { name: s.name }),
                  icon: "folder",
                  text: text("win98.space-properties", { location: s.machine ?? text("win98.this-computer"), folder: s.folder || "~", branch: s.branch ?? text("win98.none"), tabs: s.tabs.length, status: s.status ?? text("win98.idle") }),
                },
              },
            },
          ],
        }),
      ),
      this.item(u, "mines", "mines", text("win98.game-mines"), { open: () => perform({ app: "mines" }) }),
      this.item(u, "sol", "sol", text("win98.game-solitaire"), { open: () => perform({ app: "sol" }) }),
      this.item(u, "net", "network", text("win98.network-neighborhood"), { open: () => openDialog("network", {}) }),
    ];
    return root(
      u,
      v_flex()
        .id("desktop")
        .size_full()
        .pt(u(6))
        .items_center()
        .overflow_y_scroll()
        .on_mouse_down("left", (_e, cx) => {
          this.sel = null;
          cx.notify();
        })
        .on_mouse_down("right", () => {
          sfx("menu");
          openMenu([
            { label: text("win98.arrange-icons"), sub: [{ label: text("win98.by-name"), disabled: true }, { label: text("win98.by-type"), disabled: true }, { label: text("win98.auto-arrange"), checked: true, disabled: true }] },
            { label: text("win98.line-up-icons"), disabled: true },
            { sep: true },
            { label: text("win98.refresh"), act: { run: ["layout.switch", { name: "win98" }] } },
            { sep: true },
            { label: text("win98.paste"), disabled: true },
            { label: text("win98.paste-shortcut"), disabled: true },
            { sep: true },
            {
              label: text("win98.new"),
              sub: [
                { label: text("win98.new-space-menu"), icon: "folder", act: { dialog: { kind: "prompt", title: text("win98.new-space"), text: text("win98.new-space-text"), icon: "folder", machines: true, command: { name: "space.new", args: {} } } } },
                { label: text("win98.new-tab-menu"), icon: "dos", act: { dialog: { kind: "prompt", title: text("win98.new-tab"), text: text("win98.new-tab-text"), icon: "dos", command: { name: "tab.new", args: {} } } } },
              ],
            },
            { sep: true },
            { label: text("win98.properties"), act: { dialog: { kind: "display" } } },
          ]);
        }),
    ).children(items);
  }
}
