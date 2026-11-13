// The lead's model and reasoning, its context and limits, and the menus and ring that show them.

import { div, svg, image, PathBuilder, Background } from "gpui-kit";
import { v_flex, h_flex, Popover, Slider, SliderState, SliderTrack, SliderIndicator, SliderThumb } from "gpui-base";
import { run, lead as leadOf, shared, openOverlay } from "kompitum";
import { hairline } from "./util.js";

export function modelLabel(entry, id) {
  return entry?.models?.find((model) => model.id === id)?.label ?? id;
}

const EFFORT_WORDS = { low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max" };

const EFFORT_LINES = {
  low: "Quick answers, least thinking.",
  medium: "Balanced speed and depth.",
  high: "Thinks it through.",
  xhigh: "Thinks longer on hard problems.",
  max: "Thinks as long as it needs.",
};

function cap(text) {
  const value = String(text ?? "");
  return value ? value.charAt(0).toUpperCase() + value.slice(1) : value;
}

function effortWord(effort) {
  if (!effort) return null;
  return EFFORT_WORDS[effort] ?? cap(effort);
}

// The one line under the level's word: what the level does, or what having no saved level means.
function effortLine(effort) {
  if (!effort) return "Claude Code's own setting.";
  return EFFORT_LINES[effort] ?? "";
}

export function effortIds(entry) {
  return (entry?.efforts ?? [])
    .map((level) => (typeof level === "string" ? level : level?.id))
    .filter(Boolean);
}

// The lead's context and Claude's limits, from lead().usage. Times are Unix seconds; every field may be missing.
const WEEKDAY_WORDS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const MONTH_WORDS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

const pad2 = (n) => String(n).padStart(2, "0");

export function leadUsage() {
  const usage = leadOf()?.usage;
  return usage && typeof usage === "object" ? usage : null;
}

// The clock time when a reset is today, the weekday within a week, else "23 Nov" (the rule of Settings > Agents).
function resetWords(seconds) {
  if (typeof seconds !== "number" || !Number.isFinite(seconds) || seconds <= 0) return null;
  const when = new Date(seconds * 1000);
  const now = new Date();
  const midnight = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((midnight(when) - midnight(now)) / 86400000);
  if (days === 0) return `${pad2(when.getHours())}:${pad2(when.getMinutes())}`;
  if (Math.abs(days) < 7) return WEEKDAY_WORDS[when.getDay()];
  return `${when.getDate()} ${MONTH_WORDS[when.getMonth()]}`;
}

// The present window with the higher share, when it is at 75 or more; null otherwise.
function limitLine(windows) {
  const top = windows.reduce((best, w) => (!best || w.pct > best.pct ? w : best), null);
  if (!top || top.pct < 75) return null;
  const when = resetWords(top.resets);
  if (top.pct >= 100) return { text: when ? `${top.short} used up · back ${when}` : `${top.short} used up`, blocked: true };
  const pct = Math.round(top.pct);
  return { text: when ? `${top.short} ${pct}% · resets ${when}` : `${top.short} ${pct}%`, blocked: false };
}

// The ring's paths, 16 px across: a 2 px stroke on a circle of radius 7, drawn from the top, clockwise.
function ringPath(fraction) {
  const c = 8;
  const r = 7;
  const builder = PathBuilder.stroke(2).move_to(c, c - r);
  if (fraction >= 1) return builder.arc_to(r, r, 0, false, true, c, c + r).arc_to(r, r, 0, false, true, c, c - r).build();
  const angle = 2 * Math.PI * Math.min(fraction, 0.999);
  const x = c + r * Math.sin(angle);
  const y = c - r * Math.cos(angle);
  return builder.arc_to(r, r, 0, fraction > 0.5, true, x, y).build();
}

export default {
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
  },

  // What the running lead was started with (`lead.running`, set by the app at each start), when it is this
  // terminal's lead; and whether the saved model or reasoning differs from it, so a change waits for a restart.
  leadPending() {
    const runs = shared("lead.running");
    const started = runs?.terminal === this.terminal ? runs : null;
    const lead = this.person?.lead ?? {};
    const pending = Boolean(started)
      && ((started.model ?? null) !== (lead.model ?? null) || (started.effort ?? null) !== (lead.effort ?? null));
    return { started, pending };
  },

  pickModel(id, cx) {
    this.saveLead({ model: id }, cx);
    cx.notify();
  },

  restartNow(cx) {
    run("lead.restart", { terminal: this.terminal });
    this.modelOpen = false;
    cx.notify();
  },

  // The strip under either view while a saved change waits: Restart now is live only while the lead waits.
  restartStrip(t) {
    const live = this.state !== "working" && !this.ask && leadOf()?.compacting !== true;
    let button = h_flex()
      .id("composer-restart")
      .h(26)
      .px(10)
      .items_center()
      .rounded(7)
      .text_size(12)
      .font_medium();
    if (live) {
      button = button
        .bg(t.accent_fill)
        .text_color(t.accent_foreground)
        .cursor_pointer()
        .on_click((_e, cx) => this.restartNow(cx));
    } else {
      button = button.bg(t.hover).text_color(t.dim).tooltip("The lead is busy. Restart when it's done.");
    }
    return h_flex()
      .id("composer-restart-strip")
      .w_full()
      .mt(6)
      .pt(8)
      .px(8)
      .pb(4)
      .gap(8)
      .items_center()
      .border_t_1()
      .border_color(t.border)
      .child(div().flex_1().text_size(11.5).text_color(t.dim).child("Applies when the lead restarts."))
      .child(button.child("Restart now"));
  },

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
  },

  // A slim slider: a 24 px hit area around a 6 px track, a fill in the accent, an 18 px thumb with a ring.
  effortSliderEl(t, entry) {
    const levels = effortIds(entry);
    if (!this.effortSlider || levels.length === 0) return null;
    const preview = this.effortPreview ?? this.person?.lead?.effort ?? null;
    const filledThrough = this.effortTouched && preview && levels.includes(preview)
      ? levels.indexOf(preview)
      : null;
    // The track's bar sits 9 px down the 24 px track, so the fill and the thumb's centre line up with it.
    let indicator = SliderIndicator.new(this.effortSlider)
      .absolute()
      .top(9)
      .left(0)
      .w_full()
      .h(6)
      .rounded(3);
    if (this.effortTouched) {
      indicator = indicator
        .range_style((fill) => fill.rounded(3).bg(t.accent))
        .child(
          SliderThumb.new(this.effortSlider)
            .size(18)
            .rounded(9)
            .bg(t.foreground)
            .border_1()
            .border_color(t.border)
            .mt(-6)
            .ml(-9),
        );
    }
    return Slider.new(this.effortSlider)
      .id("composer-effort-slider")
      .w_full()
      .h(24)
      .mt(8)
      .child(
        SliderTrack.new(this.effortSlider)
          .relative()
          .w_full()
          .h(24)
          .child(div().absolute().top(9).left(0).w_full().h(6).rounded(3).bg(t.hover))
          .child(this.effortDots(t, levels.length, filledThrough))
          .child(indicator),
      );
  },

  // A level picked by its word or by the slider's release: saved now in the picker, held until Start in the first pick.
  pickLevel(level, cx) {
    const index = effortIds(this.leadProgram()).indexOf(level);
    if (index < 0) return;
    this.effortTouched = true;
    this.effortPreview = level;
    this.effortSlider?.set_value(index);
    if (this.saveEffortOnRelease) this.saveLead({ effort: level }, cx);
    cx.notify();
  },

  // The levels' short words under the slider, spread to its ends; the current one is in the foreground.
  effortLevelWords(t, levels, preview) {
    const current = preview && levels.includes(preview) ? preview : null;
    return h_flex()
      .id("composer-effort-levels")
      .w_full()
      .mt(6)
      .justify_between()
      .children(levels.map((level) =>
        div()
          .id(`composer-effort-level-${level}`)
          .text_size(11)
          .cursor_pointer()
          .text_color(level === current ? t.foreground : t.dim)
          .hover((s) => s.text_color(t.foreground))
          .on_click((_e, cx) => this.pickLevel(level, cx))
          .child(EFFORT_WORDS[level] ?? cap(level))));
  },

  // The level's word, its one line, the slider and the levels' words. The model picker's card and the first pick share it.
  effortBody(t, entry, lead) {
    const levels = effortIds(entry);
    const preview = this.effortPreview ?? lead.effort ?? null;
    let body = v_flex()
      .w_full()
      .gap(2)
      .child(div().text_size(17).font_semibold().text_color(t.foreground).child(effortWord(preview) ?? "Default"))
      .child(div().text_size(12.5).text_color(t.dim).child(effortLine(preview)));
    const slider = this.effortSliderEl(t, entry);
    if (slider) body = body.child(slider).child(this.effortLevelWords(t, levels, preview));
    return body;
  },

  // The models column: the Model label, then one row per model. The check sits on the saved model.
  modelsView(t, entry, lead) {
    const models = entry?.models ?? [];
    return v_flex()
      .w(220)
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
            el.child(svg("frame/settings/check.svg").size(14).text_color(t.accent)))));
  },

  // The reasoning column: Default (Claude Code's own setting), then the program's levels. A click saves its row
  // and the menu stays open. The check sits on the saved level, or on Default when none is saved.
  levelsView(t, entry, lead) {
    const saved = effortIds(entry).includes(lead.effort) ? lead.effort : null;
    const row = (id, label, level, tooltip) => h_flex()
      .id(id)
      .h(32)
      .px(8)
      .items_center()
      .rounded(7)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .when(Boolean(tooltip), (el) => el.tooltip(tooltip))
      .on_click((_e, cx) => {
        this.saveLead({ effort: level }, cx);
        cx.notify();
      })
      .child(div().flex_1().text_size(13).text_color(t.foreground).child(label))
      .when(level === saved, (el) => el.child(svg("frame/settings/check.svg").size(14).text_color(t.accent)));
    return v_flex()
      .w(150)
      .child(div().px(8).pt(4).pb(2).text_size(11.5).text_color(t.dim).child("Reasoning"))
      .child(row("composer-level-default", "Default", null, "Claude Code's own setting"))
      .children(effortIds(entry).map((level) => row(`composer-level-${level}`, effortWord(level), level, null)));
  },

  // The lead program's icon at 14 px, as the agents list draws it: a one-colour glyph or a picture. It pulses
  // while the lead works and is still otherwise. Nothing when the program has no icon.
  leadIcon(t, entry) {
    if (!entry?.icon) return null;
    const glyph = entry.tint
      ? svg(entry.icon).size(14).flex_none().text_color(t.foreground)
      : image(entry.icon).w(14).h(14).flex_none();
    return div()
      .w(14)
      .h(14)
      .flex_none()
      .child(glyph)
      .when(this.state === "working", (el) => el.loop("opacity", { from: 0.45, to: 1, period: 1200 }));
  },

  // The lead's model and reasoning: models first, then Codex-style effort.
  modelChip(t) {
    const entry = this.leadProgram();
    const lead = this.person?.lead ?? {};
    // The model the lead says it runs; the saved setting's label until it has said.
    const running = leadOf()?.model?.name;
    const model = typeof running === "string" && running.trim() ? running : lead.model ? modelLabel(entry, lead.model) : null;
    // The reasoning the lead runs with (its started setting), and a dot while a saved change waits for its restart.
    const { started, pending } = this.leadPending();
    const level = effortWord(started ? started.effort : lead.effort);
    const icon = this.leadIcon(t, entry);
    let trigger = h_flex()
      .id("composer-model")
      .h(28)
      .px(6)
      .gap(6)
      .items_center()
      .rounded(7)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .tooltip(pending ? "Changes wait for the lead's restart" : "The lead's model and reasoning");
    // The icon sits 6 px left of the name.
    let name = h_flex().gap(6).items_center();
    if (icon) name = name.child(icon);
    if (model) {
      trigger = trigger.child(name.child(div().text_size(12.5).text_color(t.foreground).child(model)));
      if (level) trigger = trigger.child(hairline(t)).child(div().text_size(12.5).text_color(t.muted).child(level));
    } else {
      trigger = trigger.child(name.child(div().text_size(12.5).text_color(t.muted).child("Choose a model")));
    }
    if (pending) trigger = trigger.child(div().id("composer-model-pending").w(6).h(6).rounded(3).bg(t.working));
    // The models and the reasoning levels side by side; the strip under both while a saved change waits.
    const content = v_flex()
      .id("composer-menu")
      .p(6)
      .rounded(12)
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .child(
        h_flex()
          .items_start()
          .child(this.modelsView(t, entry, lead))
          .when(effortIds(entry).length > 0, (el) => el
            .child(div().id("composer-menu-divider").w(1).self_stretch().mx(4).bg(t.border))
            .child(this.levelsView(t, entry, lead))),
      )
      .when(this.leadPending().pending, (el) => el.child(this.restartStrip(t)));
    return Popover.new("composer-model-menu")
      .anchor("bottom_left")
      .open(Boolean(this.modelOpen))
      .on_open_change((open, cx) => {
        this.modelOpen = open;
        if (open) {
          this.effortPreview = lead.effort ?? null;
          this.effortTouched = Boolean(lead.effort);
          this.saveEffortOnRelease = true;
          this.ensureEffortSlider(entry);
        }
        cx.notify();
      })
      .trigger(trigger)
      .content(content);
  },

  // The ring for how full the lead's context is: a track and an arc, 16 px across in a 28 px hit area.
  usageRing(t, fraction) {
    const color = fraction >= 0.95 ? t.blocked : fraction >= 0.75 ? t.working : t.muted;
    let drawing = div()
      .relative()
      .w(16)
      .h(16)
      .flex_none()
      .child(window.paint_path(ringPath(1), Background.solid(t.border)).absolute().top(0).left(0).size_full());
    if (fraction > 0) {
      drawing = drawing.child(
        window.paint_path(ringPath(fraction), Background.solid(color)).absolute().top(0).left(0).size_full(),
      );
    }
    return h_flex()
      .id("composer-usage")
      .w(28)
      .h(28)
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(6)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .tooltip(`Context ${Math.round(fraction * 100)}% · Limits`)
      .on_click(() => openOverlay({ mod: "limits", kind: "popup", width: 340, height: 480, at: "pointer", props: { terminal: this.terminal } }))
      .child(drawing);
  },

  // The limit in words, only when a present window is at 75% or more.
  usageLimit(t, windows) {
    const line = limitLine(windows);
    if (!line) return null;
    return div()
      .id("composer-limit")
      .min_w(0)
      .truncate()
      .text_size(11.5)
      .text_color(line.blocked ? t.blocked : t.working)
      .child(line.text);
  },
};
