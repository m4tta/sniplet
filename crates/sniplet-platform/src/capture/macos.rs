use std::collections::HashSet;

use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::{CFDictionary, CFDictionaryRef},
    number::CFNumber,
    string::CFString,
};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::window::{
    copy_window_info, create_image_from_array, kCGNullWindowID, kCGWindowImageBestResolution,
    kCGWindowImageShouldBeOpaque, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly,
};
use image::RgbaImage;
use sniplet_core::ImageRect;

use crate::{PlatformError, Result};

/// Capture live desktop pixels without the selector panels. The editor remains
/// capturable. Window IDs retain the compositor's front-to-back order.
pub(super) fn capture_region(region: ImageRect, excluded_window_ids: &[u32]) -> Result<RgbaImage> {
    let started = std::time::Instant::now();
    let failure = |message| PlatformError::Capture(Box::new(xcap::XCapError::new(message)));
    if ![region.x, region.y, region.width, region.height]
        .iter()
        .all(|value| value.is_finite())
        || region.width <= 0.0
        || region.height <= 0.0
    {
        return Err(failure("Invalid desktop capture region"));
    }
    let info = copy_window_info(kCGWindowListOptionOnScreenOnly, kCGNullWindowID)
        .ok_or_else(|| failure("macOS did not return window metadata"))?;
    let windows = capture_window_ids(&info, excluded_window_ids);
    // Quartz expects integer window IDs stored as pointer-sized array values,
    // without Core Foundation retain/release callbacks.
    let windows = CFArray::from_copyable(&windows);
    let image = create_image_from_array(
        CGRect::new(
            &CGPoint::new(f64::from(region.x), f64::from(region.y)),
            &CGSize::new(f64::from(region.width), f64::from(region.height)),
        ),
        windows,
        kCGWindowImageBestResolution | kCGWindowImageShouldBeOpaque,
    )
    .ok_or_else(|| {
        failure("macOS could not capture the selected region. Check Screen Recording permission")
    })?;
    if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
        eprintln!(
            "capture backend: live region readback in {:?}",
            started.elapsed()
        );
    }
    if image.bits_per_pixel() != 32 || image.bits_per_component() != 8 {
        return Err(failure(
            "macOS returned an unsupported capture pixel format",
        ));
    }
    let (width, height) = (image.width(), image.height());
    let data = image.data();
    let mut pixels = Vec::with_capacity(width * height * 4);
    for row in data
        .bytes()
        .chunks_exact(image.bytes_per_row())
        .take(height)
    {
        pixels.extend_from_slice(&row[..width * 4]);
    }
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    RgbaImage::from_raw(width as u32, height as u32, pixels)
        .ok_or_else(|| failure("macOS returned incomplete capture pixels"))
}

fn capture_window_ids(info: &CFArray, excluded_window_ids: &[u32]) -> Vec<*const std::ffi::c_void> {
    let id_key = CFString::new("kCGWindowNumber");
    info.iter()
        .filter_map(|entry| {
            // The array contains retained window metadata dictionaries.
            let window = unsafe {
                CFDictionary::<CFString, CFType>::wrap_under_get_rule(*entry as CFDictionaryRef)
            };
            let id = window.find(&id_key)?.downcast::<CFNumber>()?.to_i64()?;
            let id = u32::try_from(id).ok()?;
            (!excluded_window_ids.contains(&id)).then_some(id as usize as *const std::ffi::c_void)
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_capture_excludes_panels_but_keeps_the_editor_and_desktop_stacking_order() {
        // Panels 10 and 14 belong to the same process as editor 12.
        let metadata = [(10, 99), (14, 99), (11, 7), (12, 99), (13, 1)].map(|(id, pid)| {
            CFDictionary::from_CFType_pairs(&[
                (
                    CFString::new("kCGWindowNumber"),
                    CFNumber::from(id).as_CFType(),
                ),
                (
                    CFString::new("kCGWindowOwnerPID"),
                    CFNumber::from(pid).as_CFType(),
                ),
            ])
        });
        let ids = capture_window_ids(&CFArray::from_CFTypes(&metadata).to_untyped(), &[10, 14]);
        assert_eq!(
            ids.into_iter().map(|id| id as usize).collect::<Vec<_>>(),
            [11, 12, 13]
        );
    }
}
