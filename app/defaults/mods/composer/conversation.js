// The conversation above the box: its items, tool rows, folding and scroll.

import { div, svg } from "gpui-kit";
import { v_flex, h_flex, TextView, Scrollbar } from "gpui-base";
import { lead as leadOf, conversation as conversationFrom, settings } from "kompitum";
import { codePieces } from "./util.js";

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
    let start = this.items.length > 0 ? Math.max(0, this.items.length - 1) : 0;
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

  ownerItem(t, item) {
    return h_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .justify_end()
      .child(
        div()
          .px(12)
          .py(8)
          .rounded(14)
          .max_w("80%")
          .bg(t.selected)
          .text_size(13.5)
          .text_color(t.foreground)
          .selectable(true)
          .child(item.text ?? ""),
      );
  },

  assistantItem(t, item) {
    return v_flex()
      .id(`composer-item-${item.id}`)
      .w_full()
      .gap(6)
      .child(
        TextView.markdown(`composer-md-${item.id}`, item.text ?? "")
          .text_size(13.5)
          .text_color(t.foreground)
          .selectable(true),
      )
      .when(!item.done, (el) =>
        el.child(
          div()
            .id(`composer-streaming-${item.id}`)
            .mt(2)
            .size(6)
            .rounded(3)
            .bg(t.accent)
            .loop("opacity", { from: 0.35, to: 1, period: 900 }),
        ),
      );
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
        : div().min_w(0).text_size(12.5).text_color(t.muted).child(piece.text)));
  },

  toolItem(t, item, cx) {
    const open = Boolean(this.openTools[item.id]);
    const result = typeof item.result === "string" && item.result ? item.result : "";
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
          .child(this.toolSummary(t, item.summary ?? item.name ?? "A tool")),
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
            .font_family(settings()?.font_family ?? "Google Sans Code")
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
        .child(div().text_size(12.5).text_color(t.muted).child(`Used ${group.items.length} tools`));
    }
    return v_flex()
      .id("composer-tools-used")
      .w_full()
      .gap(14)
      .children(group.items.map((item) => this.toolItem(t, item, cx)));
  },

  drawItem(t, item, cx) {
    if (item?.kind === "owner") return this.ownerItem(t, item);
    if (item?.kind === "assistant") return this.assistantItem(t, item);
    if (item?.kind === "tool") return this.toolItem(t, item, cx);
    if (item?.kind === "notice") return this.noticeItem(t, item);
    return null;
  },

  conversationPanel(t, cx) {
    // No items: an empty panel. A started lead's new conversation is drawn by startView instead.
    if (this.items.length === 0) {
      return div().id("composer-panel").flex_1().min_h(0).relative();
    }
    // The padding sits on an inner column: on the scroll area itself it would count as content to scroll past.
    // The conversation and the box share one column: at most 760 px, centred, with 20 px gutters when narrower.
    const column = v_flex().w_full().max_w(760).px(20).pt(16).pb(16).gap(14);
    for (const group of this.conversationGroups()) {
      if (group.type === "tools") column.child(this.toolsGroup(t, group, cx));
      else column.child(this.drawItem(t, group.item, cx));
    }
    const list = v_flex()
      .id("composer-conversation")
      .flex_1()
      .min_h(0)
      .child(h_flex().w_full().justify_center().child(column))
      .overflow_y_scroll()
      .track_scroll(this.scroll)
      .on_scroll_wheel((event, cx) => {
        if (event.delta.y < 0 && this.stickToBottom) {
          this.stickToBottom = false;
          cx.notify();
        }
      });
    return div()
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
            .child("Jump to latest"),
        ),
      );
  },
};
