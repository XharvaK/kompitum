// One window for this launch's unpresented spotlights. The app supplies the
// release range; only entries shown in full here count as presented.
import { View, div } from "gpui-kit";
import { v_flex, h_flex, Scrollbar } from "gpui-base";
import { theme, props, closeOverlay, presentUpdateNotes, updates, sheet, text } from "kompitum";
import { versionHeader, releaseItems } from "./release-notes.js";

export default class WhatsNew extends View {
  init(_props, cx) {
    this.notes = props()?.notes ?? [];
    presentUpdateNotes(this.notes.filter(n => n.spotlight).map(n => n.version));
    this.handle = cx.focus_handle();
    this.handle.focus();
    cx.timer.after(600, () => this.handle.focus());
  }

  done() {
    closeOverlay(props().overlay);
  }

  releaseNotes(t) {
    const body = v_flex().gap(18);
    for (const note of this.notes) {
      const current = updates().notes?.find(n => n.version === note.version) ?? note;
      const entry = v_flex().gap(16)
        .child(versionHeader(t, note))
        .child(releaseItems(t, current, { width: this.notesWidth, onSizeChange: (size, cx) => { this.notesWidth = size.width; cx.notify(); } }));
      body.child(entry);
    }
    return body;
  }

  render() {
    const t = theme();
    const body = div().relative().flex_1().min_h(0).track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key === "enter") {
          this.done();
          cx.stop_propagation();
        }
      })
      .child(div().id("wn-body").size_full().overflow_y_scroll()
        .child(div().px(22).pt(20).pb(20).child(this.releaseNotes(t))))
      .child(Scrollbar.vertical("wn-body").mode("always").absolute().inset_0());
    const footer = h_flex().w_full().px(16).gap(8).items_center()
      .child(div().pl(6).flex_1().text_size(13).text_color(t.dim).child(text("whatsnew.footer-note")))
      .child(h_flex().id("wn-done").relative().h(30).px(12).items_center().rounded(8).cursor_pointer()
        .text_size(13).font_medium().bg(t.accent_fill).text_color(t.accent_foreground)
        .on_click(() => this.done()).child(div().absolute().inset_0().rounded(8).hover((s) => s.bg(t.hover).text_color(t.foreground)).active((s) => s.bg(t.selected))).child(text("whatsnew.got-it")));
    return sheet({ title: text("whatsnew.title"), width: 880, onClose: () => this.done(), footer }, body);
  }
}
