// Settings › Memory: what Claude remembers between sessions. Loaded through
// `kom vault memory --list`; edits write through `kom vault remember --replace`.

import { div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, TextareaState, Textarea, TextView } from "gpui-base";
import { kom } from "kompitum";

const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";

function errorText(result) {
  for (const out of [result?.stderr, result?.stdout]) {
    try {
      const message = JSON.parse(String(out ?? "")).error?.message;
      if (message) return firstLine(message);
    } catch {}
  }
  return firstLine(result?.stderr) || firstLine(result?.stdout) || `kom exited ${result?.code ?? "?"}`;
}

export function loadMemory(view, cx) {
  if (view.memoryLoading) return;
  view.memoryLoading = true;
  cx.notify();
  cx.spawn(async (cx) => {
    try {
      const result = await kom(["vault", "memory", "--list", "--json"]);
      if (result.code !== 0) throw new Error(errorText(result));
      const data = JSON.parse(result.stdout || "{}");
      if (!data || typeof data !== "object" || !Array.isArray(data.memories)) {
        throw new Error("The vault did not answer a list of memories.");
      }
      view.memory = data;
      view.memoryError = null;
    } catch (e) {
      view.memoryError = firstLine(e?.message ?? e) || "Your memory couldn't be read.";
    } finally {
      view.memoryLoading = false;
      cx.notify();
    }
  });
}

// A memory's parts, as the row says them under its title.
export function memoryMeta(memory) {
  const parts = [];
  if (memory.standing) parts.push("Standing");
  if (memory.review_after) parts.push(`Review after ${memory.review_after}`);
  if (memory.updated) parts.push(`Updated ${memory.updated}`);
  return parts.join(" \u00b7 ");
}

// Open memories by project: General (none) first, then projects alphabetically.
export function memoryGroups(memories) {
  const open = memories.filter((memory) => memory.status === "open");
  const groups = [];
  const general = open.filter((memory) => !memory.project);
  if (general.length) groups.push({ name: "General", memories: general });
  const projects = new Map();
  for (const memory of open) {
    if (!memory.project) continue;
    if (!projects.has(memory.project)) projects.set(memory.project, []);
    projects.get(memory.project).push(memory);
  }
  for (const name of [...projects.keys()].sort((a, b) => a.localeCompare(b))) {
    groups.push({ name, memories: projects.get(name) });
  }
  return groups;
}

export function saveDisabled(edit) {
  if (!edit) return true;
  const title = String(edit.title ?? "").trim();
  return !title || title.length > 120 || String(edit.body ?? "").length > 4000;
}

function rowsFor(body) {
  const lines = String(body ?? "").split(/\r?\n/).length;
  return Math.min(16, Math.max(6, lines));
}

// `kom vault remember --replace`: the title, body, and what the memory already had.
export function saveMemory(view, cx) {
  const edit = view.memoryEdit;
  if (saveDisabled(edit)) return;
  cx.spawn(async (cx) => {
    const args = ["vault", "remember", "--replace", edit.path, "--title", edit.title.trim(), "--body", edit.body];
    if (edit.project) args.push("--project", edit.project);
    if (edit.standing) args.push("--standing");
    const result = await kom(args);
    if (result.code === 0) {
      view.memoryEdit = null;
      view.memoryOpenPath = null;
      view.memoryRowError = null;
      view.memoryLoading = false;
      loadMemory(view, cx);
    } else {
      view.memoryRowError = { path: edit.path, text: errorText(result) };
    }
    cx.notify();
  });
}

// `kom vault forget --why <why> <path>`.
export function forgetMemory(view, cx) {
  const path = view.memoryForgetPath;
  const why = String(view.memoryWhyText ?? "").trim();
  if (!path || !why) return;
  cx.spawn(async (cx) => {
    const result = await kom(["vault", "forget", "--why", why, path]);
    if (result.code === 0) {
      view.memoryForgetPath = null;
      view.memoryOpenPath = null;
      view.memoryRowError = null;
      view.memoryForgottenOpen = true;
      view.memoryLoading = false;
      loadMemory(view, cx);
    } else {
      view.memoryRowError = { path, text: errorText(result) };
    }
    cx.notify();
  });
}

export function memoryPage(view, t, cx) {
  const data = view.memory;
  const memories = Array.isArray(data?.memories) ? data.memories : [];
  const problems = Array.isArray(data?.problems) ? data.problems.map(String).filter(Boolean) : [];
  const groups = memoryGroups(memories);
  const forgotten = memories.filter((memory) => memory.status !== "open");

  const heading = (text) => div().pt(10).pb(6).text_size(12).font_semibold().text_color(t.dim).child(text);
  const disabled = (id, label) =>
    div().id(id).px(12).py(5).rounded(7).text_size(13).text_color(t.dim).child(label);

  const openEdit = (memory, cx) => {
    view.memoryOpenPath = memory.path;
    view.memoryForgetPath = null;
    view.memoryRowError = null;
    view.memoryEdit = {
      path: memory.path,
      title: memory.title,
      body: memory.body,
      project: memory.project ?? null,
      standing: Boolean(memory.standing),
    };
    view.memoryTitle.set_value(memory.title);
    view.memoryBody.set_value(memory.body);
    view.memoryBody.set_rows(rowsFor(memory.body), cx);
    cx.notify();
  };

  const openForget = (memory) => {
    view.memoryOpenPath = memory.path;
    view.memoryEdit = null;
    view.memoryRowError = null;
    view.memoryForgetPath = memory.path;
    view.memoryWhyText = "";
    view.memoryWhy.set_value("");
  };

  const row = (memory, index) => {
    const open = view.memoryOpenPath === memory.path;
    const meta = memoryMeta(memory);
    const line = v_flex()
      .id(`st-memory-${index}`)
      .w_full()
      .py(9)
      .gap(3)
      .cursor_pointer()
      .on_click((_e, cx) => {
        if (open) {
          view.memoryOpenPath = null;
          view.memoryEdit = null;
          view.memoryForgetPath = null;
        } else {
          view.memoryOpenPath = memory.path;
          view.memoryEdit = null;
          view.memoryForgetPath = null;
          view.memoryRowError = null;
        }
        cx.notify();
      })
      .child(div().text_size(13).text_color(t.foreground).child(memory.title))
      .when(Boolean(meta), (el) => el.child(div().text_size(12).text_color(t.muted).child(meta)));
    if (!open) return v_flex().w_full().border_b(1).border_color(t.border).child(line);

    const edit = view.memoryEdit?.path === memory.path ? view.memoryEdit : null;
    const forgetHere = view.memoryForgetPath === memory.path;
    const error = view.memoryRowError?.path === memory.path ? view.memoryRowError.text : null;
    const body = v_flex()
      .w_full()
      .pb(10)
      .gap(9)
      .child(TextView.markdown(`memory-body-${index}`, String(memory.body ?? "")).text_size(13).text_color(t.foreground).selectable(true));
    if (!edit && !forgetHere) {
      body.child(
        h_flex()
          .gap(6)
          .child(view.button(t, `st-memory-edit-${index}`, "Edit", (_e, cx) => openEdit(memory, cx)))
          .child(view.button(t, `st-memory-forget-${index}`, "Forget", (_e, cx) => {
            openForget(memory);
            cx.notify();
          })),
      );
    }
    if (edit) {
      body
        .child(
          Input.new(view.memoryTitle)
            .w_full()
            .h(32)
            .px(10)
            .rounded(7)
            .border_1()
            .border_color(t.input)
            .bg(t.background)
            .text_size(13)
            .text_color(t.foreground),
        )
        .child(
          Textarea.new(view.memoryBody)
            .w_full()
            .px(10)
            .py(8)
            .rounded(7)
            .border_1()
            .border_color(t.input)
            .bg(t.background)
            .text_size(13)
            .text_color(t.foreground),
        )
        .child(
          h_flex()
            .gap(6)
            .when(!saveDisabled(edit), (el) =>
              el.child(view.primaryButton(t, "st-memory-save", "Save", (_e, cx) => view.memorySave(cx))),
            )
            .when(saveDisabled(edit), (el) => el.child(disabled("st-memory-save", "Save")))
            .child(view.button(t, "st-memory-cancel", "Cancel", (_e, cx) => {
              view.memoryEdit = null;
              view.memoryRowError = null;
              cx.notify();
            })),
        );
    }
    if (forgetHere) {
      body
        .child(
          Input.new(view.memoryWhy)
            .w_full()
            .h(32)
            .px(10)
            .rounded(7)
            .border_1()
            .border_color(t.input)
            .bg(t.background)
            .text_size(13)
            .text_color(t.foreground),
        )
        .child(
          h_flex()
            .gap(6)
            .when(Boolean(String(view.memoryWhyText ?? "").trim()), (el) =>
              el.child(view.button(t, "st-memory-forget-now", "Forget", (_e, cx) => view.memoryForget(cx))),
            )
            .when(!String(view.memoryWhyText ?? "").trim(), (el) =>
              el.child(disabled("st-memory-forget-now", "Forget")),
            )
            .child(view.button(t, "st-memory-cancel", "Cancel", (_e, cx) => {
              view.memoryForgetPath = null;
              view.memoryRowError = null;
              cx.notify();
            })),
        );
    }
    if (error) body.child(div().text_size(12).text_color(t.blocked).child(error));
    return v_flex().w_full().border_b(1).border_color(t.border).child(line).child(body);
  };

  const shown = [];
  groups.forEach((group) => {
    shown.push(heading(group.name));
    group.memories.forEach((memory) => shown.push(row(memory, memories.indexOf(memory))));
  });
  if (forgotten.length) {
    const open = Boolean(view.memoryForgottenOpen);
    shown.push(
      h_flex()
        .id("st-memory-forgotten")
        .w_full()
        .pt(14)
        .pb(6)
        .gap(6)
        .items_center()
        .cursor_pointer()
        .on_click((_e, cx) => {
          view.memoryForgottenOpen = !open;
          cx.notify();
        })
        .child(div().text_size(12).font_semibold().text_color(t.dim).child(`Forgotten \u00b7 ${forgotten.length}`))
        .child(div().text_size(12).text_color(t.dim).child(open ? "\u2304" : "\u203a")),
    );
    if (open) forgotten.forEach((memory) => shown.push(row(memory, memories.indexOf(memory))));
  }

  let state = null;
  if (!data && !view.memoryError) {
    state = div().py(12).text_size(12).text_color(t.muted).child("Loading\u2026");
  } else if (view.memoryLoading) {
    state = div().py(12).text_size(12).text_color(t.muted).child("Checking\u2026");
  } else if (view.memoryError) {
    state = v_flex()
      .py(12)
      .gap(4)
      .child(
        h_flex()
          .gap(4)
          .items_center()
          .child(div().text_size(12).text_color(t.muted).child("Couldn't read your memory"))
          .child(view.textButton(t, "st-memory-retry", "Retry", (_e, cx) => loadMemory(view, cx))),
      )
      .child(div().max_w(420).text_size(12).line_height("16px").text_color(t.blocked).child(view.memoryError));
  }

  return v_flex()
    .w_full()
    .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Memory"))
    .child(
      div()
        .pt(4)
        .pb(8)
        .text_size(12)
        .line_height("18px")
        .text_color(t.muted)
        .child("What Claude remembers between sessions. Standing ones are read in full at every start."),
    )
    .when(Boolean(state), (el) => el.child(state))
    .when(!state && memories.length === 0, (el) => el.child(div().py(12).text_size(12).text_color(t.muted).child("Claude hasn't remembered anything yet.")))
    .when(Boolean(problems.length), (el) =>
      el.child(
        div()
          .py(8)
          .text_size(12)
          .text_color(t.blocked)
          .tooltip(problems.join("\n"))
          .child(problems.length === 1 ? problems[0] : `${problems.length} memory notes couldn't be read.`),
      ),
    )
    .children(shown);
}
