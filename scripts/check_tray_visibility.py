"""Check the Windows editor after minimizing it or restoring it from the tray.

    python scripts/check_tray_visibility.py --pid PROCESS_ID --expect tray
    python scripts/check_tray_visibility.py --pid PROCESS_ID --expect shown

This check reads window state. It does not send input or change the window.
"""

import argparse
import ctypes
from ctypes import wintypes
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--expect", choices=("tray", "shown"), required=True)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("This check requires Windows")

    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    signatures = {
        "EnumWindows": ([callback, wintypes.LPARAM], wintypes.BOOL),
        "GetWindowTextW": ([wintypes.HWND, wintypes.LPWSTR, ctypes.c_int], ctypes.c_int),
        "GetWindowThreadProcessId": (
            [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)], wintypes.DWORD
        ),
        "IsWindowVisible": ([wintypes.HWND], wintypes.BOOL),
        "IsIconic": ([wintypes.HWND], wintypes.BOOL),
    }
    for name, (argtypes, restype) in signatures.items():
        function = getattr(user32, name)
        function.argtypes = argtypes
        function.restype = restype

    editors = []

    @callback
    def collect(hwnd, _):
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if pid.value == args.pid:
            title = ctypes.create_unicode_buffer(512)
            user32.GetWindowTextW(hwnd, title, len(title))
            if title.value == "Sniplet":
                editors.append(hwnd)
        return True

    if not user32.EnumWindows(collect, 0):
        raise ctypes.WinError(ctypes.get_last_error())
    if len(editors) != 1:
        print(f"FAIL: Expected one retained editor for pid={args.pid}; found {len(editors)}")
        return 1

    visible = bool(user32.IsWindowVisible(editors[0]))
    minimized = bool(user32.IsIconic(editors[0]))
    matches = not visible if args.expect == "tray" else visible and not minimized
    print(
        f"{'PASS' if matches else 'FAIL'}: pid={args.pid} expect={args.expect} "
        f"visible={visible} minimized={minimized}; editor window retained"
    )
    return 0 if matches else 1


if __name__ == "__main__":
    sys.exit(main())
