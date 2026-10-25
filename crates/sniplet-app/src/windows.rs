use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        IsIconic, IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOW, ShowWindow,
    },
};

fn native_window(window: &Window) -> Option<HWND> {
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    Some(HWND(handle.hwnd.get() as *mut _))
}

pub fn is_minimized(window: &Window) -> bool {
    native_window(window).is_some_and(|hwnd| unsafe { IsIconic(hwnd).as_bool() })
}

/// Hide the editor and its taskbar button. GPUI retains the window and document.
pub fn hide_editor(window: &Window) -> bool {
    let Some(hwnd) = native_window(window) else {
        return false;
    };
    // This handle belongs to the live GPUI window on its UI thread.
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    true
}

/// GPUI restores minimized windows, but does not show a hidden window itself.
pub fn show_editor(window: &Window) {
    let Some(hwnd) = native_window(window) else {
        return;
    };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            // Keep a maximized editor maximized when it was hidden directly.
            let show = if IsIconic(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            let _ = ShowWindow(hwnd, show);
        }
    }
}
