"""Run a command with no window, for builds and tests on a PC where the Owner may be in a game.

    python scripts/hidden_run.py <log file> <folder> <command...>

The whole process tree shares one hidden console, so the console programs cargo and zig start
open no window. A program that asks for a console of its own can still open one. Output is
appended to the log and the exit code is passed on. On Windows, write a relative program path
with backslashes: CreateProcess can't start one with forward slashes.
"""
import subprocess
import sys


def run(log, cwd, command):
    with open(log, "ab") as out:
        return subprocess.run(
            command,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=out,
            stderr=subprocess.STDOUT,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        ).returncode


if __name__ == "__main__":
    log, cwd, *command = sys.argv[1:]
    sys.exit(run(log, cwd, command))
