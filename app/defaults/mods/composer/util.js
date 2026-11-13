// Helpers that more than one file of the composer uses.

import { div } from "gpui-kit";

// A 1 px divider between the words of the box's bottom row: 12 px tall, in the border colour.
export function hairline(t) {
  return div().flex_none().w(1).h(12).bg(t.border);
}

// A card's Enter and digit keys wait this long after the card shows, so a key pressed in passing can't answer it.
export const CARD_WAIT_MS = 600;

// From this many tokens in the lead's context, the box offers "Compact first" for the Owner's next message.
export const COMPACT_FIRST_TOKENS = 222000;

// A tool summary split on its backticks: the even pieces are words, the odd ones are code. Empty pieces are dropped.
export function codePieces(text) {
  return String(text ?? "")
    .split("`")
    .map((piece, index) => ({ text: piece, code: index % 2 === 1 }))
    .filter((piece) => piece.text.length > 0);
}

export function errorText(error) {
  if (error == null) return "The command failed.";
  if (typeof error === "string") return error;
  if (typeof error.message === "string" && error.message.trim()) return error.message.trim();
  for (const raw of [error.stderr, error.stdout]) {
    if (typeof raw !== "string" || !raw.trim()) continue;
    try {
      const said = JSON.parse(raw.trim());
      const message = said?.error?.message ?? said?.message ?? said?.error;
      if (typeof message === "string" && message.trim()) return message.trim();
    } catch {}
    return raw.trim();
  }
  if (typeof error.code === "number") return `The command failed with exit code ${error.code}.`;
  return String(error);
}
