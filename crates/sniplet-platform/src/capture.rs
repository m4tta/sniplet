use std::{any::Any, collections::HashMap, panic::AssertUnwindSafe};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use xcap::{Monitor, Window};

use crate::{PlatformError, Result};

#[cfg(target_os = "macos")]
mod macos;

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

/// A monitor's identity, desktop origin, size in capture-image pixels, and scale. Unlike
/// `MonitorInfo` it skips the display name, which is slow on Windows and needs the main thread
/// on macOS, so captures and worker threads read this instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorGeometry {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub process_id: u32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Larger values are nearer the front.
    pub z_order: i32,
    pub is_minimized: bool,
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
/// `list_windows` gives larger z-order values to windows nearer the front on every
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

/// Every monitor with its display name. macOS reads the names through AppKit, so call this
/// only on the main thread there; captures and workers use `list_monitor_geometry`.
pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
    capture_backend("enumerate monitors", list_monitors_impl)
}

fn list_monitors_impl() -> Result<Vec<MonitorInfo>> {
    all_monitors()?
        .iter()
        .enumerate()
        .map(|(index, monitor)| monitor_info(index, monitor))
        .collect()
}

fn monitor_info(index: usize, monitor: &Monitor) -> Result<MonitorInfo> {
    let name = property("monitor", "friendly_name", monitor.friendly_name())
        .or_else(|_| property("monitor", "name", monitor.name()))?;
    let geometry = monitor_geometry(monitor)?;
    Ok(MonitorInfo {
        index,
        id: geometry.id,
        name,
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
        scale_factor: geometry.scale_factor,
        is_primary: property("monitor", "is_primary", monitor.is_primary())?,
    })
}

/// Every monitor's geometry, in enumeration order. Safe on any thread.
pub fn list_monitor_geometry() -> Result<Vec<MonitorGeometry>> {
    capture_backend("enumerate monitors", || {
        all_monitors()?.iter().map(monitor_geometry).collect()
    })
}

/// The geometry of the monitor with `id`.
pub(crate) fn geometry_of(id: u32) -> Result<MonitorGeometry> {
    capture_backend("find a monitor", || {
        find_monitor(id).map(|(_, _, geometry)| geometry)
    })
}

fn monitor_geometry(monitor: &Monitor) -> Result<MonitorGeometry> {
    let scale_factor = property("monitor", "scale_factor", monitor.scale_factor())?;
    let native_width = property("monitor", "width", monitor.width())?;
    let native_height = property("monitor", "height", monitor.height())?;
    Ok(MonitorGeometry {
        id: property("monitor", "id", monitor.id())?,
        x: property("monitor", "x", monitor.x())?,
        y: property("monitor", "y", monitor.y())?,
        width: capture_pixels(native_width, scale_factor),
        height: capture_pixels(native_height, scale_factor),
        scale_factor,
    })
}

fn all_monitors() -> Result<Vec<Monitor>> {
    Monitor::all().map_err(|source| PlatformError::Enumeration {
        kind: "monitors",
        source: Box::new(source),
    })
}

/// The monitor with `id`, its place in the enumeration, and its geometry.
fn find_monitor(id: u32) -> Result<(usize, Monitor, MonitorGeometry)> {
    let (index, monitor) = all_monitors()?
        .into_iter()
        .enumerate()
        .find(|(_, monitor)| monitor.id().is_ok_and(|candidate| candidate == id))
        .ok_or_else(|| {
            PlatformError::Capture(Box::new(xcap::XCapError::new(
                "The display was disconnected",
            )))
        })?;
    let geometry = monitor_geometry(&monitor)?;
    Ok((index, monitor, geometry))
}

pub fn capture_monitor(index: usize) -> Result<CapturedFrame> {
    capture_backend("capture a monitor", || capture_monitor_impl(index))
}

/// Capture the display under the current desktop pointer, without using an
/// editor's cached display index. Native hit testing handles display placement.
pub fn capture_monitor_under_pointer() -> Result<CapturedFrame> {
    capture_backend("capture the monitor under the pointer", || {
        let monitors = all_monitors()?;
        let point = pointer_position()?;
        // X11 returns physical pointer coordinates. xcap's Linux point lookup
        // expects desktop units and applies its global display scale itself.
        #[cfg(target_os = "linux")]
        let point = {
            let scale = monitors
                .first()
                .map_or(Ok(1.0), Monitor::scale_factor)
                .map_err(|source| PlatformError::Capture(Box::new(source)))?
                .max(1.0);
            ScreenPoint {
                x: (point.x as f32 / scale).floor() as i32,
                y: (point.y as f32 / scale).floor() as i32,
            }
        };
        let selected = Monitor::from_point(point.x, point.y)
            .map_err(|source| PlatformError::Capture(Box::new(source)))?;
        let id = property("monitor", "id", selected.id())?;
        let index = monitors
            .iter()
            .position(|monitor| monitor.id().is_ok_and(|candidate| candidate == id))
            .ok_or_else(|| {
                PlatformError::Capture(Box::new(xcap::XCapError::new(
                    "The display under the pointer was disconnected",
                )))
            })?;
        if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
            eprintln!("capture backend: pointer={point:?}; display id={id}; index={index}");
        }
        let geometry = monitor_geometry(&selected)?;
        capture_monitor_frame(index, &selected, geometry)
    })
}

#[cfg(target_os = "macos")]
fn pointer_position() -> Result<ScreenPoint> {
    use core_graphics::{
        event::CGEvent,
        event_source::{CGEventSource, CGEventSourceStateID},
    };

    let event = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .and_then(CGEvent::new)
        .map_err(|()| {
            PlatformError::PointerPositionUnavailable(
                "macOS did not return the mouse position".into(),
            )
        })?;
    let point = event.location();
    Ok(ScreenPoint {
        x: point.x.floor() as i32,
        y: point.y.floor() as i32,
    })
}

#[cfg(target_os = "windows")]
fn pointer_position() -> Result<ScreenPoint> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

    let mut point = POINT::default();
    // Reading the desktop cursor does not send input or move it.
    unsafe { GetCursorPos(&mut point) }
        .map_err(|error| PlatformError::PointerPositionUnavailable(error.to_string()))?;
    Ok(ScreenPoint {
        x: point.x,
        y: point.y,
    })
}

#[cfg(target_os = "linux")]
fn pointer_position() -> Result<ScreenPoint> {
    use enigo::Mouse;

    let input = enigo::Enigo::new(&enigo::Settings::default())
        .map_err(|error| PlatformError::PointerPositionUnavailable(error.to_string()))?;
    let (x, y) = input
        .location()
        .map_err(|error| PlatformError::PointerPositionUnavailable(error.to_string()))?;
    Ok(ScreenPoint { x, y })
}

/// Captures every display before selection windows are shown, one thread per display.
/// Enumerating once keeps the frame indices and display identities consistent within a
/// session. Windows monitor handles cannot cross threads, so each thread finds its display
/// again by ID.
pub fn capture_monitors() -> Result<Vec<CapturedFrame>> {
    capture_backend("capture all monitors", || {
        let ids = all_monitors()?
            .iter()
            .map(|monitor| property("monitor", "id", monitor.id()))
            .collect::<Result<Vec<_>>>()?;
        std::thread::scope(|scope| {
            let threads = ids
                .into_iter()
                .enumerate()
                .map(|(index, id)| {
                    scope.spawn(move || {
                        capture_backend("capture a monitor", || {
                            let (_, monitor, geometry) = find_monitor(id)?;
                            capture_monitor_frame(index, &monitor, geometry)
                        })
                    })
                })
                .collect::<Vec<_>>();
            threads
                .into_iter()
                .map(|thread| {
                    thread
                        .join()
                        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
                })
                .collect()
        })
    })
}

/// Whether Sniplet has the Screen Recording permission. This never shows a prompt.
#[cfg(target_os = "macos")]
pub fn screen_recording_allowed() -> bool {
    core_graphics::access::ScreenCaptureAccess.preflight()
}

/// Check for the Screen Recording permission that captures need, ask macOS for it when it is
/// missing, and return whether Sniplet has it. macOS shows its prompt one time per app and
/// does not wait for the answer, so a later start that is still not allowed gets no prompt.
#[cfg(target_os = "macos")]
pub fn request_screen_recording() -> bool {
    let access = core_graphics::access::ScreenCaptureAccess;
    access.preflight() || access.request()
}

/// Capture a live rectangle in Quartz desktop points, excluding selector windows.
#[cfg(target_os = "macos")]
pub fn capture_desktop_region(
    region: sniplet_core::ImageRect,
    excluded_window_ids: &[u32],
) -> Result<RgbaImage> {
    capture_backend("capture a desktop region", || {
        macos::capture_region(region, excluded_window_ids)
    })
}

fn capture_monitor_impl(index: usize) -> Result<CapturedFrame> {
    let started = std::time::Instant::now();
    let trace = std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some();
    let monitors = all_monitors()?;
    let available = monitors.len();
    let monitor = monitors
        .into_iter()
        .nth(index)
        .ok_or(PlatformError::MonitorNotFound { index, available })?;
    let geometry = monitor_geometry(&monitor)?;
    if trace {
        eprintln!("capture backend: display lookup {:?}", started.elapsed());
    }
    capture_monitor_frame(index, &monitor, geometry)
}

fn capture_monitor_frame(
    index: usize,
    monitor: &Monitor,
    geometry: MonitorGeometry,
) -> Result<CapturedFrame> {
    let started = std::time::Instant::now();
    let image = monitor
        .capture_image()
        .map_err(|source| PlatformError::Capture(Box::new(source)))?;
    if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
        eprintln!("capture backend: image {:?}", started.elapsed());
    }
    Ok(CapturedFrame {
        image,
        origin: ScreenPoint {
            x: geometry.x,
            y: geometry.y,
        },
        scale_factor: geometry.scale_factor,
        source: CaptureSource::Monitor {
            index,
            id: geometry.id,
        },
    })
}

/// Captures a rectangle of the monitor with ID `monitor_id`, in image pixels relative to the
/// monitor capture's top-left corner. This stays pixel-accurate on Retina displays even
/// though macOS screen APIs express desktop coordinates in points.
pub fn capture_monitor_region(
    monitor_id: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<CapturedFrame> {
    capture_backend("capture a monitor region", || {
        capture_monitor_region_impl(monitor_id, x, y, width, height)
    })
}

/// Whether this is a Wayland session, as xcap decides it.
fn wayland_session() -> bool {
    std::env::var_os("XDG_SESSION_TYPE").is_some_and(|kind| kind.to_str() == Some("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY")
            .is_some_and(|display| display.to_string_lossy().to_lowercase().contains("wayland"))
}

fn capture_monitor_region_impl(
    monitor_id: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<CapturedFrame> {
    let (index, monitor, info) = find_monitor(monitor_id)?;
    // The region must lie inside a monitor of this size, in image pixels.
    let inside = |monitor_width: u32, monitor_height: u32| {
        if width > 0
            && height > 0
            && x.checked_add(width)
                .is_some_and(|right| right <= monitor_width)
            && y.checked_add(height)
                .is_some_and(|bottom| bottom <= monitor_height)
        {
            Ok(())
        } else {
            Err(PlatformError::InvalidCaptureRegion {
                x,
                y,
                width,
                height,
                monitor_width,
                monitor_height,
            })
        }
    };
    let native_x = desktop_units(x, info.scale_factor);
    let native_y = desktop_units(y, info.scale_factor);
    // A Retina point rectangle rounds outward, so the image can start before the region.
    let skip_x = x.saturating_sub(capture_pixels(native_x, info.scale_factor));
    let skip_y = y.saturating_sub(capture_pixels(native_y, info.scale_factor));
    // xcap gives the Wayland screenshot monitor-relative coordinates where it expects desktop
    // ones, and on a scaled Linux desktop it checks a region against logical sizes. Both
    // capture the whole monitor and crop it.
    let image = if cfg!(target_os = "linux") && (wayland_session() || info.scale_factor != 1.0) {
        let full = monitor
            .capture_image()
            .map_err(|source| PlatformError::Capture(Box::new(source)))?;
        inside(full.width(), full.height())?;
        image::imageops::crop_imm(&full, x, y, width, height).to_image()
    } else {
        inside(info.width, info.height)?;
        let image = monitor
            .capture_region(
                native_x,
                native_y,
                desktop_units_ceil(skip_x + width, info.scale_factor),
                desktop_units_ceil(skip_y + height, info.scale_factor),
            )
            .map_err(|source| PlatformError::Capture(Box::new(source)))?;
        if skip_x == 0 && skip_y == 0 && image.dimensions() == (width, height) {
            image
        } else {
            image::imageops::crop_imm(
                &image,
                skip_x.min(image.width()),
                skip_y.min(image.height()),
                width.min(image.width().saturating_sub(skip_x)),
                height.min(image.height().saturating_sub(skip_y)),
            )
            .to_image()
        }
    };
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

fn all_windows() -> Result<Vec<Window>> {
    Window::all().map_err(|source| PlatformError::Enumeration {
        kind: "windows",
        source: Box::new(source),
    })
}

fn list_windows_impl() -> Result<Vec<WindowInfo>> {
    let windows = all_windows()?;
    let count = windows.len();
    // Windows can disappear between enumeration and property reads. Omitting a
    // stale entry keeps a single closing tooltip from breaking the picker. xcap lists
    // windows front to back on every desktop, so the position is the z-order; its own
    // `z()` lists every window again for each one.
    let windows = windows
        .iter()
        .enumerate()
        .filter_map(move |(index, window)| window_info(window, (count - index) as i32).ok());
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
    #[cfg(target_os = "macos")]
    let windows = {
        let ids = macos::application_window_ids()?;
        windows.filter(move |window| ids.contains(&window.id))
    };
    Ok(windows.collect())
}

fn window_info(window: &Window, z_order: i32) -> Result<WindowInfo> {
    Ok(WindowInfo {
        id: property("window", "id", window.id())?,
        process_id: property("window", "pid", window.pid())?,
        title: property("window", "title", window.title())?,
        x: property("window", "x", window.x())?,
        y: property("window", "y", window.y())?,
        width: property("window", "width", window.width())?,
        height: property("window", "height", window.height())?,
        z_order,
        is_minimized: property("window", "is_minimized", window.is_minimized())?,
        is_focused: property("window", "is_focused", window.is_focused())?,
    })
}

/// Each window's app name, by window ID. Windows reads the name from the app's file, so the
/// window picker asks for it after it opens instead of while it lists windows.
pub fn window_app_names() -> Result<HashMap<u32, String>> {
    capture_backend("read window app names", || {
        let mut by_process = HashMap::<u32, String>::new();
        let mut names = HashMap::new();
        for window in all_windows()? {
            let (Ok(id), Ok(process_id)) = (window.id(), window.pid()) else {
                continue;
            };
            let name = by_process
                .entry(process_id)
                .or_insert_with(|| window.app_name().unwrap_or_default());
            names.insert(id, name.clone());
        }
        Ok(names)
    })
}

pub fn capture_window(id: u32) -> Result<CapturedFrame> {
    capture_backend("capture a window", || capture_window_impl(id))
}

/// Window-only presentation; raw captures remain available for scrolling and
/// monitor/area captures. A missing wallpaper falls back to the chosen solid
/// color and returns a message for the editor rather than losing the capture.
pub fn capture_window_with_background(
    id: u32,
    style: &crate::WindowCaptureStyle,
) -> Result<(CapturedFrame, Option<String>)> {
    let mut frame = capture_window(id)?;
    let (wallpaper, warning) = if style.background == crate::WindowBackground::Wallpaper {
        match crate::window_background::load_wallpaper(style) {
            Ok(image) => (Some(image), None),
            Err(error) => (
                None,
                Some(format!(
                    "Window captured · solid background used. {error}. Choose a wallpaper image in Settings."
                )),
            ),
        }
    } else {
        (None, None)
    };
    frame.image =
        crate::compose_window_capture(frame.image, style, frame.scale_factor, wallpaper.as_ref());
    if style.background != crate::WindowBackground::TrimShadow {
        let scale = if frame.scale_factor.is_finite() {
            frame.scale_factor.clamp(1.0, 8.0)
        } else {
            1.0
        };
        let padding = desktop_units(
            (style.padding.min(120) as f32 * scale).round() as u32,
            scale,
        ) as i32;
        frame.origin.x = frame.origin.x.saturating_sub(padding);
        frame.origin.y = frame.origin.y.saturating_sub(padding);
    }
    Ok((frame, warning))
}

fn capture_window_impl(id: u32) -> Result<CapturedFrame> {
    let window = all_windows()?
        .into_iter()
        .find(|window| window.id().is_ok_and(|window_id| window_id == id))
        .ok_or(PlatformError::WindowNotFound { id })?;
    capture_window_handle(window)
}

fn capture_window_handle(window: Window) -> Result<CapturedFrame> {
    // A single capture has no use for the z-order.
    let info = window_info(&window, 0)?;
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
            title: format!("Window {id}"),
            x,
            y,
            width: 300,
            height: 200,
            z_order,
            is_minimized: false,
            is_focused: false,
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_window_picker_excludes_full_display_dock_window() {
        use core_foundation::{
            array::CFArray, base::TCFType, dictionary::CFDictionary, number::CFNumber,
            string::CFString,
        };

        let metadata = [(1, 20), (2, 0)].map(|(id, layer)| {
            CFDictionary::from_CFType_pairs(&[
                (
                    CFString::new("kCGWindowNumber"),
                    CFNumber::from(id).as_CFType(),
                ),
                (
                    CFString::new("kCGWindowLayer"),
                    CFNumber::from(layer).as_CFType(),
                ),
            ])
        });
        let ids =
            macos::application_window_ids_from_info(&CFArray::from_CFTypes(&metadata).to_untyped());
        let mut dock = window(1, 10, 0, 0, 20);
        dock.width = 2056;
        dock.height = 1329;
        let preview = window(2, 20, 4, 43, 11);
        let windows: Vec<_> = [dock, preview]
            .into_iter()
            .filter(|window| ids.contains(&window.id))
            .collect();

        let selected = window_at_point(&windows, ScreenPoint { x: 100, y: 100 }, 0).unwrap();
        assert_eq!(selected.id, 2, "the Preview window, not the Dock");
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
