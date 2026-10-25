"""Check an open Windows capture overlay without moving it or sending input.

Start an area capture, then run:
    python scripts/check_capture_alignment.py [--pid PROCESS_ID]
The client area must match the monitor bounds in physical pixels.
"""

import argparse
import ctypes
from ctypes import wintypes
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, help="Check only this Sniplet process")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("This check requires Windows")

    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    class MonitorInfo(ctypes.Structure):
        _fields_ = [
            ("cbSize", wintypes.DWORD),
            ("rcMonitor", wintypes.RECT),
            ("rcWork", wintypes.RECT),
            ("dwFlags", wintypes.DWORD),
        ]

    signatures = {
        "SetProcessDpiAwarenessContext": ([wintypes.HANDLE], wintypes.BOOL),
        "EnumWindows": ([callback, wintypes.LPARAM], wintypes.BOOL),
        "IsWindowVisible": ([wintypes.HWND], wintypes.BOOL),
        "GetWindowTextW": ([wintypes.HWND, wintypes.LPWSTR, ctypes.c_int], ctypes.c_int),
        "GetWindowThreadProcessId": (
            [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)], wintypes.DWORD
        ),
        "GetClientRect": ([wintypes.HWND, ctypes.POINTER(wintypes.RECT)], wintypes.BOOL),
        "ClientToScreen": ([wintypes.HWND, ctypes.POINTER(wintypes.POINT)], wintypes.BOOL),
        "MonitorFromWindow": ([wintypes.HWND, wintypes.DWORD], wintypes.HANDLE),
        "GetMonitorInfoW": ([wintypes.HANDLE, ctypes.POINTER(MonitorInfo)], wintypes.BOOL),
    }
    for name, (argtypes, restype) in signatures.items():
        function = getattr(user32, name)
        function.argtypes = argtypes
        function.restype = restype

    def checked(result):
        if not result:
            raise ctypes.WinError(ctypes.get_last_error())
        return result

    # Read all rectangles in physical pixels, including mixed-DPI displays.
    checked(user32.SetProcessDpiAwarenessContext(wintypes.HANDLE(-4)))
    windows = []

    @callback
    def collect(hwnd, _):
        if user32.IsWindowVisible(hwnd):
            title = ctypes.create_unicode_buffer(512)
            user32.GetWindowTextW(hwnd, title, len(title))
            if title.value.startswith("Sniplet") and "Capture area" in title.value:
                pid = wintypes.DWORD()
                user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
                if args.pid is None or args.pid == pid.value:
                    windows.append((hwnd, pid.value))
        return True

    checked(user32.EnumWindows(collect, 0))
    if not windows:
        print("FAIL: No visible area capture overlay. Start an area capture first.")
        return 1

    aligned = True
    for hwnd, pid in windows:
        rect = wintypes.RECT()
        origin = wintypes.POINT()
        checked(user32.GetClientRect(hwnd, ctypes.byref(rect)))
        checked(user32.ClientToScreen(hwnd, ctypes.byref(origin)))
        monitor = user32.MonitorFromWindow(hwnd, 2)  # MONITOR_DEFAULTTONEAREST
        info = MonitorInfo(cbSize=ctypes.sizeof(MonitorInfo))
        checked(user32.GetMonitorInfoW(monitor, ctypes.byref(info)))
        actual = (origin.x, origin.y, origin.x + rect.right, origin.y + rect.bottom)
        screen = info.rcMonitor
        expected = (screen.left, screen.top, screen.right, screen.bottom)
        matches = actual == expected
        aligned = aligned and matches
        print(f"{'PASS' if matches else 'FAIL'}: pid={pid} client={actual} monitor={expected}")
    return 0 if aligned else 1


if __name__ == "__main__":
    sys.exit(main())
