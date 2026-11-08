"""Run a command with no window and record every window that opens on the desktop meanwhile.

    python scripts/watch_windows.py [--control] <report file> <log file> <folder> <command...>

The command runs as in hidden_run.py. Each top-level window shown and each change of foreground
window, from any program, is one JSON line in the report: seconds since the start, the event, the
program, the window class and its size. A title and the program's path are kept only for a suspect,
meaning a console or a program the command might have started, so the Owner's other windows stay
out of it. The summary goes to stdout and the exit code is the command's.

--control first opens one minimized console for about a second and stops with exit code 3 if the
watcher doesn't see it, since finding no windows means nothing from a watcher that can't see them.
"""
import ctypes
import ctypes.wintypes as w
import json
import os
import subprocess
import sys
import threading
import time
from collections import Counter

from hidden_run import run

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

EVENT_SYSTEM_FOREGROUND = 0x0003
EVENT_OBJECT_SHOW = 0x8002
WINEVENT_OUTOFCONTEXT = 0
OBJID_WINDOW = 0
GA_ROOT = 2
WM_QUIT = 0x0012
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
SW_SHOWMINNOACTIVE = 7

CONSOLE_CLASSES = {"ConsoleWindowClass", "CASCADIA_HOSTING_WINDOW_CLASS", "PseudoConsoleWindow"}
TREE_PROGRAMS = {
    "bash.exe", "bun.exe", "cargo.exe", "claude.exe", "cmd.exe", "commandcode.exe", "conhost.exe", "git.exe", "kom.exe",
    "kompitum-core.exe", "link.exe", "node.exe", "npx.exe", "opencode.exe", "openconsole.exe", "ping.exe",
    "powershell.exe", "pwsh.exe", "python.exe", "reg.exe", "rustc.exe", "scp.exe", "sh.exe", "ssh.exe", "tar.exe",
    "tasklist.exe", "timeout.exe", "uv.exe", "uvx.exe", "whoami.exe", "windowsterminal.exe", "zig.exe",
}

WinEventProc = ctypes.WINFUNCTYPE(None, w.HANDLE, w.DWORD, w.HWND, w.LONG, w.LONG, w.DWORD, w.DWORD)
user32.SetWinEventHook.argtypes = (w.DWORD, w.DWORD, w.HMODULE, WinEventProc, w.DWORD, w.DWORD, w.DWORD)
user32.SetWinEventHook.restype = w.HANDLE
user32.UnhookWinEvent.argtypes = (w.HANDLE,)
user32.GetAncestor.argtypes = (w.HWND, w.UINT)
user32.GetAncestor.restype = w.HWND
user32.IsWindowVisible.argtypes = (w.HWND,)
user32.IsIconic.argtypes = (w.HWND,)
user32.GetWindowThreadProcessId.argtypes = (w.HWND, ctypes.POINTER(w.DWORD))
user32.GetClassNameW.argtypes = (w.HWND, w.LPWSTR, ctypes.c_int)
user32.GetWindowTextW.argtypes = (w.HWND, w.LPWSTR, ctypes.c_int)
user32.GetWindowRect.argtypes = (w.HWND, ctypes.POINTER(w.RECT))
kernel32.OpenProcess.argtypes = (w.DWORD, w.BOOL, w.DWORD)
kernel32.OpenProcess.restype = w.HANDLE
kernel32.QueryFullProcessImageNameW.argtypes = (w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD))
kernel32.CloseHandle.argtypes = (w.HANDLE,)


def path_of(pid):
    process = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not process:
        return "?"
    try:
        path = ctypes.create_unicode_buffer(1024)
        size = w.DWORD(len(path))
        if kernel32.QueryFullProcessImageNameW(process, 0, path, ctypes.byref(size)):
            return path.value
        return "?"
    finally:
        kernel32.CloseHandle(process)


class Watcher:
    def __init__(self, report):
        self.report = open(report, "w", encoding="utf-8")
        self.start = time.monotonic()
        self.rows = []
        self.ready = threading.Event()
        self.failure = None
        self.thread_id = None
        self.proc = WinEventProc(self.on_event)  # Kept here: the hook calls it for as long as it lives.
        self.thread = threading.Thread(target=self.pump, daemon=True)

    def on_event(self, _hook, event, hwnd, id_object, id_child, _thread, _when):
        if id_object != OBJID_WINDOW or id_child != 0 or not hwnd:
            return
        if user32.GetAncestor(hwnd, GA_ROOT) != hwnd:
            return
        if event == EVENT_OBJECT_SHOW and not user32.IsWindowVisible(hwnd):
            return
        pid = w.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        path = path_of(pid.value)
        program = os.path.basename(path)
        name = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, name, len(name))
        rect = w.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(rect))
        suspect = name.value in CONSOLE_CLASSES or program.lower() in TREE_PROGRAMS
        row = {
            "t": round(time.monotonic() - self.start, 2),
            "event": "show" if event == EVENT_OBJECT_SHOW else "foreground",
            "program": program,
            "class": name.value,
            "size": [rect.right - rect.left, rect.bottom - rect.top],
            "minimized": bool(user32.IsIconic(hwnd)),
            "suspect": suspect,
        }
        if suspect:
            title = ctypes.create_unicode_buffer(256)
            user32.GetWindowTextW(hwnd, title, len(title))
            row["title"] = title.value
            row["path"] = path  # The Claude desktop app and the CLI are both claude.exe.
        self.rows.append(row)
        self.report.write(json.dumps(row) + "\n")
        self.report.flush()

    def pump(self):
        self.thread_id = kernel32.GetCurrentThreadId()
        message = w.MSG()
        user32.PeekMessageW(ctypes.byref(message), None, 0, 0, 0)  # Makes this thread's queue.
        hooks = [
            user32.SetWinEventHook(event, event, None, self.proc, 0, 0, WINEVENT_OUTOFCONTEXT)
            for event in (EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_SHOW)
        ]
        if not all(hooks):
            self.failure = f"SetWinEventHook failed: {ctypes.get_last_error()}"
        self.ready.set()
        while user32.GetMessageW(ctypes.byref(message), None, 0, 0) > 0:
            user32.TranslateMessage(ctypes.byref(message))
            user32.DispatchMessageW(ctypes.byref(message))
        for hook in hooks:
            if hook:
                user32.UnhookWinEvent(hook)

    def __enter__(self):
        self.thread.start()
        self.ready.wait()
        if self.failure:
            raise SystemExit(self.failure)
        return self

    def __exit__(self, *_):
        time.sleep(0.5)  # Lets the last windows' events arrive.
        user32.PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0)
        self.thread.join(5)
        self.report.close()


def control(watcher):
    """Opens a real console, minimized and without focus, and says whether the watcher saw it."""
    info = subprocess.STARTUPINFO()
    info.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = SW_SHOWMINNOACTIVE
    before = len(watcher.rows)
    subprocess.run(
        ["cmd", "/c", "ping -n 2 127.0.0.1 >nul"],
        creationflags=subprocess.CREATE_NEW_CONSOLE,
        startupinfo=info,
    )
    time.sleep(0.5)
    return [row for row in watcher.rows[before:] if row["event"] == "show" and row["class"] in CONSOLE_CLASSES]


def summary(rows, control_rows):
    if control_rows is not None:
        first = control_rows[0]
        print(f"control console seen: {first['program']} {first['class']} {first['size']} minimized={first['minimized']}")
    after = rows[rows.index(control_rows[-1]) + 1:] if control_rows else rows
    print(f"after the start: {len(after)} events")
    for (event, program, name), count in sorted(Counter((r["event"], r["program"], r["class"]) for r in after).items()):
        print(f"  {count:4} {event:10} {program} {name}")
    suspects = [r for r in after if r["suspect"]]
    print(f"suspects: {len(suspects)}")
    for row in suspects[:40]:
        print("  " + json.dumps(row))


def main():
    args = sys.argv[1:]
    with_control = args[0] == "--control"
    if with_control:
        args = args[1:]
    report, log, cwd, *command = args
    with Watcher(report) as watcher:
        control_rows = control(watcher) if with_control else None
        if with_control and not control_rows:
            print("the control console was not seen, so this watcher can't vouch for anything", flush=True)
            return 3
        code = run(log, cwd, command)
    summary(watcher.rows, control_rows)
    return code


if __name__ == "__main__":
    sys.exit(main())
