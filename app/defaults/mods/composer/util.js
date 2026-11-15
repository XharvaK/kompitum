// Helpers that more than one file of the composer uses.

import { text } from "kompitum";

// A card's Enter and digit keys wait this long after the card shows, so a key pressed in passing can't answer it.
export const CARD_WAIT_MS = 600;

// From this many tokens in the lead's context, the box offers "Compact first" for the Owner's next message.
export const COMPACT_FIRST_TOKENS = 222000;
// How long a pulse may run before it stops and the word or glyph carries the state alone (UD's pulseLimit).
export const PULSE_MS = 3000;

// A tool summary split on its backticks: the even pieces are words, the odd ones are code. Empty pieces are dropped.
export function codePieces(text) {
  return String(text ?? "")
    .split("`")
    .map((piece, index) => ({ text: piece, code: index % 2 === 1 }))
    .filter((piece) => piece.text.length > 0);
}

// The Owner's words as Markdown that draws exactly as typed. The bubble needs a Markdown view, the only
// text view that reports a selection, so Markdown's marks are escaped (`$` too: inline math is on), each
// line break becomes a hard break instead of reflowing, and leading spaces and blank lines are kept as
// no-break spaces. Marks that only count at a line's start (headings, quotes, lists, rules) are escaped
// there alone, so ordinary words like `lead.rs` stay as they are.
export function literalMarkdown(text) {
  const value = String(text ?? "").replace(/\r\n?/g, "\n").replace(/\n+$/, "");
  if (!value) return "";
  return value
    .split("\n")
    .map((line) => {
      const indent = line.match(/^[ \t]*/)[0];
      const rest = line
        .slice(indent.length)
        .replace(/[\\`*_[\]<~|$&]/g, "\\$&")
        .replace(/^([#>+=-])/, "\\$1")
        .replace(/^(\d+)([.)])/, "$1\\$2");
      return indent.replace(/\t/g, "    ").replace(/ /g, " ") + rest || " ";
    })
    .join("\\\n");
}

// UTF-16 index (what JavaScript strings count) to UTF-8 bytes, without TextEncoder: QuickJS may lack it.
export function byteIndex(text, i) {
  const value = String(text ?? "");
  const index = Math.max(0, Math.min(Math.trunc(Number(i) || 0), value.length));
  let bytes = 0;
  for (let at = 0; at < index; ) {
    const code = value.charCodeAt(at);
    if (code < 0x80) {
      bytes += 1;
      at += 1;
    } else if (code < 0x800) {
      bytes += 2;
      at += 1;
    } else if (code >= 0xd800 && code <= 0xdbff && at + 1 < value.length) {
      const next = value.charCodeAt(at + 1);
      bytes += next >= 0xdc00 && next <= 0xdfff ? 4 : 3;
      at += next >= 0xdc00 && next <= 0xdfff ? 2 : 1;
    } else {
      bytes += 3;
      at += 1;
    }
  }
  return bytes;
}

// UTF-8 byte offset back to a UTF-16 index; an offset inside a character lands after it.
export function charIndex(text, b) {
  const value = String(text ?? "");
  const target = Math.max(0, Math.min(Math.trunc(Number(b) || 0), byteIndex(value, value.length)));
  let bytes = 0;
  for (let at = 0; at < value.length; ) {
    if (bytes >= target) return at;
    const code = value.charCodeAt(at);
    if (code < 0x80) {
      bytes += 1;
      at += 1;
    } else if (code < 0x800) {
      bytes += 2;
      at += 1;
    } else if (code >= 0xd800 && code <= 0xdbff && at + 1 < value.length) {
      const next = value.charCodeAt(at + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        at += 2;
      } else {
        bytes += 3;
        at += 1;
      }
    } else {
      bytes += 3;
      at += 1;
    }
  }
  return value.length;
}

export function errorText(error) {
  if (error == null) return text("composer.command-failed");
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
  if (typeof error.code === "number") return text("composer.command-exit", { code: error.code });
  return String(error);
}
