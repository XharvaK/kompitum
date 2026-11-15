// The composer's one list, drawn above the box: the lead's `/` commands, the
// `@` folder's files, or the box's own history searched with Ctrl+R.

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { leadCommands, listDir, text } from "kompitum";
import { byteIndex, charIndex } from "./util.js";

const LIST_ROWS = 8;

// A history entry's first line, with the whitespace around it gone.
function firstLine(text) {
  return String(text ?? "").split(/\r?\n/, 1)[0].trim();
}

export default {
  // The text before the cursor ends in `/fragment`: the fragment, else null.
  listFragment(text, cursor) {
    const before = String(text ?? "").slice(0, cursor);
    const match = before.match(/(^|\s)\/([^\s\/]*)$/);
    return match ? match[2] : null;
  },

  // The text before the cursor ends in `@fragment` (at a start or after a space): the match,
  // with the fragment after `@` and where that `@` sits, else null.
  fileFragment(text, cursor) {
    const before = String(text ?? "").slice(0, cursor);
    const match = before.match(/(^|\s)@("[^"]*|[^\s"]*)$/);
    return match ? { fragment: match[2], at: match.index + match[1].length } : null;
  },

  // The fragment after `@` split at its last `/` or `\`: the folder part, with its separator,
  // and the prefix within it.
  fileParts(fragment) {
    const body = String(fragment ?? "").replace(/^"/, "");
    const cut = Math.max(body.lastIndexOf("/"), body.lastIndexOf("\\"));
    return cut < 0
      ? { folder: "", prefix: body }
      : { folder: body.slice(0, cut + 1), prefix: body.slice(cut + 1) };
  },

  // selected_range() counts UTF-8 bytes; the editor and this script count UTF-16 indexes.
  listCursor() {
    const range = this.text?.selected_range?.();
    return charIndex(this.text?.value?.() ?? "", range?.start ?? 0);
  },

  // Whether the panel shows and its keys outrank the editor's.
  listShown() {
    if (!this.listOpen) return false;
    return this.listKind === "history" || this.listMatches.length > 0;
  },

  // A change to the box: open the list once for the opening, or filter what it already holds.
  refreshList(cx) {
    if (this.listOpen && this.listKind === "history") {
      this.filterHistory(cx);
      return;
    }
    const text = this.text?.value?.() ?? "";
    const cursor = this.listCursor();
    const at = this.fileFragment(text, cursor);
    const slash = this.listFragment(text, cursor);
    if (at === null && slash === null) {
      this.closeList(cx);
      return;
    }
    const kind = at !== null ? "files" : "commands";
    if (!this.listOpen || this.listKind !== kind) {
      this.listOpen = true;
      this.listKind = kind;
      this.listMatches = [];
      this.listIndex = 0;
      this.listWindowStart = 0;
      this.listDirs = {};
      this.listFolder = null;
      this.historyKept = null;
      this.listScan += 1;
      if (kind === "commands") {
        const scan = this.listScan;
        this.listCommands = [];
        // A task's own cx, never the one from the change: the answer lands after this call returns.
        cx.spawn(async (cx) => {
          try {
            const list = await leadCommands();
            if (!this.listOpen || this.listKind !== "commands" || scan !== this.listScan) return;
            this.listCommands = Array.isArray(list) ? list : [];
            this.filterList(cx);
          } catch {
            if (scan === this.listScan) this.closeList(cx);
          }
        });
        this.applyKeyContext();
        cx.notify();
        return;
      }
      this.applyKeyContext();
      cx.notify();
    }
    if (kind === "files") this.refreshFiles(cx);
    else this.filterList(cx);
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
    this.applyKeyContext();
    cx.notify();
  },

  // The `@` folder's entries for the current fragment: the folder is read once and kept until the
  // list closes, and read again when it changes.
  refreshFiles(cx) {
    const at = this.fileFragment(this.text?.value?.() ?? "", this.listCursor());
    if (at === null) {
      this.closeList(cx);
      return;
    }
    const { folder } = this.fileParts(at.fragment);
    if (this.listFolder !== folder) {
      this.listFolder = folder;
      if (!Object.prototype.hasOwnProperty.call(this.listDirs, folder)) {
        const scan = this.listScan;
        this.listMatches = [];
        this.listIndex = 0;
        this.listWindowStart = 0;
        cx.spawn(async (cx) => {
          let list = [];
          try {
            list = await listDir(folder);
          } catch {
            list = [];
          }
          if (!this.listOpen || this.listKind !== "files" || scan !== this.listScan || this.listFolder !== folder) return;
          this.listDirs[folder] = Array.isArray(list) ? list : [];
          this.filterFiles(cx);
        });
        this.applyKeyContext();
        cx.notify();
        return;
      }
    }
    this.filterFiles(cx);
  },

  // The folder's entries whose name starts with the prefix, in the order the folder gave them;
  // a name starting with `.` shows only when the prefix does.
  filterFiles(cx) {
    const at = this.fileFragment(this.text?.value?.() ?? "", this.listCursor());
    if (at === null) {
      this.closeList(cx);
      return;
    }
    const { folder, prefix } = this.fileParts(at.fragment);
    const entries = this.listDirs[folder] ?? [];
    const needle = prefix.toLowerCase();
    const dots = prefix.startsWith(".");
    const matches = entries.filter((entry) => {
      const name = String(entry?.name ?? "");
      if (!name || (!dots && name.startsWith("."))) return false;
      return name.toLowerCase().startsWith(needle);
    });
    const same = matches.length === this.listMatches.length
      && matches.every((entry, index) =>
        entry.name === this.listMatches[index].name && entry.dir === this.listMatches[index].dir);
    this.listMatches = matches;
    if (!same) this.listIndex = 0;
    this.listIndex = Math.min(this.listIndex, Math.max(0, matches.length - 1));
    this.clampListWindow();
    this.applyKeyContext();
    cx.notify();
  },

  // Ctrl+R: the box's own history, newest first, filtered by whatever the Owner types.
  openHistory(cx) {
    // The card holds the box while the lead asks; there is no text to search then.
    if (!this.text || this.ask) return;
    if (this.listOpen) this.closeList(cx);
    this.listOpen = true;
    this.listKind = "history";
    this.listCommands = [];
    this.listMatches = [];
    this.listIndex = 0;
    this.listWindowStart = 0;
    this.listScan += 1;
    this.listDirs = {};
    this.listFolder = null;
    this.historyKept = this.text.value();
    this.text.set_value("");
    this.applyKeyContext();
    this.filterHistory(cx);
    this.text.focus();
    cx.notify();
  },

  // Every history entry containing the query, newest first; every one while the query is empty.
  filterHistory(cx) {
    const query = String(this.text?.value?.() ?? "").toLowerCase();
    const matches = (this.history ?? []).filter((entry) => String(entry).toLowerCase().includes(query));
    const same = matches.length === this.listMatches.length
      && matches.every((entry, index) => entry === this.listMatches[index]);
    this.listMatches = matches;
    if (!same) this.listIndex = 0;
    this.listIndex = Math.min(this.listIndex, Math.max(0, matches.length - 1));
    this.clampListWindow();
    this.applyKeyContext();
    cx.notify();
  },

  // Ctrl+R in the box opens the history, or moves down it; anywhere else it does nothing.
  historySearchKey(cx) {
    if (this.listOpen && this.listKind === "history") {
      this.listKey(1, cx);
      return;
    }
    if (!this.listOpen) this.openHistory(cx);
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
    const restore = this.listKind === "history" ? this.historyKept : null;
    this.listOpen = false;
    this.listKind = null;
    this.listCommands = [];
    this.listMatches = [];
    this.listDirs = {};
    this.listFolder = null;
    this.historyKept = null;
    this.listWindowStart = 0;
    this.listScan += 1;
    if (restore != null && this.text) {
      this.text.set_value(restore);
      this.text.set_selected_range({ start: restore.length, end: restore.length });
    }
    this.applyKeyContext();
    if (cx) cx.notify();
  },

  // The textarea's own context decides which keys outrank the editor's.
  applyKeyContext() {
    if (!this.text) return;
    this.text.set_key_context(this.listShown() ? "ComposerList" : this.historyMode ? "ComposerHistory" : null);
  },

  listKey(direction, cx) {
    if (!this.listShown() || this.listMatches.length === 0) return;
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

  // Replace the typed `@fragment` with the file's path and a space, or the folder's path and a
  // slash, which stays open on that folder. Quoted when the whole path has a space.
  insertFile(entry, cx) {
    if (!entry) return;
    const text = this.text?.value?.() ?? "";
    const cursor = this.listCursor();
    const at = this.fileFragment(text, cursor);
    if (at === null) {
      this.closeList(cx);
      return;
    }
    const { folder } = this.fileParts(at.fragment);
    // Paths are written with `/`, however they were typed.
    const path = `${folder.replace(/\\/g, "/")}${String(entry.name ?? "")}/`;
    const quoted = /[ \t]/.test(path);
    const isDir = Boolean(entry.dir);
    const insertion = isDir
      ? (quoted ? `@"${path}` : `@${path}`)
      : (quoted ? `@"${path.slice(0, -1)}" ` : `@${path.slice(0, -1)} `);
    this.text.set_selected_range({ start: at.at, end: cursor });
    this.text.replace(insertion);
    if (isDir) {
      const end = at.at + insertion.length;
      this.text.set_selected_range({ start: end, end });
      this.text.focus();
      this.refreshList(cx);
      cx.notify();
      return;
    }
    this.closeList(cx);
    this.text.focus();
    cx.notify();
  },

  // The highlighted history entry goes into the box in place of the query, the caret after it.
  insertHistory(entry, cx) {
    if (entry == null) return;
    this.historyKept = null;
    this.text.set_value(String(entry));
    this.text.set_selected_range({ start: String(entry).length, end: String(entry).length });
    this.closeList(cx);
    this.text.focus();
    cx.notify();
  },

  insertPicker(cx, trailing) {
    const entry = this.listMatches[this.listIndex];
    if (!entry) return;
    if (this.listKind === "history") {
      this.insertHistory(entry, cx);
      return;
    }
    if (this.listKind === "files") {
      this.insertFile(entry, cx);
      return;
    }
    this.insertList(entry, trailing, cx);
  },

  listInsert(cx) {
    this.insertPicker(cx, true);
  },

  listPick(cx) {
    const entry = this.listMatches[this.listIndex];
    if (!entry) return;
    if (this.listKind === "files" || this.listKind === "history") {
      this.insertPicker(cx, false);
      return;
    }
    this.insertList(entry, Boolean(entry.hint), cx);
    // A command that takes no argument sends itself; one with a hint waits for its argument.
    if (!entry.hint) this.send(cx);
  },

  listClick(index, cx) {
    if (index < 0 || index >= this.listMatches.length) return;
    this.listIndex = index;
    this.insertPicker(cx, true);
  },

  // The header row of the history search: a name for the panel, not a row that can be picked.
  listHeader(t) {
    return h_flex()
      .id("composer-list-header")
      .h(24)
      .px(8)
      .items_center()
      .child(div().text_size(12).text_color(t.dim).child(text("composer.search-history")));
  },

  listMenu(t, cx) {
    if (!this.listShown()) return null;
    const start = Math.max(0, Math.min(this.listWindowStart ?? 0, Math.max(0, this.listMatches.length - 1)));
    const rows = this.listMatches.slice(start, start + LIST_ROWS);
    let panel = v_flex()
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
      .role("list_box");
    if (this.listKind === "history") panel = panel.child(this.listHeader(t));
    return panel.children(rows.map((entry, offset) => {
      const index = start + offset;
      const on = index === this.listIndex;
      if (this.listKind === "history") {
        return h_flex()
          .id(`composer-list-${index}`)
          .h(28)
          .px(8)
          .items_center()
          .rounded(8)
          .when(on, (el) => el.bg(t.selected))
          .role("list_box_option")
          .aria_selected(on)
          .when(on, (el) => el.aria_active_descendant())
          .hover((s) => s.bg(t.hover))
          .on_hover((hovered, cx) => {
            if (hovered && this.listIndex !== index) {
              this.listIndex = index;
              this.clampListWindow();
              cx.notify();
            }
          })
          .on_click((_e, cx) => this.listClick(index, cx))
          .child(div().flex_1().min_w(0).truncate().text_size(13).text_color(t.foreground).child(firstLine(entry)));
      }
      if (this.listKind === "files") {
        return h_flex()
          .id(`composer-list-${index}`)
          .h(28)
          .px(8)
          .items_center()
          .rounded(8)
          .when(on, (el) => el.bg(t.selected))
          .role("list_box_option")
          .aria_selected(on)
          .when(on, (el) => el.aria_active_descendant())
          .hover((s) => s.bg(t.hover))
          .on_hover((hovered, cx) => {
            if (hovered && this.listIndex !== index) {
              this.listIndex = index;
              this.clampListWindow();
              cx.notify();
            }
          })
          .on_click((_e, cx) => this.listClick(index, cx))
          .child(div().text_size(13).text_color(t.foreground).child(`${entry.name}${entry.dir ? "/" : ""}`));
      }
      return h_flex()
        .id(`composer-list-${index}`)
        .h(28)
        .px(8)
        .gap(8)
        .items_center()
        .rounded(8)
        .when(on, (el) => el.bg(t.selected))
        .role("list_box_option")
        .aria_selected(on)
        .when(on, (el) => el.aria_active_descendant())
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
