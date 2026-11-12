// The lead's model and reasoning, its context and limits, and the menus and ring that show them.

import { div, svg, image, PathBuilder, Background } from "gpui-kit";
import { v_flex, h_flex, Popover, Slider, SliderState, SliderTrack, SliderIndicator, SliderThumb } from "gpui-base";
import { run, lead as leadOf, shared } from "kompitum";

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

// Tokens as thousands: 271000 is "271K".
function kTokens(tokens) {
  return `${Math.round(tokens / 1000)}K`;
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

  resetEffort(cx) {
    this.saveLead({ effort: null }, cx);
    this.effortTouched = false;
    this.effortPreview = null;
    this.effortSlider?.set_value(0);
    cx.notify();
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

  // The models, then (when the program has levels) a hairline and the Reasoning row that opens the effort card.
  modelsView(t, entry, lead) {
    const models = entry?.models ?? [];
    const reasoning = effortIds(entry).length > 0;
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
      .when(reasoning, (el) => el
        .child(div().h(1).my(4).bg(t.border))
        .child(
          h_flex()
            .id("composer-reasoning-row")
            .h(32)
            .px(8)
            .items_center()
            .rounded(7)
            .cursor_pointer()
            .hover((s) => s.bg(t.hover))
            .on_click((_e, cx) => {
              this.pickerView = "effort";
              this.ensureEffortSlider(entry);
              cx.notify();
            })
            .child(div().flex_1().text_size(13).text_color(t.foreground).child("Reasoning"))
            .child(h_flex()
              .gap(4)
              .items_center()
              .child(div().text_size(12.5).text_color(t.dim).child(effortWord(lead.effort) ?? "Default"))
              .child(div().text_size(12.5).text_color(t.dim).child("›")))))
      .when(this.leadPending().pending, (el) => el.child(this.restartStrip(t)));
  },

  // The effort card: a top row (back to the models, and the reset when a level is saved), then the body.
  effortView(t, entry, lead) {
    return v_flex()
      .id("composer-effort")
      .w_full()
      .gap(10)
      .child(
        h_flex()
          .w_full()
          .h(24)
          .items_center()
          .justify_between()
          .child(
            h_flex()
              .id("composer-effort-model")
              .items_center()
              .cursor_pointer()
              .text_size(12.5)
              .text_color(t.muted)
              .hover((s) => s.text_color(t.foreground))
              .on_click((_e, cx) => {
                this.pickerView = "models";
                cx.notify();
              })
              .child("‹ ")
              .child(lead.model ? modelLabel(entry, lead.model) : ""),
          )
          .when(Boolean(lead.effort), (el) => el.child(
            h_flex()
              .id("composer-effort-reset")
              .w(24)
              .h(24)
              .items_center()
              .justify_center()
              .rounded(6)
              .cursor_pointer()
              .text_color(t.muted)
              .hover((s) => s.bg(t.hover).text_color(t.foreground))
              .tooltip("Use Claude Code's own setting")
              .on_click((_e, cx) => this.resetEffort(cx))
              .child(svg("mods/composer/rotate-ccw.svg").size(14)),
          )),
      )
      .child(this.effortBody(t, entry, lead))
      .when(this.leadPending().pending, (el) => el.child(this.restartStrip(t)));
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
      .px(8)
      .gap(5)
      .items_center()
      .rounded(7)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .tooltip(pending ? "Changes wait for the lead's restart" : "The lead's model and reasoning");
    // The icon sits 6 px left of the name.
    let name = h_flex().gap(6).items_center();
    if (icon) name = name.child(icon);
    if (model) {
      trigger = trigger.child(name.child(div().text_size(12.5).font_medium().text_color(t.foreground).child(model)));
      if (level) trigger = trigger.child(div().text_size(12.5).text_color(t.dim).child(level));
    } else {
      trigger = trigger.child(name.child(div().text_size(12.5).text_color(t.muted).child("Choose a model")));
    }
    if (pending) trigger = trigger.child(div().id("composer-model-pending").w(6).h(6).rounded(3).bg(t.working));
    trigger = trigger.child(svg("mods/workspace-chrome/chevron-down.svg").size(11).text_color(t.dim));
    const effort = this.pickerView === "effort" && effortIds(entry).length > 0;
    // The effort card is 280 wide with 16 px padding; the models list stays 260 wide with 6.
    const content = v_flex()
      .when(effort, (el) => el.w(280).p(16).rounded(14))
      .when(!effort, (el) => el.w(260).p(6).rounded(12))
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .child(effort
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
      .tooltip(`Context ${Math.round(fraction * 100)}%`)
      .child(drawing);
  },

  // Compact now is live only while the lead waits, no card is open, Claude is not compacting and the count is known.
  compactReady() {
    return (
      this.state === "waiting"
      && !this.ask
      && leadOf()?.compacting !== true
      && typeof leadUsage()?.context_tokens === "number"
    );
  },

  compactNow(cx) {
    this.usageOpen = false;
    run("lead.compact", { terminal: this.terminal });
    cx.notify();
  },

  // The card the ring opens: the context, one row per present limit, and Compact now.
  usageCard(t, usage, windows) {
    const compact = this.compactReady();
    const rows = windows.map((w) => {
      const when = resetWords(w.resets);
      const over = w.pct >= 100;
      const words = over
        ? (when ? `Used up · back ${when}` : "Used up")
        : (when ? `${Math.round(w.pct)}% · resets ${when}` : `${Math.round(w.pct)}%`);
      return h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .child(div().text_size(12.5).text_color(t.muted).child(w.long))
        .child(div().text_size(12.5).text_color(over ? t.blocked : t.foreground).child(words));
    });
    return v_flex()
      .w(260)
      .p(12)
      .gap(8)
      .rounded(10)
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .child(
        v_flex()
          .gap(4)
          .child(
            h_flex()
              .w_full()
              .justify_between()
              .items_center()
              .child(div().text_size(12.5).text_color(t.muted).child("Context"))
              .child(
                div()
                  .text_size(12.5)
                  .text_color(t.foreground)
                  .child(`${kTokens(usage.context_tokens)} of ${kTokens(usage.compact_at)}`),
              ),
          )
          .child(div().text_size(12).text_color(t.dim).child("Kompitum tidies up by itself while you're away.")),
      )
      .children(rows)
      .child(
        h_flex()
          .id("composer-compact-now")
          .w_full()
          .h(30)
          .rounded(8)
          .border_1()
          .border_color(t.border)
          .items_center()
          .justify_center()
          .when(compact, (el) => el.cursor_pointer().hover((s) => s.bg(t.hover)).on_click((_e, cx) => this.compactNow(cx)))
          .child(div().text_size(12.5).text_color(compact ? t.foreground : t.dim).child("Compact now")),
      );
  },

  // The ring and the card it opens: a popover whose trigger is the ring.
  usageControl(t, usage, fraction, windows) {
    return Popover.new("composer-usage-card")
      .anchor("bottom_right")
      .open(Boolean(this.usageOpen))
      .on_open_change((open, cx) => {
        this.usageOpen = open;
        cx.notify();
      })
      .trigger(this.usageRing(t, fraction))
      .content(this.usageCard(t, usage, windows));
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
