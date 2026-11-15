// Settings › Vault: where the vault is, how far it is from pushed, what needs
// the Owner, and the way into the mailbox. Loaded through `kom vault open`.

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { kom, run, text } from "kompitum";

const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";

function problemText(problem) {
  if (typeof problem === "string") return problem;
  if (problem && typeof problem === "object") {
    return String(problem.message ?? problem.why ?? problem.error ?? JSON.stringify(problem));
  }
  return String(problem ?? "");
}

// A problem that says the vault's pull failed, not one about a note's shape.
const PULL_FAILED = /couldn't be updated|could not be pulled|may be behind|still working in the vault|nothing was pulled/i;
// A ref lock, a rebase onto several branches, or another puller in the way.
const PULL_BUSY =
  /cannot rebase onto multiple branches|cannot lock|unable to create|unable to update local ref|index\.lock|another git process seems to be running in this repository|remove the file manually to continue|\.lock/i;
const PULL_TIMED_OUT = /took longer than|timed out|timeout/i;

// The plain words under State; git's own text stays in the line's tooltip.
export function pullWords(problem) {
  const words = problemText(problem);
  if (PULL_BUSY.test(words)) return text("settings.pull-busy");
  if (PULL_TIMED_OUT.test(words)) return text("settings.pull-timeout");
  return text("settings.pull-failed");
}

function pullProblem(problems) {
  const problem = problems.find((text) => PULL_FAILED.test(text));
  return problem ? { detail: problem, words: pullWords(problem) } : null;
}

// Piped, kom says what went wrong as JSON: {"error": {"message", "hint"}}; show its words.
function errorText(result) {
  for (const out of [result?.stderr, result?.stdout]) {
    try {
      const message = JSON.parse(String(out ?? "")).error?.message;
      if (message) return firstLine(message);
    } catch {}
  }
  return firstLine(result?.stderr) || firstLine(result?.stdout) || `kom exited ${result?.code ?? "?"}`;
}

function ago(at) {
  if (!at) return "";
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60) return text("settings.just-now");
  const minutes = Math.floor(seconds / 60);
  return text("settings.minutes-ago", { minutes });
}

export function loadVault(view, cx) {
  if (view.vaultLoading) return;
  view.vaultLoading = true;
  cx.notify();
  cx.spawn(async (cx) => {
    try {
      const result = await kom(["vault", "open", "--json", "--as", "owner"]);
      if (result.code !== 0) throw new Error(errorText(result));
      const data = JSON.parse(result.stdout || "{}");
      if (!data || typeof data !== "object") throw new Error(text("settings.vault-no-object"));
      view.vault = data;
      view.vaultError = null;
      view.vaultCheckedAt = Date.now();
    } catch (e) {
      view.vaultError = firstLine(e?.message ?? e) || text("settings.vault-couldnt-read");
    } finally {
      view.vaultLoading = false;
      cx.notify();
    }
  });
}

export function vaultSection(view, t, cx) {
  const vault = view.vault;
  const folder = vault?.vault ? String(vault.vault) : "";
  const unpushed = Number(vault?.unpushed ?? 0);
  const needs = Array.isArray(vault?.notes) ? vault.notes.length : 0;
  const problems = Array.isArray(vault?.problems) ? vault.problems.map(problemText).filter(Boolean) : [];
  const pull = pullProblem(problems);
  // Notes that couldn't be filed: not a failed pull, so they wait under Unfiled.
  const filed = problems.filter((problem) => !PULL_FAILED.test(problem));
  const value = (label) => div().max_w(480).min_w(0).truncate().text_size(12).text_color(t.muted).child(label);
  const placeholder = () => div().w(110).h(9).rounded(5).bg(t.hover);
  // Windows shows its own backslashes, not the forward slashes git prints.
  const shown = /^[A-Za-z]:\//.test(folder) ? folder.replace(/\//g, "\\") : folder;
  const folderControl = folder ? value(shown) : vault || view.vaultError ? value(text("settings.not-set")) : placeholder();
  let state;
  if (!vault && !view.vaultError) {
    state = placeholder();
  } else if (view.vaultLoading) {
    state = div().text_size(12).text_color(t.muted).child(text("settings.checking"));
  } else if (view.vaultError) {
    state = v_flex()
      .items_end()
      .gap(3)
      .child(
        h_flex()
          .gap(4)
          .items_center()
          .child(div().text_size(12).text_color(t.muted).child(text("settings.couldnt-update")))
          .child(view.textButton(t, "st-vault-retry", text("settings.retry"), (_e, cx) => loadVault(view, cx))),
      )
      .child(div().max_w(360).text_size(12).line_height("16px").text_color(t.dim).child(view.vaultError));
  } else if (pull) {
    state = v_flex()
      .items_end()
      .gap(3)
      .child(
        h_flex()
          .gap(4)
          .items_center()
          .child(
            div()
              .text_size(12)
              .text_color(t.blocked)
              .child(view.vaultCheckedAt
                ? text("settings.couldnt-pull-ago", { ago: ago(view.vaultCheckedAt) })
                : text("settings.couldnt-pull")),
          )
          .child(view.textButton(t, "st-vault-retry", text("settings.retry"), (_e, cx) => loadVault(view, cx))),
      )
      .child(
        div()
          .max_w(360)
          .text_size(12)
          .line_height("16px")
          .text_color(t.blocked)
          .tooltip(pull.detail)
          .child(pull.words),
      );
  } else if (unpushed > 0) {
    state = value(`${unpushed} ${unpushed === 1 ? "note" : "notes"} not sent yet`);
  } else {
    state = value(view.vaultCheckedAt
      ? text("settings.up-to-date-ago", { ago: ago(view.vaultCheckedAt) })
      : text("settings.up-to-date"));
  }
  // Folder and Unfiled are the rarely used rows: they sit under Advanced, except a folder that isn't set,
  // which stays in sight.
  const folderRow = view.row(t, text("settings.folder"), null, folderControl);
  const unset = !folder && Boolean(vault || view.vaultError);
  const advancedRows = unset ? [] : [folderRow];
  if (filed.length > 0) {
    advancedRows.push(view.row(t, text("settings.unfiled"), null, h_flex().gap(8).items_center()
      // The problems' own words stay in the tooltip, as git's words do under Sync.
      .child(value(`${filed.length} ${filed.length === 1 ? "note" : "notes"}`).tooltip(filed.join("\n")))
      .child(view.textButton(t, "st-vault-unfiled", text("settings.show"), () => run("app.mailbox")))));
  }
  return v_flex()
    .w_full()
    .child(div().pb(8).text_size(12).line_height("18px").text_color(t.dim)
      .child(text("settings.vault-about")))
    .when(unset, (el) => el.child(folderRow))
    .child(view.row(t, text("settings.sync"), null, state))
    .child(view.row(t, text("settings.needs-you"), null, value(`${needs} ${needs === 1 ? "note" : "notes"}`)))
    .child(view.advanced(t, "vault", advancedRows, ["Folder", "Unfiled"]))
    .child(h_flex().pt(16).child(view.button(t, "st-open-mailbox", text("settings.open-mailbox"), () => run("app.mailbox"))));
}
