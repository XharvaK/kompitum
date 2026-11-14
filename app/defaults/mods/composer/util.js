// Helpers that more than one file of the composer uses.

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
