use std::{any::Any, panic::AssertUnwindSafe};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use xcap::{Monitor, Window};

use crate::{PlatformError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub index: usize,
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub is_primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub process_id: u32,
    pub app_name: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z_order: i32,
    pub is_minimized: bool,
    pub is_maximized: bool,
    pub is_focused: bool,
}

impl WindowInfo {
    pub fn is_capture_candidate(&self, excluded_process_id: u32) -> bool {
        self.process_id != excluded_process_id
            && !self.is_minimized
            && !self.title.trim().is_empty()
            && self.width > 0
            && self.height > 0
    }

    pub fn contains(&self, point: ScreenPoint) -> bool {
        let left = i64::from(self.x);
        let top = i64::from(self.y);
        let right = left + i64::from(self.width);
        let bottom = top + i64::from(self.height);
        let x = i64::from(point.x);
        let y = i64::from(point.y);
        x >= left && x < right && y >= top && y < bottom
    }
}

/// Returns the frontmost capturable application window at a desktop point.
///
/// `xcap` reports larger z-order values for windows nearer the front on every
/// supported desktop backend. Keeping this policy beside window enumeration
/// makes hover selection and any future native picker agree.
pub fn window_at_point(
    windows: &[WindowInfo],
    point: ScreenPoint,
    excluded_process_id: u32,
) -> Option<&WindowInfo> {
    windows
        .iter()
        .filter(|window| window.is_capture_candidate(excluded_process_id) && window.contains(point))
        .max_by_key(|window| window.z_order)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureSource {
    Monitor { index: usize, id: u32 },
    MonitorRegion { index: usize, id: u32 },
    Window { id: u32 },
}

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub image: RgbaImage,
    /// Desktop-space position of the captured image's top-left corner.
    pub origin: ScreenPoint,
    pub scale_factor: f32,
    pub source: CaptureSource,
}

impl CapturedFrame {
    pub fn into_document(self) -> sniplet_core::Document {
        sniplet_core::Document::new(self.image)
    }
}

fn property<T>(
    kind: &'static str,
    property: &'static str,
    value: std::result::Result<T, xcap::XCapError>,
) -> Result<T> {
    value.map_err(|source| PlatformError::CaptureProperty {
        kind,
        property,
        source: Box::new(source),
    })
}

pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
    capture_backend("enumerate monitors", list_monitors_impl)
}

fn list_monitors_impl() -> Result<Vec<MonitorInfo>> {
    let monitors = Monitor::all().map_err(|source| PlatformError::Enumeration {
        kind: "monitors",
        source: Box::new(source),
    })?;

    monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| monitor_info(index, monitor))
        .collect()
}

fn monitor_info(index: usize, monitor: &Monitor) -> Result<MonitorInfo> {
    let name = property("monitor", "friendly_name", monitor.friendly_name())
        .or_else(|_| property("monitor", "name", monitor.name()))?;
    let scale_factor = property("monitor", "scale_factor", monitor.scale_factor())?;
    let native_width = property("monitor", "width", monitor.width())?;
    let native_height = property("monitor", "height", monitor.height())?;
    Ok(MonitorInfo {
        index,
        id: property("monitor", "id", monitor.id())?,
        name,
        x: property("monitor", "x", monitor.x())?,
        y: property("monitor", "y", monitor.y())?,
        width: capture_pixels(native_width, scale_factor),
        height: capture_pixels(native_height, scale_factor),
        scale_factor,
        is_primary: property("monitor", "is_primary", monitor.is_primary())?,
    })
}

pub fn capture_monitor(index: usize) -> Result<CapturedFrame> {
    capture_backend("capture a monitor", || capture_monitor_impl(index))
}

fn capture_monitor_impl(index: usize) -> Result<CapturedFrame> {
    let monitors = Monitor::all().map_err(|source| PlatformError::Enumeration {
        kind: "monitors",
        source: Box::new(source),
    })?;
    let available = monitors.len();
    let monitor = monitors
        .into_iter()
        .nth(index)
        .ok_or(PlatformError::MonitorNotFound { index, available })?;
    let info = monitor_info(index, &monitor)?;
    let image = monitor
        .capture_image()
        .map_err(|source| PlatformError::Capture(Box::new(source)))?;
    Ok(CapturedFrame {
        image,
        origin: ScreenPoint {
            x: info.x,
            y: info.y,
        },
        scale_factor: info.scale_factor,
        source: CaptureSource::Monitor { index, id: info.id },
    })
}

/// Captures a rectangle using image pixels relative to the monitor capture's
/// top-left corner. This stays pixel-accurate on Retina displays even though
/// macOS screen APIs express desktop coordinates in points.
pub fn capture_monitor_region(
    index: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<CapturedFrame> {
    capture_backend("capture a monitor region", || {
        capture_monitor_region_impl(index, x, y, width, height)
    })
}

fn capture_monitor_region_impl(
    index: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<CapturedFrame> {
    let monitors = Monitor::all().map_err(|source| PlatformError::Enumeration {
        kind: "monitors",
        source: Box::new(source),
    })?;
    let available = monitors.len();
    let monitor = monitors
        .into_iter()
        .nth(index)
        .ok_or(PlatformError::MonitorNotFound { index, available })?;
    let info = monitor_info(index, &monitor)?;
    if width == 0
        || height == 0
        || x.checked_add(width).is_none_or(|right| right > info.width)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > info.height)
    {
        return Err(PlatformError::InvalidCaptureRegion {
            x,
            y,
            width,
            height,
            monitor_width: info.width,
            monitor_height: info.height,
        });
    }
    let native_x = desktop_units(x, info.scale_factor);
    let native_y = desktop_units(y, info.scale_factor);
    let native_width = desktop_units_ceil(width, info.scale_factor);
    let native_height = desktop_units_ceil(height, info.scale_factor);
    let mut image = monitor
        .capture_region(native_x, native_y, native_width, native_height)
        .map_err(|source| PlatformError::Capture(Box::new(source)))?;
    // Rounding a Retina point rectangle outward can produce an extra pixel.
    if image.width() > width || image.height() > height {
        image = image::imageops::crop_imm(
            &image,
            0,
            0,
            width.min(image.width()),
            height.min(image.height()),
        )
        .to_image();
    }
    Ok(CapturedFrame {
        image,
        origin: ScreenPoint {
            x: info.x.saturating_add(native_x as i32),
            y: info.y.saturating_add(native_y as i32),
        },
        scale_factor: info.scale_factor,
        source: CaptureSource::MonitorRegion { index, id: info.id },
    })
}

#[cfg(target_os = "macos")]
fn capture_pixels(desktop_units: u32, scale_factor: f32) -> u32 {
    ((desktop_units as f32) * scale_factor).round() as u32
}

#[cfg(not(target_os = "macos"))]
fn capture_pixels(desktop_units: u32, _scale_factor: f32) -> u32 {
    desktop_units
}

#[cfg(target_os = "macos")]
pub(crate) fn desktop_units(image_pixels: u32, scale_factor: f32) -> u32 {
    ((image_pixels as f32) / scale_factor.max(1.0)).floor() as u32
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn desktop_units(image_pixels: u32, _scale_factor: f32) -> u32 {
    image_pixels
}

#[cfg(target_os = "macos")]
fn desktop_units_ceil(image_pixels: u32, scale_factor: f32) -> u32 {
    ((image_pixels as f32) / scale_factor.max(1.0)).ceil() as u32
}

#[cfg(not(target_os = "macos"))]
fn desktop_units_ceil(image_pixels: u32, _scale_factor: f32) -> u32 {
    image_pixels
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    capture_backend("enumerate windows", list_windows_impl)
}

fn list_windows_impl() -> Result<Vec<WindowInfo>> {
    let windows = Window::all().map_err(|source| PlatformError::Enumeration {
        kind: "windows",
        source: Box::new(source),
    })?;
    // Windows can disappear between enumeration and property reads. Omitting a
    // stale entry keeps a single closing tooltip from breaking the picker.
    let windows = windows.iter().filter_map(|window| window_info(window).ok());
    // Tool windows include notification toasts and floating helper overlays,
    // rather than the app windows offered by the capture picker.
    #[cfg(windows)]
    let windows = windows.filter(|window| {
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, WS_EX_TOOLWINDOW},
        };
        let handle = HWND(window.id as usize as *mut std::ffi::c_void);
        // Reading the style does not retain or change the enumerated window.
        let style = unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32;
        style & WS_EX_TOOLWINDOW.0 == 0
    });
    Ok(windows.collect())
}

fn window_info(window: &Window) -> Result<WindowInfo> {
    Ok(WindowInfo {
        id: property("window", "id", window.id())?,
        process_id: property("window", "pid", window.pid())?,
        app_name: property("window", "app_name", window.app_name())?,
        title: property("window", "title", window.title())?,
        x: property("window", "x", window.x())?,
        y: property("window", "y", window.y())?,
        width: property("window", "width", window.width())?,
        height: property("window", "height", window.height())?,
        z_order: property("window", "z", window.z())?,
        is_minimized: property("window", "is_minimized", window.is_minimized())?,
        is_maximized: property("window", "is_maximized", window.is_maximized())?,
        is_focused: property("window", "is_focused", window.is_focused())?,
    })
}

pub fn capture_window(id: u32) -> Result<CapturedFrame> {
    capture_backend("capture a window", || capture_window_impl(id))
}

fn capture_window_impl(id: u32) -> Result<CapturedFrame> {
    let windows = Window::all().map_err(|source| PlatformError::Enumeration {
        kind: "windows",
        source: Box::new(source),
    })?;
    let mut selected = None;
    for window in windows {
        if window.id().is_ok_and(|window_id| window_id == id) {
            selected = Some(window);
            break;
        }
    }
    let window = selected.ok_or(PlatformError::WindowNotFound { id })?;
    capture_window_handle(window)
}

pub fn capture_active_window() -> Result<CapturedFrame> {
    capture_backend("capture the active window", capture_active_window_impl)
}

fn capture_active_window_impl() -> Result<CapturedFrame> {
    let windows = Window::all().map_err(|source| PlatformError::Enumeration {
        kind: "windows",
        source: Box::new(source),
    })?;
    for window in windows {
        if window.is_focused().unwrap_or(false) {
            return capture_window_handle(window);
        }
    }
    Err(PlatformError::ActiveWindowNotFound)
}

fn capture_window_handle(window: Window) -> Result<CapturedFrame> {
    let info = window_info(&window)?;
    if info.is_minimized {
        return Err(PlatformError::WindowMinimized { id: info.id });
    }
    let scale_factor = window
        .current_monitor()
        .and_then(|monitor| monitor.scale_factor())
        .unwrap_or(1.0);
    let image = window
        .capture_image()
        .map_err(|source| PlatformError::Capture(Box::new(source)))?;
    Ok(CapturedFrame {
        image,
        origin: ScreenPoint {
            x: info.x,
            y: info.y,
        },
        scale_factor,
        source: CaptureSource::Window { id: info.id },
    })
}

fn capture_backend<T>(operation: &'static str, action: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(AssertUnwindSafe(action)).map_err(|payload| {
        PlatformError::CaptureBackendUnavailable {
            operation,
            reason: panic_reason(payload.as_ref()),
        }
    })?
}

fn panic_reason(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "the native backend stopped unexpectedly".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_backend_panics_become_regular_errors() {
        let result = capture_backend("test capture", || -> Result<()> {
            panic!("unsupported compositor protocol")
        });

        let error = result.unwrap_err();
        assert!(matches!(
            &error,
            PlatformError::CaptureBackendUnavailable {
                operation: "test capture",
                ..
            }
        ));
        assert!(
            error
                .to_string()
                .contains("unsupported compositor protocol")
        );
    }

    fn window(id: u32, process_id: u32, x: i32, y: i32, z_order: i32) -> WindowInfo {
        WindowInfo {
            id,
            process_id,
            app_name: format!("App {id}"),
            title: format!("Window {id}"),
            x,
            y,
            width: 300,
            height: 200,
            z_order,
            is_minimized: false,
            is_maximized: false,
            is_focused: false,
        }
    }

    #[test]
    fn hover_hit_test_chooses_the_frontmost_eligible_window() {
        let mut back = window(1, 10, 0, 0, 4);
        let front = window(2, 20, 50, 40, 9);
        let own = window(3, 30, 60, 50, 20);
        let mut minimized = window(4, 40, 60, 50, 30);
        minimized.is_minimized = true;
        back.is_focused = true;

        let windows = [back, front, own, minimized];
        let selected = window_at_point(&windows, ScreenPoint { x: 100, y: 100 }, 30).unwrap();

        assert_eq!(selected.id, 2);
    }

    #[test]
    fn hover_hit_test_handles_negative_desktop_coordinates_and_edges() {
        let window = window(7, 70, -500, -250, 1);
        let windows = [window];

        assert_eq!(
            window_at_point(&windows, ScreenPoint { x: -500, y: -250 }, 0).map(|w| w.id),
            Some(7)
        );
        assert_eq!(
            window_at_point(&windows, ScreenPoint { x: -201, y: -51 }, 0).map(|w| w.id),
            Some(7)
        );
        assert!(window_at_point(&windows, ScreenPoint { x: -200, y: -50 }, 0).is_none());
    }
}
