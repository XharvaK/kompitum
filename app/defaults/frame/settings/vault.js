// Settings › Vault: where the vault is, how far it is from pushed, what needs
// the Owner, and the way into the mailbox. Loaded through `kom vault open`.

import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { kom, run } from "kompitum";

const firstLine = (v) => String(v ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";

function problemText(problem) {
  if (typeof problem === "string") return problem;
  if (problem && typeof problem === "object") {
    return String(problem.message ?? problem.why ?? problem.error ?? JSON.stringify(problem));
  }
  return String(problem ?? "");
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

export function loadVault(view, cx) {
  if (view.vaultLoading) return;
  view.vaultLoading = true;
  cx.spawn(async (cx) => {
    try {
      const result = await kom(["vault", "open", "--json", "--as", "owner"]);
      if (result.code !== 0) throw new Error(errorText(result));
      const data = JSON.parse(result.stdout || "{}");
      if (!data || typeof data !== "object") throw new Error("The vault did not answer an object.");
      view.vault = data;
      view.vaultError = null;
    } catch (e) {
      view.vaultError = firstLine(e?.message ?? e) || "The vault couldn't be read.";
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
  const value = (label) => div().max_w(480).min_w(0).truncate().text_size(12.5).text_color(t.muted).child(label);
  const state = vault ? (unpushed > 0 ? `${unpushed} ${unpushed === 1 ? "note" : "notes"} not sent yet` : "Up to date") : "Reading…";
  return v_flex()
    .w_full()
    .child(view.heading(t, "Vault"))
    .child(div().pb(8).text_size(12.5).line_height("18px").text_color(t.dim)
      .child("The shared Git folder where agents leave each other notes. The Owner is owner."))
    .when(Boolean(view.vaultError), (el) =>
      el.child(div().id("st-vault-error").pb(10).text_size(12.5).text_color(t.blocked).child(view.vaultError)),
    )
    .child(view.row(t, "Folder", null, value(folder || "Unknown")))
    .child(view.row(t, "State", null, value(state)))
    .children(problems.map((problem, i) =>
      div().id(`st-vault-problem-${i}`).pb(6).text_size(12.5).line_height("18px").text_color(t.blocked).child(problem),
    ))
    .child(view.row(t, "Needs you", null, value(`${needs} ${needs === 1 ? "note" : "notes"}`)))
    .child(h_flex().pt(16).child(view.button(t, "st-open-mailbox", "Open the mailbox", () => run("app.mailbox"))));
}
