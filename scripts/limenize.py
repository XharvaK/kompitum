#!/usr/bin/env python3
"""Build Limen's renamed tree from an upstream herdr-core or herdr-app commit.

    python scripts/limenize.py check [--app] [--ref REF] [--report PATH]
    python scripts/limenize.py sync  [--app] [--ref REF] [--branch NAME] [--trailer TEXT]... [--report PATH]

Both commands read git objects only: the working tree, the real index, HEAD and
every ref except ``refs/heads/<branch>`` are never touched.

With ``--app`` the app's files go under ``app/``, ``--ref`` defaults to
``upstream-app/master`` and ``--branch`` to ``upstream-app-renamed``.

``counts.files`` counts the entries in REF's tree; ``dropped`` is the subset
left out. Problems that are not tied to a line (a path, a missing licence
line, a failed rustfmt) report line 0. In JSON, ``branch`` is
``{"name": ..., "sha": ...}`` (null for check) and each ``tokens`` entry is
``{"new": ..., "count": ...}``.
"""

import argparse
import fnmatch
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)

DROP = (
    ".github/",
    "README.md",
    "AGENTS.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "crates/herdr-core/tests/macos_distribution.rs",
)

APP_DROP = (
    ".github/",
    ".gitmodules",
    "README.md",
    "AGENTS.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
)

VERBATIM = (
    "CHANGELOG.md",
    "LICENSE",
    "distribution/notices/*.txt",
    "distribution/windows/*",
)

LICENSE_FILES = ("scripts/notices", "crates/herdr-core/THIRD-PARTY-NOTICES")
OLD_LICENSE = "hc (herdr-core) is Apache-2.0."
NEW_LICENSE = (
    "lc (limen-core) is Apache-2.0, a modified fork of herdr-core "
    "(https://github.com/herdrdev/herdr-core)."
)
APP_LICENSE_LINE = (
    "limen-app is Apache-2.0, a modified fork of herdr-app "
    "(https://github.com/herdrdev/herdr-app)."
)

EDGE = r"(?:(?<![A-Za-z0-9])|(?<=\\[nrt]))"

# herdr.dev and every subdomain of it. A plain herdr -> limen rule would make
# limen.dev, a domain we do not own.
HOST_RULE = (
    r"(?<![A-Za-z0-9.-])((?:[A-Za-z0-9-]+\.)*)herdr\.dev"
    r"(?![A-Za-z0-9-]|\.[A-Za-z0-9-])",
    r"\1limen.invalid",
)

TEXT_RULES = (
    HOST_RULE,
    (r"herdr", "limen"),
    (r"Herdr", "Limen"),
    (r"HERDR", "LIMEN"),
    (r"nohc", "nolc"),
    (EDGE + r"hcs(?![A-Za-z0-9])", "lcs"),
    (EDGE + r"hc(?![A-Za-z0-9])", "lc"),
    (EDGE + r"HC_", "LIMEN_"),
    (EDGE + r"HC(?![A-Za-z0-9_])", "LC"),
)

# Vendored code has its own words starting with hc, so only our own name
# changes there. It is not ours to format either.
VENDOR_TEXT_RULES = (
    HOST_RULE,
    (r"herdr", "limen"),
    (r"Herdr", "Limen"),
    (r"HERDR", "LIMEN"),
)

PATH_RULES = TEXT_RULES[1:4] + TEXT_RULES[5:7]

LEFTOVER = (
    r"(?i)herdr",
    r"(?<![A-Za-z0-9])hc",
    r"[A-Za-z0-9]hc(?![A-Za-z0-9])",
    r"(?<![A-Za-z0-9])HC(?![A-Za-z0-9])",
)

VENDOR_LEFTOVER = (r"(?i)herdr",)

COLLISION = (
    r"(?<![A-Za-z0-9])lcs?(?![A-Za-z0-9])",
    r"(?i)limen",
)

WRONG = (r"limendev", r"limen\.dev")

RESERVED = (
    "LIMEN_VAULT",
    "LIMEN_AGENT",
    "LIMEN_POLL_SECONDS",
    "LIMEN_SEEN_FILE",
    "LIMEN_LISTEN",
    "LIMEN_BELL_",
)

# Words outside vendor/ that the scans would flag but that are not our names.
# The second item is the exact token around the match, not the line.
ALLOWED = (
    # A shell's own "-lc" flag, not the lc command.
    ("src/core_bin.rs", "-lc"),
    # The Haskell compiler ghc, in the program list that picks an icon.
    ("defaults/programs.json", "ghc"),
    # yaahc is a GitHub user in a dependency's repository URL.
    ("THIRD-PARTY-NOTICES", "https://github.com/yaahc/displaydoc"),
)

# github.com/herdrdev/ and the path after it is upstream's own address, used
# in attributions and build-from-source links. It stays as written and is not
# a leftover.
UPSTREAM_ADDRESS = re.compile(r"github\.com/herdrdev/[^\s\"'<>()\[\]{}]*")
ADDRESS_MARK = "\x02address-%d\x02"

_TEXT_RULES = tuple((re.compile(p), r) for p, r in TEXT_RULES)
_VENDOR_RULES = tuple((re.compile(p), r) for p, r in VENDOR_TEXT_RULES)
_PATH_RULES = tuple((re.compile(p), r) for p, r in PATH_RULES)
_LEFTOVER = tuple(re.compile(p) for p in LEFTOVER)
_VENDOR_LEFTOVER = tuple(re.compile(p) for p in VENDOR_LEFTOVER)
_COLLISION = tuple(re.compile(p) for p in COLLISION)
_WRONG = tuple(re.compile(p) for p in WRONG)

TOKEN_CHARS = frozenset(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.:/$@-"
)
DEPENDENCIES = re.compile(r"(?m)^dependencies = (\[[^\]]*\])$")


class GitError(RuntimeError):
    pass


def run(cmd, input=None, env=None):
    return subprocess.run(
        cmd,
        input=input,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        creationflags=NO_WINDOW,
        env=env,
    )


def git(args, input=None, env=None):
    result = run(["git"] + args, input=input, env=env)
    if result.returncode:
        message = result.stderr.decode("utf-8", "replace").strip()
        raise GitError("git %s failed: %s" % (" ".join(args), message))
    return result.stdout


def git_optional(args):
    result = run(["git"] + args)
    if result.returncode:
        return None
    return result.stdout.decode("utf-8", "replace").strip()


def decode_path(raw):
    return raw.decode("utf-8", "surrogateescape")


def encode_path(path):
    return path.encode("utf-8", "surrogateescape")


def token_bounds(text, start, end):
    while start and text[start - 1] in TOKEN_CHARS:
        start -= 1
    while end < len(text) and text[end] in TOKEN_CHARS:
        end += 1
    return start, end


def protect_addresses(text):
    """Set upstream's own GitHub addresses aside while the rules run."""
    addresses = []

    def take(match):
        addresses.append(match.group(0))
        return ADDRESS_MARK % (len(addresses) - 1)

    return UPSTREAM_ADDRESS.sub(take, text), addresses


def restore_addresses(text, addresses):
    for index, address in enumerate(addresses):
        text = text.replace(ADDRESS_MARK % index, address)
    return text


def apply_text(text, tokens=None, rules=None):
    """Apply the text rules in order, optionally counting surrounding tokens."""
    if rules is None:
        rules = _TEXT_RULES
    text, addresses = protect_addresses(text)
    for pattern, replacement in rules:
        if tokens is not None:
            for match in pattern.finditer(text):
                start, end = token_bounds(text, match.start(), match.end())
                token = text[start:end]
                context = text[start - 1:start]
                entry = tokens.get(token)
                if entry is None:
                    new = apply_text(context + token, rules=rules)[len(context):]
                    entry = {"new": new, "count": 0}
                    tokens[token] = entry
                entry["count"] += 1
        text = pattern.sub(replacement, text)
    return restore_addresses(text, addresses)


def rename_path(path):
    for pattern, replacement in _PATH_RULES:
        path = pattern.sub(replacement, path)
    return path


def is_text(data):
    if b"\0" in data[:8000]:
        return False
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


def is_dropped(path, app=False):
    for item in APP_DROP if app else DROP:
        if item.endswith("/"):
            if path == item[:-1] or path.startswith(item):
                return True
        elif path == item:
            return True
    return False


def is_verbatim(path):
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in VERBATIM)


def is_app_verbatim(path):
    if path == "LICENSE" or fnmatch.fnmatchcase(path, "licenses/*.txt"):
        return True
    if fnmatch.fnmatchcase(path, "assets/fonts/*"):
        return True
    if fnmatch.fnmatchcase(path, "defaults/icons/LICENSE*"):
        return True
    if fnmatch.fnmatchcase(path, "defaults/mods/win98/fonts/LICENSE*"):
        return True
    if path.startswith("vendor/"):
        name = path.rsplit("/", 1)[-1]
        if name.startswith("LICENSE") or name.startswith("NOTICE"):
            return True
    return False


def is_vendor(path):
    return path.startswith("vendor/")


def allowed(path, text, match):
    """Whether this match is a word the allow table names for this path."""
    if path.startswith("app/"):
        path = path[len("app/"):]
    start, end = token_bounds(text, match.start(), match.end())
    return (path, text[start:end]) in ALLOWED


def allowed_word(path, word):
    if path.startswith("app/"):
        path = path[len("app/"):]
    return (path, word) in ALLOWED


def same_line_endings(original, formatted):
    """rustfmt writes LF; keep a CRLF-only file on CRLF."""
    if b"\r\n" in original and b"\n" not in original.replace(b"\r\n", b""):
        return formatted.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
    return formatted


def line_problems(path, text, patterns, label):
    problems = []
    for number, line in enumerate(text.splitlines(), 1):
        for pattern in patterns:
            match = pattern.search(line)
            if match and not allowed(path, line, match):
                problems.append("%s:%d: %s %r" % (path, number, label, match.group(0)))
                break
    return problems


def reserved_problems(path, text):
    problems = []
    for number, line in enumerate(text.splitlines(), 1):
        for word in RESERVED:
            if word in line and not allowed_word(path, word):
                problems.append("%s:%d: reserved %r" % (path, number, word))
                break
    return problems


def rename_dependency(entry, app=False):
    if entry == "herdr-core" or entry.startswith("herdr-core "):
        return "limen-core" + entry[len("herdr-core"):]
    if app and (entry == "herdr-app" or entry.startswith("herdr-app ")):
        return "limen-app" + entry[len("herdr-app"):]
    return entry


def version_key(version):
    """Cargo's order: 0.9.0 before 0.10.0, and 1.0.0-rc.1 before 1.0.0."""
    core, _, pre = version.partition("+")[0].partition("-")
    numbers = tuple(int(part) if part.isdigit() else 0 for part in core.split("."))
    if not pre:
        return (numbers, 1, ())
    return (numbers, 0, tuple((0, int(part), "") if part.isdigit() else (1, 0, part) for part in pre.split(".")))


def dependency_key(entry):
    name, _, rest = entry.partition(" ")
    return (name, version_key(rest.partition(" ")[0]))


def rewrite_dependencies(match, app=False):
    inner = match.group(1)[1:-1]
    entries = re.findall(r'"([^"]*)"', inner)
    renamed = [rename_dependency(entry, app) for entry in entries]
    if renamed == entries:
        return match.group(0)
    ordered = sorted(renamed, key=dependency_key)
    if "\n" in inner:
        body = "".join(' "%s",\n' % entry for entry in ordered)
        return "dependencies = [\n" + body + "]"
    return "dependencies = [" + ", ".join('"%s"' % entry for entry in ordered) + "]"


def lock_key(block):
    name = re.search(r'(?m)^name = "(.*)"$', block)
    version = re.search(r'(?m)^version = "(.*)"$', block)
    return (name.group(1) if name else "", version_key(version.group(1) if version else ""))


def rewrite_lock(text, app=False):
    newline = "\n"
    if "\r\n" in text and "\n" not in text.replace("\r\n", ""):
        newline = "\r\n"
        text = text.replace("\r\n", "\n")
    marker = "[[package]]"
    index = text.find(marker)
    if index < 0:
        return text.replace("\n", newline)
    header = text[:index]
    blocks = [marker + chunk for chunk in text[index:].split(marker)[1:]]
    fixed = []
    for block in blocks:
        body = block.rstrip("\n")
        body = re.sub(r'(?m)^name = "herdr-core"$', 'name = "limen-core"', body)
        if app:
            body = re.sub(r'(?m)^name = "herdr-app"$', 'name = "limen-app"', body)
        body = DEPENDENCIES.sub(lambda match: rewrite_dependencies(match, app), body)
        fixed.append(body)
    fixed.sort(key=lock_key)
    return (header + "\n\n".join(fixed) + "\n").replace("\n", newline)


def lock_inputs(lock, workspace_names):
    """What upstream's tests/notices.rs hashes for Cargo.lock."""
    packages = []
    for package in lock.split("[[package]]")[1:]:
        def field(key):
            for line in package.splitlines():
                if line.startswith(key):
                    rest = line[len(key):]
                    if rest.startswith(' = "') and rest.endswith('"'):
                        return rest[4:-1]
            return None

        name = field("name")
        version = field("version")
        source = field("source") or ""
        checksum = field("checksum") or ""
        if source == "" and name in workspace_names:
            continue
        packages.append("%s\t%s\t%s\t%s" % (name, version, source, checksum))
    packages.sort()
    return "\n".join(packages)


def manifest_inputs(manifest):
    """What upstream's tests/notices.rs hashes for Cargo.toml."""
    lines = []
    in_package = False
    for line in manifest.split("\n"):
        if line.startswith("["):
            in_package = line == "[package]"
        if in_package and line.startswith("version = "):
            lines.append('version = "<workspace version>"')
        else:
            lines.append(line)
    return "\n".join(lines)


def package_name(manifest):
    section = None
    for line in manifest.splitlines():
        if line.startswith("["):
            section = line.strip()
        elif section == "[package]":
            match = re.match(r'\s*name = "([^"]+)"', line)
            if match:
                return match.group(1)
    return None


def notices_hash(path, data, workspace_names):
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return None
    text = text.replace("\r\n", "\n")
    if path == "Cargo.lock":
        text = lock_inputs(text, workspace_names)
    elif path == "Cargo.toml":
        text = manifest_inputs(text)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def read_tree(sha):
    entries = []
    for record in git(["ls-tree", "-r", "-z", sha]).split(b"\0"):
        if not record:
            continue
        meta, path = record.split(b"\t", 1)
        mode, kind, oid = meta.split(b" ")
        entries.append((mode.decode(), kind.decode(), oid.decode(), path))
    return entries


def read_blobs(oids):
    if not oids:
        return {}
    payload = ("\n".join(sorted(set(oids))) + "\n").encode()
    result = run(["git", "cat-file", "--batch"], input=payload)
    if result.returncode:
        message = result.stderr.decode("utf-8", "replace").strip()
        raise GitError("git cat-file --batch failed: %s" % message)
    data = result.stdout
    blobs = {}
    position = 0
    while position < len(data):
        end = data.index(b"\n", position)
        header = data[position:end].decode("utf-8", "replace").split(" ")
        if len(header) < 3:
            raise GitError("git cat-file --batch: %s" % " ".join(header))
        size = int(header[2])
        start = end + 1
        blobs[header[0]] = data[start:start + size]
        position = start + size + 1
    return blobs


def write_blob(data):
    return git(["hash-object", "-w", "--stdin", "--no-filters"], input=data).decode().strip()


def write_tree(entries):
    directory = tempfile.mkdtemp(prefix="limenize-index-")
    index = os.path.join(directory, "index")
    env = dict(os.environ, GIT_INDEX_FILE=index)
    try:
        payload = b"".join(
            ("%s %s\t" % (mode, oid)).encode() + encode_path(path) + b"\0"
            for mode, oid, path in entries
        )
        git(["update-index", "-z", "--index-info"], input=payload, env=env)
        return git(["write-tree"], env=env).decode().strip()
    finally:
        shutil.rmtree(directory, ignore_errors=True)


class Renamer:
    def __init__(self, sha, rustfmt_dir, app=False):
        self.sha = sha
        self.rustfmt_dir = rustfmt_dir
        self.app = app
        self.problems = []
        self.counts = dict.fromkeys(
            ("files", "changed", "moved", "dropped", "verbatim", "binary", "formatted"), 0
        )
        self.tokens = {}
        self.entries = []
        self.seen_paths = {}
        self.notice_inputs = set()
        self.notice_outputs = {}
        self.manifests = {}
        self.notices_text = None

    def problem(self, path, line, text):
        self.problems.append("%s:%d: %s" % (path, line, text))

    def build(self):
        tree = read_tree(self.sha)
        blobs = read_blobs(
            [oid for mode, kind, oid, path in tree if kind != "commit"]
        )
        if self.app:
            self.read_notice_inputs(tree, blobs)
        for mode, kind, oid, raw_path in tree:
            self.add(mode, kind, oid, raw_path, blobs)
        if self.app:
            self.finalize_notices()
        return write_tree(self.entries)

    def read_notice_inputs(self, tree, blobs):
        for mode, kind, oid, raw_path in tree:
            if decode_path(raw_path) == "THIRD-PARTY-NOTICES" and kind != "commit":
                text = blobs[oid].decode("utf-8", "replace")
                self.notice_inputs = set(
                    re.findall(r"(?m)^Input-SHA256: [0-9a-f]{64} ([^\r\n]+)$", text)
                )
                return

    def add(self, mode, kind, oid, raw_path, blobs):
        path = decode_path(raw_path)
        self.counts["files"] += 1
        if is_dropped(path, self.app):
            self.counts["dropped"] += 1
            return
        new_path = path if kind == "commit" else rename_path(path)
        if self.app:
            new_path = "app/" + new_path
        self.scan_path(new_path)
        if new_path in self.seen_paths:
            self.problem(
                new_path,
                0,
                "duplicate path from %r and %r" % (self.seen_paths[new_path], path),
            )
            return
        self.seen_paths[new_path] = path
        if kind == "commit":
            self.entries.append((mode, oid, new_path))
            return
        if new_path != ("app/" + path if self.app else path):
            self.counts["moved"] += 1
        data = blobs[oid]
        new_data = data
        verbatim = is_app_verbatim(path) if self.app else is_verbatim(path)
        if verbatim:
            self.counts["verbatim"] += 1
        elif path == "Cargo.lock":
            new_data = self.rewrite_lock_file(new_path, data)
        elif not self.app and path in LICENSE_FILES:
            new_data = self.rewrite_licence_file(path, new_path, data)
        elif self.app and path == "THIRD-PARTY-NOTICES":
            new_data = self.rewrite_notices_file(new_path, data)
        elif is_text(data):
            new_data = self.rewrite_text_file(path, new_path, data)
        else:
            self.counts["binary"] += 1
            if b"herdr" in data:
                self.problem(new_path, 0, "binary contains 'herdr'")
        if self.app:
            relative = new_path[len("app/"):]
            if path in self.notice_inputs:
                self.notice_outputs[relative] = new_data
            if path.endswith("Cargo.toml") and not is_vendor(path):
                self.manifests[relative] = new_data
        if new_data != data:
            self.counts["changed"] += 1
            oid = write_blob(new_data)
        self.entries.append((mode, oid, new_path))

    def scan_path(self, path):
        relative = path[len("app/"):] if path.startswith("app/") else path
        patterns = _VENDOR_LEFTOVER if is_vendor(relative) else _LEFTOVER
        for pattern in patterns:
            match = pattern.search(path)
            if match and not allowed(path, path, match):
                self.problem(path, 0, "leftover %r" % match.group(0))
                return
        for pattern in _WRONG:
            match = pattern.search(path)
            if match and not allowed(path, path, match):
                self.problem(path, 0, "wrong mapping %r" % match.group(0))
                return
        for word in RESERVED:
            if word in path and not allowed_word(path, word):
                self.problem(path, 0, "reserved %r" % word)
                return

    def scan_collisions(self, path, text):
        for number, line in enumerate(text.splitlines(), 1):
            for pattern in _COLLISION:
                match = pattern.search(line)
                if match and not allowed(path, line, match):
                    self.problem(path, number, "collision %r" % match.group(0))
                    break

    def scan_post(self, path, text, vendor=False):
        scan = text.replace(NEW_LICENSE, "")
        scan = UPSTREAM_ADDRESS.sub("", scan)
        patterns = _VENDOR_LEFTOVER if vendor else _LEFTOVER
        for problem in line_problems(path, scan, patterns, "leftover"):
            self.problems.append(problem)
        for problem in line_problems(path, scan, _WRONG, "wrong mapping"):
            self.problems.append(problem)
        for problem in reserved_problems(path, scan):
            self.problems.append(problem)

    def rewrite_text_file(self, path, new_path, data):
        text = data.decode("utf-8")
        vendor = self.app and is_vendor(path)
        if not vendor:
            self.scan_collisions(new_path, text)
        rules = _VENDOR_RULES if vendor else _TEXT_RULES
        new_text = apply_text(text, self.tokens, rules)
        self.scan_post(new_path, new_text, vendor)
        new_data = new_text.encode("utf-8")
        if path.endswith(".rs") and new_data != data and not vendor:
            formatted = self.run_rustfmt(new_path, new_data)
            if formatted is not None:
                new_data = same_line_endings(data, formatted)
        return new_data

    def rewrite_notices_file(self, path, data):
        if not is_text(data):
            self.problem(path, 0, "not valid UTF-8")
            return data
        text = data.decode("utf-8")
        self.scan_collisions(path, text)
        new_text = apply_text(text, self.tokens)
        self.scan_post(path, new_text)
        newline = "\n"
        if "\r\n" in new_text and "\n" not in new_text.replace("\r\n", ""):
            newline = "\r\n"
        match = re.search(r"(?m)^=+\r?\n", new_text)
        if match:
            new_text = (
                new_text[: match.end()]
                + APP_LICENSE_LINE
                + newline
                + new_text[match.end():]
            )
        else:
            self.problem(path, 0, "third-party notices has no title underline")
        self.notices_text = new_text
        return new_text.encode("utf-8")

    def workspace_names(self):
        names = set()
        for path, data in self.manifests.items():
            if is_vendor(path):
                continue
            try:
                name = package_name(data.decode("utf-8"))
            except UnicodeDecodeError:
                name = None
            if name:
                names.add(name)
        return names

    def finalize_notices(self):
        if self.notices_text is None:
            return
        workspace = self.workspace_names()

        def replace(match):
            path = match.group(1)
            data = self.notice_outputs.get(path)
            if data is None:
                if path.startswith("vendor/ghostty/"):
                    # The submodule's bytes are not part of this tree and do
                    # not change, so upstream's line still holds.
                    return match.group(0)
                self.problem("app/THIRD-PARTY-NOTICES", 0, "missing notice input %r" % path)
                return match.group(0)
            digest = notices_hash(path, data, workspace)
            if digest is None:
                self.problem("app/THIRD-PARTY-NOTICES", 0, "notice input %r is not UTF-8" % path)
                return match.group(0)
            return "Input-SHA256: %s %s" % (digest, path)

        new_text = re.sub(
            r"(?m)^Input-SHA256: [0-9a-f]{64} ([^\r\n]+)$", replace, self.notices_text
        )
        if new_text == self.notices_text:
            return
        for index, (mode, oid, entry_path) in enumerate(self.entries):
            if entry_path == "app/THIRD-PARTY-NOTICES":
                self.entries[index] = (mode, write_blob(new_text.encode("utf-8")), entry_path)
                return

    def rewrite_licence_file(self, path, new_path, data):
        if not is_text(data):
            self.problem(new_path, 0, "not valid UTF-8")
            return data
        text = data.decode("utf-8")
        self.scan_collisions(new_path, text)
        if path == "crates/herdr-core/THIRD-PARTY-NOTICES":
            new_text = text.replace(OLD_LICENSE, NEW_LICENSE, 1)
        else:
            marker = "\x01licence\x01"
            new_text = text.replace(OLD_LICENSE, marker, 1)
            new_text = apply_text(new_text, self.tokens)
            new_text = new_text.replace(marker, NEW_LICENSE, 1)
        if OLD_LICENSE not in text:
            self.problem(new_path, 0, "missing licence line %r" % OLD_LICENSE)
        self.scan_post(new_path, new_text)
        return new_text.encode("utf-8")

    def rewrite_lock_file(self, path, data):
        if not is_text(data):
            self.problem(path, 0, "not valid UTF-8")
            return data
        new_text = rewrite_lock(data.decode("utf-8"), self.app)
        for number, line in enumerate(new_text.splitlines(), 1):
            if "herdr" in line:
                self.problem(path, number, "leftover 'herdr'")
        return new_text.encode("utf-8")

    def run_rustfmt(self, path, data):
        command = os.environ.get("RUSTFMT") or "rustfmt"
        try:
            result = run(
                [command, "--edition", "2024", "--config-path", self.rustfmt_dir],
                input=data,
            )
        except OSError as exc:
            self.problem(path, 0, "rustfmt: %s" % exc)
            return None
        if result.returncode:
            stderr = result.stderr.decode("utf-8", "replace").strip()
            first = stderr.splitlines()[0] if stderr else "exit %d" % result.returncode
            self.problem(path, 0, "rustfmt: %s" % first)
            return None
        self.counts["formatted"] += 1
        return result.stdout


def rustfmt_config_dir(ref):
    directory = tempfile.mkdtemp(prefix="limenize-rustfmt-")
    result = run(["git", "show", "%s:rustfmt.toml" % ref])
    with open(os.path.join(directory, "rustfmt.toml"), "wb") as handle:
        handle.write(result.stdout if result.returncode == 0 else b"")
    return directory


def resolve(ref):
    sha = git_optional(["rev-parse", "--verify", "--end-of-options", ref + "^{commit}"])
    if not sha:
        raise GitError("cannot resolve %r to a commit" % ref)
    return sha


def current_branch():
    return git_optional(["symbolic-ref", "--short", "-q", "HEAD"])


def ref_tip(name):
    return git_optional(["rev-parse", "--verify", "--end-of-options", name + "^{commit}"])


def ref_tree(name):
    return git_optional(["rev-parse", "--verify", "--end-of-options", name + "^{tree}"])


def commit_message(sha, trailers):
    lines = [
        "Upstream %s, names changed by scripts/limenize.py" % sha[:12],
        "",
        "Upstream: %s" % sha,
    ]
    lines.extend(trailers)
    return "\n".join(lines) + "\n"


def make_commit(tree, parents, message):
    args = ["commit-tree", tree]
    for parent in parents:
        args.extend(["-p", parent])
    return git(args, input=message.encode("utf-8")).decode().strip()


def write_report(path, report):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")


def parse_args(argv):
    parser = argparse.ArgumentParser(prog="limenize.py")
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("check", help="build and report, move no ref")
    sync = commands.add_parser("sync", help="build and record a commit on a branch")
    for command in (check, sync):
        command.add_argument("--app", action="store_true", help="rename upstream's app under app/")
        command.add_argument("--ref")
        command.add_argument("--report")
    sync.add_argument("--branch")
    sync.add_argument("--trailer", action="append", default=[])
    args = parser.parse_args(argv)
    if args.ref is None:
        args.ref = "upstream-app/master" if args.app else "upstream/master"
    if args.command == "sync" and args.branch is None:
        args.branch = "upstream-app-renamed" if args.app else "upstream-renamed"
    return args


def summary(args, sha, tree, counts, problems, up_to_date, commit):
    print("limenize: %s %s -> tree %s" % (args.ref, sha[:12], tree), file=sys.stderr)
    print(
        "counts: "
        + ", ".join("%s=%d" % (key, counts[key]) for key in counts),
        file=sys.stderr,
    )
    if up_to_date:
        print("up to date", file=sys.stderr)
    if commit:
        print("commit %s on %s" % (commit, args.branch), file=sys.stderr)
    if problems:
        print("problems: %d" % len(problems), file=sys.stderr)
    else:
        print("problems: 0", file=sys.stderr)


def run_command(args):
    sha = resolve(args.ref)
    if args.command == "sync" and current_branch() == args.branch:
        print(
            "limenize: refusing to update %s: it is checked out in this worktree"
            % args.branch,
            file=sys.stderr,
        )
        return 1

    config_dir = rustfmt_config_dir(args.ref)
    try:
        renamer = Renamer(sha, config_dir, app=args.app)
        tree = renamer.build()
    finally:
        shutil.rmtree(config_dir, ignore_errors=True)

    branch = None
    up_to_date = False
    new_commit = None
    if args.command == "sync":
        branch = {"name": args.branch, "sha": ref_tip("refs/heads/" + args.branch)}
        if branch["sha"] and ref_tree(branch["sha"]) == tree:
            up_to_date = True
        elif not renamer.problems:
            parents = ([branch["sha"]] if branch["sha"] else []) + [sha]
            new_commit = make_commit(tree, parents, commit_message(sha, args.trailer))
            git(
                [
                    "update-ref",
                    "refs/heads/" + args.branch,
                    new_commit,
                    branch["sha"] or "0" * len(sha),
                ]
            )
            branch["sha"] = new_commit

    result = {
        "ref": args.ref,
        "sha": sha,
        "tree": tree,
        "branch": branch,
        "counts": renamer.counts,
        "problems": renamer.problems[:50],
        "problems_total": len(renamer.problems),
    }
    if args.report:
        report = dict(result)
        report["problems"] = renamer.problems
        report["tokens"] = renamer.tokens

    summary(
        args,
        sha,
        tree,
        renamer.counts,
        renamer.problems,
        up_to_date,
        new_commit,
    )
    print(json.dumps(result, sort_keys=True))
    if args.report:
        try:
            write_report(args.report, report)
        except OSError as exc:
            print("limenize: cannot write report: %s" % exc, file=sys.stderr)
            return 1
    return 1 if renamer.problems else 0


def main(argv=None):
    args = parse_args(argv)
    try:
        return run_command(args)
    except GitError as exc:
        print("limenize: %s" % exc, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
