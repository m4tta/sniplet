use std::collections::HashSet;

use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::{CFDictionary, CFDictionaryRef},
    number::CFNumber,
    string::CFString,
};
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly,
};

use crate::{PlatformError, Result};

pub(super) fn application_window_ids() -> Result<HashSet<u32>> {
    let info = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    )
    .ok_or_else(|| PlatformError::Enumeration {
        kind: "windows",
        source: Box::new(xcap::XCapError::new("macOS did not return window metadata")),
    })?;
    Ok(application_window_ids_from_info(&info))
}

pub(super) fn application_window_ids_from_info(info: &CFArray) -> HashSet<u32> {
    let id_key = CFString::new("kCGWindowNumber");
    let layer_key = CFString::new("kCGWindowLayer");
    info.iter()
        .filter_map(|entry| {
            // CGWindowListCopyWindowInfo returns an array of window dictionaries.
            let window = unsafe {
                CFDictionary::<CFString, CFType>::wrap_under_get_rule(*entry as CFDictionaryRef)
            };
            // Normal app windows use layer 0. The Dock has an invisible layer-20
            // window over the whole display, above the app windows in z-order.
            let layer = window.find(&layer_key)?.downcast::<CFNumber>()?.to_i64()?;
            if layer != 0 {
                return None;
            }
            let id = window.find(&id_key)?.downcast::<CFNumber>()?.to_i64()?;
            u32::try_from(id).ok()
        })
        .collect()
}
