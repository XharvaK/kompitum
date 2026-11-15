// A path as a card or a setting reads it: inside the folder it works in, relative to it;
// still long, its root, an ellipsis, then its last two parts whole.

import { window } from "kompitum";

const LIMIT = 60;

// A folder without its trailing separators, except a root ("/" or "C:\").
function tidy(folder) {
  const value = String(folder ?? "");
  return value.length > 1 ? value.replace(/[\\/]+$/, "") : value;
}

// Windows compares without case and reads / and \ alike.
function alike(value) {
  const slashed = String(value ?? "").replace(/\\/g, "/");
  return window()?.platform === "windows" ? slashed.toLowerCase() : slashed;
}

function inside(value, folder) {
  const base = tidy(folder);
  if (!base) return false;
  const path = alike(value);
  const root = alike(base);
  return path === root || path.startsWith(root.endsWith("/") ? root : `${root}/`);
}

// The path under `folder`, or whole when it is outside it (or no folder is known).
export function relativePath(path, folder) {
  const value = String(path ?? "");
  if (!inside(value, folder)) return value;
  const rest = value.slice(tidy(folder).length).replace(/^[\\/]+/, "");
  return rest || value;
}

// The path a row or a card draws: relative when it can be, and over 60 characters
// clipped to its root, "…", and its last two parts whole.
export function shortPath(path, folder) {
  const value = relativePath(path, folder);
  if (value.length <= LIMIT) return value;
  const parts = value.split(/[\\/]+/).filter(Boolean);
  if (parts.length <= 2) return value;
  const separator = value.match(/[\\/]/)?.[0] ?? "/";
  const drive = value.match(/^[A-Za-z]:[\\/]?/)?.[0] ?? value.match(/^[\\/]+/)?.[0] ?? "";
  const first = /^[A-Za-z]:/.test(drive) ? parts[1] : parts[0];
  if (!first) return value;
  return `${drive}${first}${separator}…${separator}${parts.slice(-2).join(separator)}`;
}
