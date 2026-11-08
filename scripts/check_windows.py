#!/usr/bin/env python3
"""`just check` for a Windows PC without `just`: format, clippy on both systems, the Windows suite.

With --focused, the task tests and the docs' stand in for the suite: what a worker runs beside other workers,
each on its own build folder, since several whole suites at once slow each other into failing.

Run it in a hidden console: a worker's, or through scripts/hidden_run.py on the Owner's PC. It runs every
step even after a failure, then repeats the end of each failed step and prints unittest-shaped lines,
which is what Kompitum's checks read.
"""
import collections
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NO_WINDOW = 0x08000000
# Writes a throwaway value to the real HKCU\Environment, so it runs only when the Owner agrees.
SKIP = "registry_changes_after_start_are_seen_by_each_new_terminal"
RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed")
FAILED = re.compile(r"^test (\S+) \.\.\. FAILED")


def touch_crate_roots() -> None:
    """Cargo judges freshness by file times, and a shared build folder may hold another worktree's build."""
    for crate in (ROOT / "crates").iterdir():
        for name in ("src/lib.rs", "src/main.rs", "src/bin"):
            path = crate / name
            for file in path.glob("*.rs") if path.is_dir() else [path] if path.exists() else []:
                os.utime(file)


def step(name: str, argv: list, env: dict = None) -> tuple:
    """Runs one step, streaming its output. Returns (passed, the last lines, tests run, failed test names)."""
    print(f"--- {name}: {' '.join(argv)}", flush=True)
    process = subprocess.Popen(argv, cwd=ROOT, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, encoding="utf-8", errors="replace",
                               creationflags=NO_WINDOW)
    last, ran, failed = collections.deque(maxlen=40), 0, []
    for line in process.stdout:
        print(line, end="", flush=True)
        last.append(line)
        result = RESULT.match(line)
        if result:
            ran += int(result.group(1)) + int(result.group(2))
        name_failed = FAILED.match(line)
        if name_failed:
            failed.append(name_failed.group(1))
    return process.wait() == 0, "".join(last), ran, failed


def main() -> int:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    started = time.monotonic()
    touch_crate_roots()
    metadata = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT,
                              stdin=subprocess.DEVNULL, capture_output=True, encoding="utf-8", errors="replace",
                              creationflags=NO_WINDOW)
    if metadata.returncode != 0:
        print(metadata.stderr)
        print("ERROR: cargo metadata")
        print("FAILED (errors=1)")
        return 1
    target = Path(json.loads(metadata.stdout)["target_directory"])
    # Linux code doesn't compile on Windows, so it gets a type check, with Zig stubbed and a build folder
    # of its own so the stub's empty library never meets a real build.
    linux = dict(os.environ, ZIG=str(ROOT / "scripts" / "zig-stub.cmd"), CARGO_TARGET_DIR=str(target / "lint-linux"))
    # A test binary's watchdog is set for a quiet PC. On a busy one it aborts a healthy run, and the abort
    # loses the words of any failure that run had already found.
    patient = dict(os.environ, KOMPITUM_TEST_WATCHDOG_S="300")
    if "--focused" in sys.argv:
        tests = [
            ("the worker loop's unit tests",
             ["cargo", "test", "--locked", "-p", "kompitum-core", "--lib", "--no-fail-fast", "--", "task::"], patient),
            ("the worker loop's tests",
             ["cargo", "test", "--locked", "-p", "kompitum-core", "--test", "task", "--no-fail-fast"], patient),
            ("the docs and the schema",
             ["cargo", "test", "--locked", "-p", "kompitum-core", "--test", "cli_docs", "--test", "schema_file"], None),
        ]
    else:
        tests = [("the Windows suite", ["cargo", "test", "--locked", "--no-fail-fast", "--", "--skip", SKIP], patient)]
    steps = [
        ("format", ["cargo", "fmt", "--all", "--", "--check"], None),
        ("clippy for Windows", ["cargo", "clippy", "--locked", "--all-targets", "--", "-D", "warnings"], None),
        ("clippy for Linux", ["cargo", "clippy", "--locked", "--target", "x86_64-unknown-linux-musl",
                              "--all-targets", "--", "-D", "warnings"], linux),
        *tests,
    ]
    results = [(name, *step(name, argv, env)) for name, argv, env in steps]
    bad = [result for result in results if not result[1]]
    for name, _, last, _, _ in bad:
        print(f"--- the end of {name}\n{last}", flush=True)
    for name, _, _, _, failed in bad:
        for test in failed:
            print(f"FAIL: {test}")
        if not failed:
            print(f"ERROR: {name}")
    print(f"Ran {sum(result[3] for result in results)} tests in {time.monotonic() - started:.0f}s")
    print(f"FAILED (steps={len(bad)})" if bad else "OK")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
