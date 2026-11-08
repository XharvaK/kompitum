#!/usr/bin/env python3
"""Rename the product from Limen to Kompitum, in place, from the repository root.

Run once: python scripts/rename_to_kompitum.py

It reads git's file list, rewrites every text file with the rules below, then
moves every path whose name changes with ``git mv``. CHANGELOG.md and binary
files keep their bytes. It writes a file only when something changed, so line
endings and everything else stay byte for byte as they were.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
ROOT = Path.cwd()

# A whole token: not preceded by a letter, digit, "_" or "-" (a \n, \r or \t
# escape right before it counts as a start), and not followed by a letter,
# digit or "_".
EDGE = r"(?:(?<![A-Za-z0-9_-])|(?<=\\[nrt]))"
R8 = re.compile(EDGE + r"(nolc|lcs?|LC)(?![A-Za-z0-9_])")
R8_REPLACEMENT = {"nolc": "nokom", "lc": "kom", "lcs": "koms", "LC": "KOM"}
R9_START = re.compile(EDGE + r"lc_")
R9_END = re.compile(r"_lc(?![A-Za-z0-9_])")

R1 = re.compile(r"XharvaK/limen-core")
R2 = re.compile(r"XharvaK/Limen(?![A-Za-z0-9-])")
R3 = re.compile(r"(Projects[\\/]+)limen-core")
R4 = re.compile(r"(Projects[\\/]+)Limen(?![A-Za-z0-9-])")

COUNTS = {name: 0 for name in ("R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8", "R9")}


def data_run(text, start, end):
    """Whether the match sits in a run of more than 80 non-space characters."""
    a, b = start, end
    while a > 0 and not text[a - 1].isspace():
        a -= 1
    while b < len(text) and not text[b].isspace():
        b += 1
    return b - a > 80


def substitute(rule, pattern, replacement, text, path=False):
    """Replace each match unless it is data; count only what changed."""
    def replace(match):
        if not path and data_run(text, match.start(), match.end()):
            return match.group(0)
        COUNTS[rule] += 1
        return replacement(match) if callable(replacement) else match.expand(replacement)

    return pattern.sub(replace, text)


def token_substitute(rule, pattern, replacement, text, path=False):
    def replace(match):
        if not path and data_run(text, match.start(), match.end()):
            return match.group(0)
        COUNTS[rule] += 1
        return replacement

    return pattern.sub(replace, text)


def transform(text, path=False):
    text = substitute("R1", R1, "XharvaK/kompitum", text, path)
    text = substitute("R2", R2, "XharvaK/kompitum-mcp", text, path)
    text = substitute("R3", R3, r"\1kompitum", text, path)
    text = substitute("R4", R4, r"\1kompitum-mcp", text, path)

    # R5-R7: plain substrings, everywhere, inside words too.
    for rule, old, new in (("R5", "limen", "kompitum"), ("R6", "Limen", "Kompitum"), ("R7", "LIMEN", "KOMPITUM")):
        count = text.count(old)
        if count:
            COUNTS[rule] += count
            text = text.replace(old, new)

    # R8 and R9 are tokens. Each match is replaced by its own word.
    def word(match):
        return R8_REPLACEMENT[match.group(1)]

    text = substitute("R8", R8, word, text, path)
    text = token_substitute("R9", R9_START, "kom_", text, path)
    text = token_substitute("R9", R9_END, "_kom", text, path)
    return text


def git(*args, **kwargs):
    return subprocess.run(["git"] + list(args), cwd=ROOT, creationflags=NO_WINDOW, check=True, **kwargs)


def tracked_files():
    out = git("ls-files", "-z", capture_output=True).stdout
    return [name for name in out.decode("utf-8", "surrogateescape").split("\0") if name]


def main():
    names = tracked_files()
    unreadable = []
    changed = 0
    for name in names:
        if name.rsplit("/", 1)[-1] == "CHANGELOG.md":
            continue
        path = ROOT / name
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"\0" in data[:8192]:
            continue
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            unreadable.append(name)
            continue
        new_text = transform(text)
        if new_text != text:
            path.write_bytes(new_text.encode("utf-8"))
            changed += 1

    moves = []
    for name in names:
        new_name = transform(name, path=True)
        if new_name != name:
            moves.append((name, new_name))
    if len({new for _, new in moves}) != len(moves):
        raise SystemExit("two paths would land on the same name")
    for old, new in moves:
        destination = ROOT / new
        destination.parent.mkdir(parents=True, exist_ok=True)
        git("mv", "--", old, new)

    # git does not track directories: drop the old ones where every file moved.
    old_dirs = set()
    for old, _ in moves:
        parent = Path(old).parent
        while parent != Path("."):
            old_dirs.add(parent)
            parent = parent.parent
    for folder in sorted(old_dirs, key=lambda p: len(p.parts), reverse=True):
        try:
            (ROOT / folder).rmdir()
        except OSError:
            pass

    for rule, count in COUNTS.items():
        print(f"{rule}: {count}")
    print(f"files changed: {changed}")
    print(f"paths moved: {len(moves)}")
    if unreadable:
        print("not UTF-8, left as they were:")
        for name in unreadable:
            print(f"  {name}")


if __name__ == "__main__":
    main()
