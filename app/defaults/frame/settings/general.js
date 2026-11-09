// Settings › General: the folder the lead starts in, and the welcome again.

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { home, pickFolder, run } from "kompitum";

function chooseFolder(view, cx) {
  cx.spawn(async (cx) => {
    const folder = await pickFolder({ prompt: "Choose" });
    if (!folder) return; // a cancelled pick keeps the old choice
    view.change((person) => {
      person.lead = { ...(person.lead ?? {}), folder };
    }, cx);
  });
}

export function generalPage(view, t, cx) {
  const folder = view.person?.lead?.folder || home();
  return v_flex()
    .w_full()
    .child(
      view.row(
        t,
        "Home folder",
        "Your lead starts here. A new folder starts a new conversation.",
        h_flex()
          .flex_none()
          .gap(8)
          .items_center()
          .child(div().max_w(340).truncate().text_size(13).text_color(t.muted).child(folder))
          .child(view.button(t, "st-general-folder", "Choose\u2026", (_e, cx) => chooseFolder(view, cx))),
        "folder",
      ),
    )
    .child(
      view.row(
        t,
        "Welcome",
        null,
        view.button(t, "st-general-welcome", "Show again", () => run("app.welcome")),
        "welcome",
      ),
    );
}
