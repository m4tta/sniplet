//! The keyboard shortcuts that the operating system keeps for itself.

/// A system shortcut that is turned on: a macOS virtual key code and Carbon modifier bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemShortcut {
    pub key_code: u16,
    /// Only the four bits below. The system's other bits are dropped.
    pub modifiers: u32,
}

impl SystemShortcut {
    pub const COMMAND: u32 = 0x0100;
    pub const SHIFT: u32 = 0x0200;
    pub const OPTION: u32 = 0x0800;
    pub const CONTROL: u32 = 0x1000;
}

/// Every macOS system shortcut that is turned on, such as the screenshot keys. An app that
/// registers the same keys shares them with the system. Empty on other systems, where a
/// taken shortcut fails to register instead.
pub fn system_shortcuts() -> Vec<SystemShortcut> {
    #[cfg(target_os = "macos")]
    return macos::system_shortcuts();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, c_char, c_void};

    use super::SystemShortcut;

    /// A Core Foundation object.
    type Ref = *const c_void;

    const SINT64: isize = 4; // kCFNumberSInt64Type

    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn CopySymbolicHotKeys(out: *mut Ref) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFNumberGetValue(number: Ref, kind: isize, out: *mut c_void) -> u8;
        fn CFBooleanGetValue(boolean: Ref) -> u8;
        fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
        fn CFRelease(value: Ref);
    }

    pub fn system_shortcuts() -> Vec<SystemShortcut> {
        let mut found = Vec::new();
        // SAFETY: `CopySymbolicHotKeys` returns an array of dictionaries that this function
        // owns and releases, like the three key strings it creates. Each value is checked for
        // null before it is read as the type that the Carbon headers give for its key.
        unsafe {
            let mut keys: Ref = std::ptr::null();
            if CopySymbolicHotKeys(&mut keys) != 0 || keys.is_null() {
                return found;
            }
            // The Carbon headers define each dictionary key as a string of its own name.
            let name =
                |name: &CStr| CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8);
            let code_key = name(c"kHISymbolicHotKeyCode");
            let modifiers_key = name(c"kHISymbolicHotKeyModifiers");
            let enabled_key = name(c"kHISymbolicHotKeyEnabled");
            let number = |entry: Ref, key: Ref| {
                let value = CFDictionaryGetValue(entry, key);
                let mut out = 0i64;
                (!value.is_null()
                    && CFNumberGetValue(value, SINT64, (&raw mut out).cast::<c_void>()) != 0)
                    .then_some(out)
            };
            for index in 0..CFArrayGetCount(keys) {
                let entry = CFArrayGetValueAtIndex(keys, index);
                if entry.is_null() {
                    continue;
                }
                let enabled = CFDictionaryGetValue(entry, enabled_key);
                if enabled.is_null() || CFBooleanGetValue(enabled) == 0 {
                    continue;
                }
                if let Some(code) = number(entry, code_key)
                    && let Some(modifiers) = number(entry, modifiers_key)
                    && let Ok(key_code) = u16::try_from(code)
                {
                    let known = SystemShortcut::COMMAND
                        | SystemShortcut::SHIFT
                        | SystemShortcut::OPTION
                        | SystemShortcut::CONTROL;
                    found.push(SystemShortcut {
                        key_code,
                        modifiers: modifiers as u32 & known,
                    });
                }
            }
            for value in [keys, code_key, modifiers_key, enabled_key] {
                if !value.is_null() {
                    CFRelease(value);
                }
            }
        }
        found
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn macos_lists_system_shortcuts_that_are_on() {
        // Every Mac has some on (Spotlight, Mission Control, screenshots), so an empty list
        // means that the system call or one of its dictionary keys failed.
        assert!(!super::system_shortcuts().is_empty());
    }
}
