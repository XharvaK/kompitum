// The conversation above the box: its items, tool rows, folding and scroll.

import { div, svg } from "gpui-kit";
import { v_flex, h_flex, TextView, Scrollbar } from "gpui-base";
import { lead as leadOf, conversation as conversationFrom, settings, run, text } from "kompitum";
import { codePieces, literalMarkdown } from "./util.js";

export default {
  syncConversation() {
    const meta = leadOf()?.conversation;
    if (!meta) {
      if (this.seenVersion !== null) {
        this.items = [];
        this.seenVersion = null;
      }
      return;
    }
    if (meta.version === this.seenVersion) return;
    // An earlier row can change after a later one: a tool ends out of order, and an
    // assistant keeps streaming. Read again from the first row that is still open.
    let start = Math.max(0, this.items.length - 1);
    const open = this.items.findIndex((item) =>
      (item?.kind === "tool" && item.state === "running") || (item?.kind === "assistant" && !item.done));
    if (open >= 0 && open < start) start = open;
    if ((meta.length ?? 0) < this.items.length) start = 0;
    let fresh = [];
    try {
      fresh = conversationFrom(start) ?? [];
    } catch {
      fresh = [];
    }
    if (!Array.isArray(fresh)) fresh = [];
    this.items = this.items.slice(0, start).concat(fresh);
    this.seenVersion = meta.version;
    // Stuck to the bottom, a new item (or a new text batch) keeps the newest one in view.
    if (this.stickToBottom) this.scroll.scroll_to_end();
  },

  terminalShown() {
    return leadOf()?.terminal === "shown";
  },

  revealNote(t) {
    const lead = leadOf();
    if (lead?.terminal !== "shown") return null;
    if (lead?.reveal !== "auto" && lead?.reveal !== "login" && lead?.reveal !== "startup") return null;
    const why = typeof lead.reveal_why === "string" && lead.reveal_why.trim() ? lead.reveal_why : null;
    if (!why) return null;
    return div()
      .id("composer-reveal-note")
      .px(20)
      .pb(8)
      .text_size(12)
      .text_color(t.dim)
      .child(why);
  },

  conversationGroups() {
    const groups = [];
    let i = 0;
    while (i < this.items.length) {
      const item = this.items[i];
      // Only finished tools fold; a failed one keeps its own line, so its x stays in sight.
      if (item?.kind === "tool" && item.state === "done") {
        let j = i;
        while (j < this.items.length && this.items[j]?.kind === "tool" && this.items[j].state === "done") j += 1;
        if (j - i >= 2) {
          groups.push({ type: "tools", items: this.items.slice(i, j), start: i });
          i = j;
          continue;
        }
      }
      groups.push({ type: "item", item, start: i });
      i += 1;
    }
    return groups;
  },

  ownerItem(t, item, cx) {
    const quoted = this.quote?.itemId === item.id;
    return v_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .items_end()
      .gap(4)
      .child(
        div()
          .px(12)
          .py(8)
          .rounded(14)
          .max_w("80%")
          .bg(t.selected)
          .text_size(13)
          .text_color(t.foreground)
          .child(
            TextView.markdown(`composer-owner-md-${item.id}`, literalMarkdown(item.text))
              .text_size(13)
              .text_color(t.foreground)
              .on_selection_change((text, cx) => this.selectQuote(item.id, text, cx)),
          ),
      )
      .when(Boolean(item.steering), (el) =>
        el.child(
          div()
            .id(`composer-steering-${item.id}`)
            .text_size(12)
            .text_color(t.dim)
            .child(text("composer.steering")),
        ))
      .when(quoted, (el) => el.child(this.replyButton(t, item.id, cx)));
  },

  assistantItem(t, item, cx) {
    return v_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .gap(6)
      .child(
        TextView.markdown(`composer-md-${item.id}`, item.text ?? "")
          .text_size(13)
          .text_color(t.foreground)
          .selectable(true)
          .on_selection_change((text, cx) => this.selectQuote(item.id, text, cx)),
      )
      .when(!item.done, (el) =>
        el.child(
          div()
            .id(`composer-streaming-${item.id}`)
            .mt(2)
            .size(6)
            .rounded(3)
            // Still: the words arriving show the reply is coming.
            .bg(t.accent),
        ),
      )
      .when(this.quote?.itemId === item.id, (el) =>
        el.child(h_flex().w_full().justify_end().child(this.replyButton(t, item.id, cx))));
  },

  replyButton(t, itemId, cx) {
    return h_flex()
      .id(`composer-quote-reply-${itemId}`)
      .h(24)
      .px(8)
      .items_center()
      .rounded(6)
      .border_1()
      .border_color(t.border)
      .bg(t.raised)
      .text_size(12)
      .text_color(t.foreground)
      .cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => this.replyToQuote(cx))
      .child(text("composer.reply"));
  },

  selectQuote(itemId, text, cx) {
    // The Owner's leading spaces are drawn as no-break spaces (literalMarkdown); a quote takes them back.
    if (text) this.quote = { itemId, text: String(text).replace(/ /g, " ") };
    else if (this.quote?.itemId === itemId) this.quote = null;
    cx.notify();
  },

  // The quote goes into the box as a Markdown block, after whatever is already there, and the
  // keyboard follows it.
  replyToQuote(cx) {
    const quote = this.quote;
    if (!quote) return;
    const block = `${String(quote.text).replace(/\r?\n+$/, "").split(/\r?\n/).map((line) => `> ${line}`).join("\n")}\n\n`;
    const existing = this.text.value();
    const next = existing ? `${existing}\n\n${block}` : block;
    this.text.set_value(next);
    this.text.set_selected_range({ start: next.length, end: next.length });
    this.text.focus();
    this.quote = null;
    this.syncDraft();
    this.setHistory(true, cx);
    cx.notify();
  },

  toolIcon(t, state) {
    if (state === "running") return svg("mods/workspace-chrome/spinner.svg").size(14).text_color(t.muted);
    if (state === "failed") return svg("mods/workspace-chrome/x.svg").size(14).text_color(t.blocked);
    return svg("mods/workspace-chrome/check.svg").size(14).text_color(t.muted);
  },

  // A tool's summary: its words as they are, and its code (a command) in the code font, cut at the row's edge.
  toolSummary(t, text) {
    return h_flex()
      .min_w(0)
      .gap(4)
      .children(codePieces(text).map((piece) => piece.code
        ? div()
          .min_w(0)
          .truncate()
          .text_size(12)
          .font_family(settings()?.font_family ?? "Google Sans Code")
          .text_color(t.muted)
          .child(piece.text)
        : div().min_w(0).text_size(12).text_color(t.muted).child(piece.text)));
  },

  toolItem(t, item, cx) {
    const open = Boolean(this.openTools[item.id]);
    const result = typeof item.result === "string" && item.result ? item.result : "";
    const code = typeof item.code === "string" && item.code ? item.code : "";
    const mono = settings()?.font_family ?? "Google Sans Code";
    return v_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .gap(6)
      .child(
        h_flex()
          .id(`composer-tool-${item.id}`)
          .gap(6)
          .items_center()
          .cursor_pointer()
          .on_click((_e, cx) => {
            this.openTools[item.id] = !open;
            cx.notify();
          })
          .child(this.toolIcon(t, item.state))
          .child(this.toolSummary(t, item.summary ?? item.name ?? text("composer.a-tool"))),
      )
      .when(open && code, (el) =>
        el.child(
          div()
            .id(`composer-tool-code-${item.id}`)
            .px(10)
            .py(8)
            .rounded(8)
            .bg(t.surface)
            .text_size(12)
            .font_family(mono)
            .text_color(t.dim)
            .child(code),
        ),
      )
      .when(open && result, (el) =>
        el.child(
          div()
            .id(`composer-tool-result-${item.id}`)
            .px(10)
            .py(8)
            .rounded(8)
            .bg(t.surface)
            .text_size(12)
            .font_family(mono)
            .text_color(t.dim)
            .child(result.split(/\r?\n/).slice(0, 12).join("\n")),
        ),
      );
  },

  noticeItem(t, item) {
    return div()
      .id(`composer-item-${item.id}`)
      .w_full()
      .text_center()
      .text_size(12)
      .text_color(t.dim)
      .child(item.text ?? "");
  },

  toolsGroup(t, group, cx) {
    const open = Boolean(this.openToolGroups[group.start]);
    if (!open) {
      return h_flex()
        .id("composer-tools-used")
        .gap(6)
        .items_center()
        .cursor_pointer()
        .on_click((_e, cx) => {
          this.openToolGroups[group.start] = true;
          cx.notify();
        })
        .child(svg("mods/workspace-chrome/check.svg").size(14).text_color(t.muted))
        .child(div().text_size(12).text_color(t.muted).child(text("composer.tools-used", { count: group.items.length })));
    }
    return v_flex()
      .id("composer-tools-used")
      .w_full()
      .gap(14)
      .children(group.items.map((item) => this.toolItem(t, item, cx)));
  },

  drawItem(t, item, cx) {
    if (item?.kind === "owner") return this.ownerItem(t, item, cx);
    if (item?.kind === "assistant") return this.assistantItem(t, item, cx);
    if (item?.kind === "tool") return this.toolItem(t, item, cx);
    if (item?.kind === "notice") return this.noticeItem(t, item);
    return null;
  },

  // Each finished turn's edits, keyed by the index of the turn's last item.
  turnSummaries() {
    const summaries = new Map();
    const owners = this.items
      .map((item, index) => (item?.kind === "owner" ? index : -1))
      .filter((index) => index >= 0);
    for (let n = 0; n < owners.length; n += 1) {
      const start = owners[n];
      const next = owners[n + 1] ?? this.items.length;
      const end = next - 1;
      // The newest turn is still running while the lead works; it ends at its Stop.
      const inProgress = next === this.items.length
        && (leadOf()?.state === "working"
          || leadOf()?.state === "asking"
          || this.items.slice(start + 1).some((item) =>
            (item?.kind === "assistant" && !item.done) || (item?.kind === "tool" && item.state === "running")));
      if (inProgress) continue;
      const paths = new Set();
      let added = 0;
      let removed = 0;
      for (let index = start + 1; index <= end; index += 1) {
        const item = this.items[index];
        if (item?.kind !== "tool" || !item.edit) continue;
        paths.add(item.edit.path);
        added += Number(item.edit.added) || 0;
        removed += Number(item.edit.removed) || 0;
      }
      if (paths.size > 0) summaries.set(end, { start, files: paths.size, added, removed });
    }
    return summaries;
  },

  turnSummary(t, summary, cx) {
    return h_flex()
      .id(`composer-turn-summary-${summary.start}`)
      .gap(6)
      .items_center()
      .child(
        div()
          .text_size(12)
          .text_color(t.muted)
          .child(summary.files === 1
            ? text("composer.files-changed-one", { count: summary.files })
            : text("composer.files-changed", { count: summary.files })),
      )
      .child(div().text_size(12).text_color(t.muted).child(text("composer.diff-count", { added: summary.added, removed: summary.removed })))
      .child(
        div()
          .id(`composer-turn-review-${summary.start}`)
          .text_size(12)
          .text_color(t.accent)
          .cursor_pointer()
          .on_click((_e, _cx) => {
            run("layout.select", { stack: "panel-body", child: "changes" });
            run("layout.show", { id: "panel" });
          })
          .child(text("composer.review")),
      );
  },

  conversationPanel(t, cx) {
    // No items: an empty panel. A started lead's new conversation is drawn by startView instead.
    if (this.items.length === 0) {
      return div().id("composer-panel").flex_1().min_h(0).relative();
    }
    // The padding sits on an inner column: on the scroll area itself it would count as content to scroll past.
    // The conversation and the box share one column: at most 760 px, centred, with 20 px gutters when narrower.
    const column = v_flex().w_full().max_w(760).px(20).pt(16).pb(16).gap(14);
    const summaries = this.turnSummaries();
    for (const group of this.conversationGroups()) {
      const end = group.type === "tools" ? group.start + group.items.length - 1 : group.start;
      if (group.type === "tools") column.child(this.toolsGroup(t, group, cx));
      else column.child(this.drawItem(t, group.item, cx));
      const summary = summaries.get(end);
      if (summary) column.child(this.turnSummary(t, summary, cx));
    }
    const list = v_flex()
      .id("composer-conversation")
      .flex_1()
      .min_h(0)
      .child(h_flex().w_full().justify_center().child(column))
      .overflow_y_scroll()
      .track_scroll(this.scroll)
      // A card (or a chip, or a line of text) changes the box's height, so this list's viewport changes a
      // moment later. Stuck to the bottom, follow the last item through that change too.
      .on_size_change((_size, cx) => {
        if (!this.stickToBottom) return;
        this.scroll.scroll_to_end();
        cx.notify();
      })
      .on_scroll_wheel((event, cx) => {
        if (event.delta.y < 0 && this.stickToBottom) {
          this.stickToBottom = false;
          cx.notify();
        }
      });
    return v_flex()
      .id("composer-panel")
      .flex_1()
      .min_h(0)
      .relative()
      .child(list)
      .child(Scrollbar.vertical("composer-conversation").absolute().inset_0())
      .when(!this.stickToBottom, (el) =>
        el.child(
          h_flex()
            .id("composer-jump-latest")
            .absolute()
            .bottom(8)
            .h(26)
            .px(12)
            .items_center()
            .rounded(13)
            .border_1()
            .border_color(t.border)
            .bg(t.raised)
            .text_size(12)
            .text_color(t.foreground)
            .cursor_pointer()
            .on_click((_e, cx) => {
              this.stickToBottom = true;
              this.scroll.scroll_to_end();
              cx.notify();
            })
            .child(text("composer.jump-latest")),
        ),
      );
  },
};
