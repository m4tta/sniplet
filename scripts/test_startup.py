"""Check Windows startup window states without changing startup registration.

Run after `cargo build -p sniplet-app --locked`:
    python scripts/test_startup.py target/debug/sniplet.exe
Only the processes started by this script are stopped.
"""

import argparse
import ctypes
from ctypes import wintypes
from pathlib import Path
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("This check requires Windows.")
    executable = args.executable.resolve(strict=True)
    help_result = subprocess.run(
        [str(executable), "--help"], capture_output=True, timeout=15,
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    if help_result.returncode != 0 or b"--minimized" not in help_result.stdout:
        parser.error("Build the app with startup support before you run this check.")
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.IsIconic.argtypes = [wintypes.HWND]

    def windows(process_id):
        result = {}

        @callback_type
        def visit(handle, _):
            owner = wintypes.DWORD()
            user32.GetWindowThreadProcessId(handle, ctypes.byref(owner))
            if owner.value == process_id:
                title = ctypes.create_unicode_buffer(256)
                user32.GetWindowTextW(handle, title, len(title))
                if title.value in ("Sniplet", "Sniplet Settings"):
                    result[title.value] = bool(
                        user32.IsWindowVisible(handle) and not user32.IsIconic(handle)
                    )
            return True

        user32.EnumWindows(visit, 0)
        return result

    for flags, expected in [
        (["--minimized"], {"Sniplet": False}),
        ([], {"Sniplet": True}),
        (["--minimized", "--settings"], {"Sniplet": True, "Sniplet Settings": True}),
    ]:
        process = subprocess.Popen(
            [str(executable), *flags],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        try:
            deadline = time.monotonic() + 15
            stable_since = None
            state = {}
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f"{flags}: app exited with code {process.returncode}")
                state = windows(process.pid)
                if state == expected:
                    if stable_since is None:
                        stable_since = time.monotonic()
                    if time.monotonic() - stable_since >= 1:
                        break
                else:
                    stable_since = None
                time.sleep(0.1)
            else:
                raise RuntimeError(f"{flags}: expected {expected}, got {state}")
            print(f"PASS {flags or ['manual launch']}: {state}")
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)


if __name__ == "__main__":
    main()
