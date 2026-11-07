"""Check app mods with the built app, hidden: `python scripts/check_app_mods.py <mod folder>...`.

Runs `limen-app check` on each folder with a short app home of its own (a long one
fails on Windows), prints what it said, and fails if any check failed. The app is the
main checkout's debug build, or LIMEN_APP_EXE.
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_EXE = Path(os.environ.get("USERPROFILE", str(Path.home()))) / "Projects/limen-core/app/target/debug/limen-app.exe"
HOME = Path.home() / ".limen-scratch" / "mod-check"
NO_WINDOW = 0x08000000


def main() -> int:
    folders = sys.argv[1:]
    if not folders:
        print("usage: check_app_mods.py <mod folder>...")
        return 2
    exe = Path(os.environ.get("LIMEN_APP_EXE", str(DEFAULT_EXE)))
    if not exe.exists():
        print(f"no built app at {exe}: build it or set LIMEN_APP_EXE")
        return 2
    HOME.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, LIMEN_APP_HOME=str(HOME))
    flags = NO_WINDOW if os.name == "nt" else 0
    failed = 0
    for folder in folders:
        path = Path(folder)
        if not path.is_absolute():
            path = Path.cwd() / path
        done = subprocess.run([str(exe), "check", str(path)], env=env, capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=120, creationflags=flags)
        said = (done.stdout + done.stderr).strip()
        print(f"{folder}: exit {done.returncode}")
        if said:
            print(said)
        failed += done.returncode != 0
    print(f"checked {len(folders)} mods, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
