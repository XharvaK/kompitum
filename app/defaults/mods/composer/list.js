// The composer's `/` list: the lead's commands, drawn above the box.

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { leadCommands } from "kompitum";
import { byteIndex, charIndex } from "./util.js";

const LIST_ROWS = 8;

export default {
  // The text before the cursor ends in `/fragment`: the fragment, else null.
  listFragment(text, cursor) {
    const before = String(text ?? "").slice(0, cursor);
    const match = before.match(/(^|\s)\/([^\s\/]*)$/);
    return match ? match[2] : null;
  },

  // selected_range() counts UTF-8 bytes; the editor and this script count UTF-16 indexes.
  listCursor() {
    const range = this.text?.selected_range?.();
    return charIndex(this.text?.value?.() ?? "", range?.start ?? 0);
  },

  // A change to the box: open the list once for the opening, or filter what it already holds.
  refreshList(cx) {
    const fragment = this.listFragment(this.text?.value?.() ?? "", this.listCursor());
    if (fragment === null) {
      this.closeList(cx);
      return;
    }
    if (!this.listOpen) {
      this.listOpen = true;
      this.listCommands = [];
      this.listMatches = [];
      this.listIndex = 0;
      this.listWindowStart = 0;
      const scan = ++this.listScan;
      this.applyKeyContext();
      // A task's own cx, never the one from the change: the answer lands after this call returns.
      cx.spawn(async (cx) => {
        try {
          const list = await leadCommands();
          if (!this.listOpen || scan !== this.listScan) return;
          this.listCommands = Array.isArray(list) ? list : [];
          this.filterList(cx);
        } catch {
          if (scan === this.listScan) this.closeList(cx);
        }
      });
      cx.notify();
      return;
    }
    this.filterList(cx);
  },

  // Names that start with the fragment first, then names whose part after the last `:` does;
  // case-insensitive and alphabetical within each group. No other matching.
  filterList(cx) {
    const fragment = this.listFragment(this.text?.value?.() ?? "", this.listCursor());
    if (fragment === null) {
      this.closeList(cx);
      return;
    }
    const needle = fragment.toLowerCase();
    const starting = [];
    const after = [];
    for (const entry of this.listCommands) {
      const name = String(entry?.name ?? "");
      if (!name) continue;
      const lower = name.toLowerCase();
      if (lower.startsWith(needle)) starting.push(entry);
      else if (lower.slice(lower.lastIndexOf(":") + 1).startsWith(needle)) after.push(entry);
    }
    const byName = (a, b) =>
      String(a.name).toLowerCase().localeCompare(String(b.name).toLowerCase())
      || String(a.name).localeCompare(String(b.name));
    starting.sort(byName);
    after.sort(byName);
    const matches = starting.concat(after);
    if (matches.length === 0) {
      this.closeList(cx);
      return;
    }
    const same = matches.length === this.listMatches.length
      && matches.every((entry, index) =>
        entry.name === this.listMatches[index].name && entry.source === this.listMatches[index].source);
    this.listMatches = matches;
    // A new set of matches starts at its first row; the same set keeps the highlight where it was.
    if (!same) this.listIndex = 0;
    this.listIndex = Math.min(this.listIndex, matches.length - 1);
    this.clampListWindow();
    cx.notify();
  },

  // The eight-row window slides just enough that the highlighted row stays in it.
  clampListWindow() {
    const last = Math.max(0, this.listMatches.length - LIST_ROWS);
    let start = Number(this.listWindowStart) || 0;
    if (this.listIndex < start) start = this.listIndex;
    if (this.listIndex >= start + LIST_ROWS) start = this.listIndex - LIST_ROWS + 1;
    this.listWindowStart = Math.max(0, Math.min(start, last));
  },

  closeList(cx) {
    if (!this.listOpen) return;
    this.listOpen = false;
    this.listCommands = [];
    this.listMatches = [];
    this.listWindowStart = 0;
    this.listScan += 1;
    this.applyKeyContext();
    if (cx) cx.notify();
  },

  // The textarea's own context decides which keys outrank the editor's.
  applyKeyContext() {
    if (!this.text) return;
    this.text.set_key_context(this.listOpen ? "ComposerList" : this.historyMode ? "ComposerHistory" : null);
  },

  listKey(direction, cx) {
    if (!this.listOpen || this.listMatches.length === 0) return;
    const count = this.listMatches.length;
    this.listIndex = (this.listIndex + direction + count) % count;
    this.clampListWindow();
    cx.notify();
  },

  // Replace the typed `/fragment` with `/name`, plus the space a hint's argument needs.
  insertList(entry, trailing, cx) {
    if (!entry) return;
    const text = this.text?.value?.() ?? "";
    const cursorBytes = this.text?.selected_range?.()?.start ?? 0;
    const cursor = charIndex(text, cursorBytes);
    const match = String(text).slice(0, cursor).match(/(^|\s)\/([^\s\/]*)$/);
    if (!match) {
      this.closeList(cx);
      return;
    }
    const startBytes = byteIndex(text, match.index + match[1].length);
    const endBytes = byteIndex(text, cursor);
    // The fragment's byte offsets converted back for the binding, whose set_selected_range
    // counts UTF-16 indexes even though the state's selected_range counts bytes.
    const start = charIndex(text, startBytes);
    const end = charIndex(text, endBytes);
    const insertion = `/${entry.name}${trailing ? " " : ""}`;
    this.text.set_selected_range({ start, end });
    this.text.replace(insertion);
    this.closeList(cx);
    this.text.focus();
    cx.notify();
  },

  listInsert(cx) {
    this.insertList(this.listMatches[this.listIndex], true, cx);
  },

  listPick(cx) {
    const entry = this.listMatches[this.listIndex];
    if (!entry) return;
    this.insertList(entry, Boolean(entry.hint), cx);
    // A command that takes no argument sends itself; one with a hint waits for its argument.
    if (!entry.hint) this.send(cx);
  },

  listClick(index, cx) {
    if (index < 0 || index >= this.listMatches.length) return;
    this.listIndex = index;
    this.insertList(this.listMatches[index], true, cx);
  },

  listMenu(t, cx) {
    if (!this.listOpen || this.listMatches.length === 0) return null;
    const start = Math.max(0, Math.min(this.listWindowStart ?? 0, this.listMatches.length - 1));
    const rows = this.listMatches.slice(start, start + LIST_ROWS);
    return v_flex()
      .id("composer-list")
      .absolute()
      .left(0)
      .right(0)
      .bottom("100%")
      .mb(4)
      .p(6)
      .rounded(12)
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .children(rows.map((entry, offset) => {
        const index = start + offset;
        const on = index === this.listIndex;
        return h_flex()
          .id(`composer-list-${index}`)
          .h(28)
          .px(8)
          .gap(8)
          .items_center()
          .rounded(8)
          .when(on, (el) => el.bg(t.selected))
          .hover((s) => s.bg(t.hover))
          .on_hover((hovered, cx) => {
            if (hovered && this.listIndex !== index) {
              this.listIndex = index;
              this.clampListWindow();
              cx.notify();
            }
          })
          .on_click((_e, cx) => this.listClick(index, cx))
          .child(div().flex_none().text_size(13).text_color(t.foreground).child(`/${entry.name}`))
          .when(Boolean(entry.hint), (el) =>
            el.child(div().flex_none().text_size(13).text_color(t.dim).child(entry.hint)))
          .child(
            div()
              .flex_1()
              .min_w(0)
              .truncate()
              .text_size(12)
              .text_color(t.muted)
              .child(String(entry.description ?? "")),
          );
      }));
  },
};
