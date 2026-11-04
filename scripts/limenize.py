#!/usr/bin/env python3
"""Build Limen's renamed tree from an upstream herdr-core commit.

    python scripts/limenize.py check [--ref REF] [--report PATH]
    python scripts/limenize.py sync  [--ref REF] [--branch NAME] [--trailer TEXT]... [--report PATH]

Both commands read git objects only: the working tree, the real index, HEAD and
every ref except ``refs/heads/<branch>`` are never touched.

``counts.files`` counts the entries in REF's tree; ``dropped`` is the subset
left out. Problems that are not tied to a line (a path, a missing licence
line, a failed rustfmt) report line 0. In JSON, ``branch`` is
``{"name": ..., "sha": ...}`` (null for check) and each ``tokens`` entry is
``{"new": ..., "count": ...}``.
"""

import argparse
import fnmatch
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

EDGE = r"(?:(?<![A-Za-z0-9])|(?<=\\[nrt]))"

TEXT_RULES = (
    (r"releases\.herdr\.dev", "releases.limen.invalid"),
    (r"herdr", "limen"),
    (r"Herdr", "Limen"),
    (r"HERDR", "LIMEN"),
    (r"nohc", "nolc"),
    (EDGE + r"hcs(?![A-Za-z0-9])", "lcs"),
    (EDGE + r"hc(?![A-Za-z0-9])", "lc"),
    (EDGE + r"HC_", "LIMEN_"),
    (EDGE + r"HC(?![A-Za-z0-9_])", "LC"),
)

PATH_RULES = TEXT_RULES[1:4] + TEXT_RULES[5:7]

LEFTOVER = (
    r"(?i)herdr",
    r"(?<![A-Za-z0-9])hc",
    r"[A-Za-z0-9]hc(?![A-Za-z0-9])",
    r"(?<![A-Za-z0-9])HC(?![A-Za-z0-9])",
)

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

_TEXT_RULES = tuple((re.compile(p), r) for p, r in TEXT_RULES)
_PATH_RULES = tuple((re.compile(p), r) for p, r in PATH_RULES)
_LEFTOVER = tuple(re.compile(p) for p in LEFTOVER)
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


def apply_text(text, tokens=None):
    """Apply the text rules in order, optionally counting surrounding tokens."""
    for pattern, replacement in _TEXT_RULES:
        if tokens is not None:
            for match in pattern.finditer(text):
                start, end = token_bounds(text, match.start(), match.end())
                token = text[start:end]
                context = text[start - 1:start]
                entry = tokens.get(token)
                if entry is None:
                    new = apply_text(context + token)[len(context):]
                    entry = {"new": new, "count": 0}
                    tokens[token] = entry
                entry["count"] += 1
        text = pattern.sub(replacement, text)
    return text


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


def is_dropped(path):
    for item in DROP:
        if item.endswith("/"):
            if path == item[:-1] or path.startswith(item):
                return True
        elif path == item:
            return True
    return False


def is_verbatim(path):
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in VERBATIM)


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
            if match:
                problems.append("%s:%d: %s %r" % (path, number, label, match.group(0)))
                break
    return problems


def reserved_problems(path, text):
    problems = []
    for number, line in enumerate(text.splitlines(), 1):
        for word in RESERVED:
            if word in line:
                problems.append("%s:%d: reserved %r" % (path, number, word))
                break
    return problems


def rename_dependency(entry):
    if entry == "herdr-core" or entry.startswith("herdr-core "):
        return "limen-core" + entry[len("herdr-core"):]
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


def rewrite_dependencies(match):
    inner = match.group(1)[1:-1]
    entries = re.findall(r'"([^"]*)"', inner)
    renamed = [rename_dependency(entry) for entry in entries]
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


def rewrite_lock(text):
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
        body = DEPENDENCIES.sub(rewrite_dependencies, body)
        fixed.append(body)
    fixed.sort(key=lock_key)
    return (header + "\n\n".join(fixed) + "\n").replace("\n", newline)


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
    def __init__(self, sha, rustfmt_dir):
        self.sha = sha
        self.rustfmt_dir = rustfmt_dir
        self.problems = []
        self.counts = dict.fromkeys(
            ("files", "changed", "moved", "dropped", "verbatim", "binary", "formatted"), 0
        )
        self.tokens = {}
        self.entries = []
        self.seen_paths = {}

    def problem(self, path, line, text):
        self.problems.append("%s:%d: %s" % (path, line, text))

    def build(self):
        tree = read_tree(self.sha)
        blobs = read_blobs(
            [oid for mode, kind, oid, path in tree if kind != "commit"]
        )
        for mode, kind, oid, raw_path in tree:
            self.add(mode, kind, oid, raw_path, blobs)
        return write_tree(self.entries)

    def add(self, mode, kind, oid, raw_path, blobs):
        path = decode_path(raw_path)
        self.counts["files"] += 1
        if is_dropped(path):
            self.counts["dropped"] += 1
            return
        new_path = path if kind == "commit" else rename_path(path)
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
        if new_path != path:
            self.counts["moved"] += 1
        data = blobs[oid]
        new_data = data
        if is_verbatim(path):
            self.counts["verbatim"] += 1
        elif path == "Cargo.lock":
            new_data = self.rewrite_lock_file(new_path, data)
        elif path in LICENSE_FILES:
            new_data = self.rewrite_licence_file(path, new_path, data)
        elif is_text(data):
            new_data = self.rewrite_text_file(new_path, data)
        else:
            self.counts["binary"] += 1
            if b"herdr" in data:
                self.problem(new_path, 0, "binary contains 'herdr'")
        if new_data != data:
            self.counts["changed"] += 1
            oid = write_blob(new_data)
        self.entries.append((mode, oid, new_path))

    def scan_path(self, path):
        for pattern in _LEFTOVER:
            match = pattern.search(path)
            if match:
                self.problem(path, 0, "leftover %r" % match.group(0))
                return
        for pattern in _WRONG:
            match = pattern.search(path)
            if match:
                self.problem(path, 0, "wrong mapping %r" % match.group(0))
                return
        for word in RESERVED:
            if word in path:
                self.problem(path, 0, "reserved %r" % word)
                return

    def scan_collisions(self, path, text):
        for number, line in enumerate(text.splitlines(), 1):
            for pattern in _COLLISION:
                match = pattern.search(line)
                if match:
                    self.problem(path, number, "collision %r" % match.group(0))
                    break

    def scan_post(self, path, text):
        scan = text.replace(NEW_LICENSE, "")
        for problem in line_problems(path, scan, _LEFTOVER, "leftover"):
            self.problems.append(problem)
        for problem in line_problems(path, scan, _WRONG, "wrong mapping"):
            self.problems.append(problem)
        for problem in reserved_problems(path, scan):
            self.problems.append(problem)

    def rewrite_text_file(self, path, data):
        text = data.decode("utf-8")
        self.scan_collisions(path, text)
        new_text = apply_text(text, self.tokens)
        self.scan_post(path, new_text)
        new_data = new_text.encode("utf-8")
        if path.endswith(".rs") and new_data != data:
            formatted = self.run_rustfmt(path, new_data)
            if formatted is not None:
                new_data = same_line_endings(data, formatted)
        return new_data

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
        new_text = rewrite_lock(data.decode("utf-8"))
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
        command.add_argument("--ref", default="upstream/master")
        command.add_argument("--report")
    sync.add_argument("--branch", default="upstream-renamed")
    sync.add_argument("--trailer", action="append", default=[])
    return parser.parse_args(argv)


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
        renamer = Renamer(sha, config_dir)
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
