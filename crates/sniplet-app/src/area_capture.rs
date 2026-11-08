//! The area selection: one overlay per monitor, sharing one selection in desktop units, so a
//! drag can start on one monitor and end on another. Windows and Linux show a frozen image
//! of each monitor; macOS selects over the live desktop and captures after the release.
use std::{cell::RefCell, rc::Rc};

use image::{Rgba, RgbaImage};
use rune::{Cursor, Cx, Drag, El, Key, KeyEvent, Phase, Pixels, Tx, WindowHandle, el, text};
use sniplet_core::{Document, ImageRect, Point as ImagePoint};

use crate::{
    capture,
    capture_overlay::{self, Monitor, Rect, WindowState},
    editor::{Command, Editor},
    shell::{self, EscapeSession},
    ui::{BODY, Inbox},
};

/// Desktop coordinates and display geometry are retained for Repeat Capture.
#[derive(Clone, Debug)]
pub(crate) struct Region {
    bounds: Rect,
    displays: Vec<Monitor>,
}

#[derive(Clone)]
struct PixelsInRegion {
    image: RgbaImage,
    bounds: Rect,
}

#[derive(Clone)]
struct Screen {
    /// The monitor's pixels: a frozen frame, or on macOS the selected part once captured.
    frame: Option<PixelsInRegion>,
    layout: Monitor,
    scale_factor: f32,
}

impl Screen {
    fn new(frame: sniplet_platform::CapturedFrame) -> Self {
        let layout = Monitor::from_frame(&frame);
        Self {
            frame: Some(PixelsInRegion {
                image: frame.image,
                bounds: layout.bounds,
            }),
            layout,
            scale_factor: frame.scale_factor,
        }
    }

    #[cfg(target_os = "macos")]
    fn live(info: &sniplet_platform::MonitorInfo) -> Self {
        Self {
            frame: None,
            layout: Monitor::from_info(info),
            scale_factor: info.scale_factor,
        }
    }

    fn scale(&self) -> f32 {
        self.layout.scale()
    }
}

fn selection_scale(screens: &[Screen], bounds: Rect) -> f32 {
    screens
        .iter()
        .filter(|screen| screen.layout.bounds.overlaps(&bounds))
        .map(Screen::scale)
        .fold(1.0, f32::max)
}

fn region(screens: &[Screen], bounds: Rect) -> Region {
    Region {
        bounds,
        displays: screens.iter().map(|screen| screen.layout.clone()).collect(),
    }
}

fn layout_matches(region: &Region, screens: &[Screen]) -> bool {
    region.displays.len() == screens.len()
        && screens
            .iter()
            .all(|screen| region.displays.contains(&screen.layout))
}

/// Compose in desktop units at the highest selected display scale. Empty desktop space stays
/// black; each display samples its own pixels.
fn compose(screens: &[Screen], bounds: Rect) -> Option<(RgbaImage, f32)> {
    let desktop = screens
        .iter()
        .map(|screen| screen.layout.bounds)
        .reduce(|a, b| a.union(&b))?;
    let bounds = bounds.intersect(&desktop);
    if !screens
        .iter()
        .any(|screen| screen.layout.bounds.overlaps(&bounds))
    {
        return None;
    }
    let scale = selection_scale(screens, bounds);
    let left = (bounds.x * scale).floor();
    let top = (bounds.y * scale).floor();
    let width = ((bounds.right() * scale).ceil() - left) as u32;
    let height = ((bounds.bottom() * scale).ceil() - top) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut image = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
    for screen in screens {
        if !screen.layout.bounds.overlaps(&bounds) {
            continue;
        }
        let frame = screen.frame.as_ref()?;
        let display = frame.bounds;
        let clamp = |value: f32, max: u32| value.round().clamp(0.0, max as f32) as u32;
        let x0 = clamp(display.x * scale - left, width);
        let y0 = clamp(display.y * scale - top, height);
        let x1 = clamp(display.right() * scale - left, width);
        let y1 = clamp(display.bottom() * scale - top, height);
        let sx = frame.image.width() as f32 / display.w;
        let sy = frame.image.height() as f32 / display.h;
        for y in y0..y1 {
            let source_y = (((top + y as f32 + 0.5) / scale - display.y) * sy)
                .floor()
                .clamp(0.0, (frame.image.height() - 1) as f32) as u32;
            for x in x0..x1 {
                let source_x = (((left + x as f32 + 0.5) / scale - display.x) * sx)
                    .floor()
                    .clamp(0.0, (frame.image.width() - 1) as f32)
                    as u32;
                image.put_pixel(x, y, *frame.image.get_pixel(source_x, source_y));
            }
        }
    }
    Some((image, scale))
}

/// Round outward to each display's pixel grid. Keeping its desktop origin preserves Retina
/// pixels and selections spanning displays with different scales.
#[cfg(target_os = "macos")]
fn capture_bounds(screen: &Screen, selection: Rect) -> Rect {
    let bounds = selection.intersect(&screen.layout.bounds);
    let scale = screen.scale();
    let (left, top) = (screen.layout.bounds.x, screen.layout.bounds.y);
    let x0 = ((bounds.x - left) * scale).floor() / scale + left;
    let y0 = ((bounds.y - top) * scale).floor() / scale + top;
    let x1 = ((bounds.right() - left) * scale).ceil() / scale + left;
    let y1 = ((bounds.bottom() - top) * scale).ceil() / scale + top;
    Rect::new(x0, y0, x1 - x0, y1 - y0).intersect(&screen.layout.bounds)
}

/// The selected pixels. macOS reads them from the live desktop now, leaving out the
/// selection windows in `excluded`.
fn capture_selected(
    screens: Vec<Screen>,
    bounds: Rect,
    #[cfg(target_os = "macos")] monitors: &[sniplet_platform::MonitorInfo],
    #[cfg(target_os = "macos")] excluded: &[u32],
) -> Result<(RgbaImage, f32), String> {
    #[cfg(target_os = "macos")]
    let mut screens = screens;
    #[cfg(target_os = "macos")]
    for screen in &mut screens {
        if screen.frame.is_some() || !screen.layout.bounds.overlaps(&bounds) {
            continue;
        }
        if !monitors
            .iter()
            .any(|info| Monitor::from_info(info) == screen.layout)
        {
            return Err("Capture stopped because the display layout changed".into());
        }
        let region = capture_bounds(screen, bounds);
        let image = sniplet_platform::capture_desktop_region(
            ImageRect::new(region.x, region.y, region.w, region.h),
            excluded,
        )
        .map_err(|error| error.to_string())?;
        screen.frame = Some(PixelsInRegion {
            image,
            bounds: region,
        });
    }
    compose(&screens, bounds).ok_or_else(|| "The capture area is outside the displays".into())
}

/// Start an area capture, a capture for OCR, an added capture, or Repeat Capture of the
/// last area. The editor hides first and shows again with the result.
pub(crate) fn start(
    command: Command,
    previous: Option<Region>,
    editor: &mut Editor,
) -> Result<(), String> {
    if matches!(command, Command::Repeat) && previous.is_none() {
        return Err("Capture an area before using Repeat Capture".into());
    }
    let escape = shell::begin_escape_session();
    let delay = shell::hide_for_capture();
    #[cfg(target_os = "macos")]
    {
        // Live selection leaves the editor in place, so there is no delay. xcap reads AppKit
        // display names, so the monitors are listed on the UI thread.
        let _ = delay;
        let screens = sniplet_platform::list_monitors()
            .map(|monitors| monitors.iter().map(Screen::live).collect());
        ready(editor, screens, command, previous, escape);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let session = escape.clone();
        editor.inbox.spawn(
            move || {
                std::thread::sleep(delay);
                (!session.is_cancelled()).then(|| {
                    sniplet_platform::capture_monitors()
                        .map(|frames| frames.into_iter().map(Screen::new).collect())
                })
            },
            move |editor, screens, _| match screens {
                Some(screens) => ready(editor, screens, command, previous, escape),
                None => shell::end_escape_session(&escape),
            },
        );
    }
    Ok(())
}

/// The monitors are ready: open the selection, or capture the last area again.
fn ready(
    editor: &mut Editor,
    screens: sniplet_platform::Result<Vec<Screen>>,
    command: Command,
    previous: Option<Region>,
    escape: EscapeSession,
) {
    if escape.is_cancelled() {
        shell::end_escape_session(&escape);
        return;
    }
    let result = screens
        .map_err(|error| error.to_string())
        .and_then(|screens| {
            if screens.is_empty() {
                return Err("No displays are available for capture".into());
            }
            match previous.filter(|_| matches!(command, Command::Repeat)) {
                Some(previous) if !layout_matches(&previous, &screens) => {
                    Err("Repeat Capture stopped because the display layout changed".into())
                }
                Some(previous) => {
                    capture_and_load(
                        screens,
                        previous.bounds,
                        &editor.inbox,
                        command,
                        escape.clone(),
                        Vec::new(),
                    );
                    Ok(())
                }
                None => {
                    open(screens, Finish::Area(command), &editor.inbox, &escape);
                    Ok(())
                }
            }
        });
    if let Err(error) = result {
        editor.status = error;
        shell::end_escape_session(&escape);
        shell::show_editor();
    }
}

/// Capture `bounds` on a worker, then load it into the editor and show the editor.
fn capture_and_load(
    screens: Vec<Screen>,
    bounds: Rect,
    editor: &Inbox<Editor>,
    command: Command,
    escape: EscapeSession,
    excluded: Vec<u32>,
) {
    let saved = region(&screens, bounds);
    // xcap reads AppKit display names. Keep metadata access on the UI thread.
    #[cfg(target_os = "macos")]
    let monitors = sniplet_platform::list_monitors().map_err(|error| error.to_string());
    // Only macOS reads the live desktop, which must leave the overlays out.
    let _ = &excluded;
    let session = escape.clone();
    editor.spawn(
        move || {
            if session.is_cancelled() {
                return Err("Capture cancelled".to_owned());
            }
            capture_selected(
                screens,
                bounds,
                #[cfg(target_os = "macos")]
                &monitors?,
                #[cfg(target_os = "macos")]
                &excluded,
            )
            .map(|(image, scale)| (Document::new(image), scale))
        },
        move |editor, result, tx| {
            if escape.is_cancelled() {
                shell::end_escape_session(&escape);
                return;
            }
            match result {
                Ok((document, scale)) => {
                    capture::copy_capture_if_enabled(editor, &document);
                    editor.scroll_region = None;
                    editor.scroll_frames.clear();
                    if matches!(command, Command::Area) {
                        editor.last_capture = Some(saved);
                    }
                    if matches!(command, Command::AddCapture) {
                        editor.add_image(document.original().clone(), "Added capture", true);
                    } else {
                        let status = if matches!(command, Command::Repeat) {
                            "Repeated area capture"
                        } else {
                            "Area captured"
                        };
                        editor.load(document, status);
                        editor.set_measure_scale(scale);
                        if matches!(command, Command::CaptureOcr) {
                            editor.command(Command::Ocr, tx);
                        }
                    }
                }
                Err(error) => editor.status = error,
            }
            shell::show_editor();
            shell::end_escape_session(&escape);
        },
    );
}

/// What a finished selection does.
#[derive(Clone, Copy)]
enum Finish {
    /// Capture the area for this command.
    Area(Command),
    /// Start a scrolling capture of the area on this monitor.
    Scroll {
        command: Command,
        monitor: usize,
        scale_factor: f32,
    },
}

/// The selection every monitor's overlay shares.
struct Selection {
    screens: Vec<Screen>,
    windows: Vec<WindowHandle>,
    /// Each overlay's inbox, to redraw the others when the selection changes.
    overlays: Vec<Inbox<Overlay>>,
    editor: Inbox<Editor>,
    finish: Finish,
    escape: EscapeSession,
    start: Option<ImagePoint>,
    end: ImagePoint,
    completed: bool,
}

impl Selection {
    fn bounds(&self) -> Option<Rect> {
        self.start.map(|start| Rect::from_corners(start, self.end))
    }

    fn move_to(&mut self, end: ImagePoint, shift: bool) {
        let Some(start) = self.start.filter(|_| !self.completed) else {
            return;
        };
        self.end = if shift {
            capture::square_end(start, end)
        } else {
            end
        };
    }

    /// Redraw every overlay.
    fn notify(&self) {
        for overlay in &self.overlays {
            overlay.send(|_, _| {});
        }
    }

    fn close(&mut self) {
        self.completed = true;
        for window in self.windows.drain(..) {
            window.close();
        }
    }

    fn cancel(&mut self) {
        if self.completed {
            return;
        }
        shell::cancel_escape_session(&self.escape);
        self.close();
    }

    fn finish(&mut self) {
        if self.completed || self.escape.is_cancelled() {
            return;
        }
        let Some(bounds) = self.bounds() else {
            return;
        };
        if bounds.w < 2.0 || bounds.h < 2.0 {
            return;
        }
        // macOS reads the live desktop after the release. Leave the overlays out of it.
        let excluded = self
            .windows
            .iter()
            .filter_map(WindowHandle::window_number)
            .collect::<Vec<_>>();
        // Mark completion before closing the overlays so closing does not cancel.
        self.close();
        let screens = std::mem::take(&mut self.screens);
        match self.finish {
            Finish::Area(command) => capture_and_load(
                screens,
                bounds,
                &self.editor,
                command,
                self.escape.clone(),
                excluded,
            ),
            Finish::Scroll {
                command,
                monitor,
                scale_factor,
            } => {
                let screen = &screens[0];
                let Some(frame) = &screen.frame else {
                    return;
                };
                let scale = screen.scale();
                let origin = screen.layout.bounds;
                let rect = ImageRect::new(
                    (bounds.x - origin.x) * scale,
                    (bounds.y - origin.y) * scale,
                    bounds.w * scale,
                    bounds.h * scale,
                );
                capture::scroll_selected(
                    capture::ScrollSelection {
                        command,
                        monitor,
                        scale_factor,
                        source: &frame.image,
                        rect,
                    },
                    &self.editor,
                    &self.escape,
                );
            }
        }
    }
}

/// One monitor's overlay window.
pub(crate) struct Overlay {
    inbox: Inbox<Overlay>,
    selection: Rc<RefCell<Selection>>,
    /// The overlay's place in `Selection::windows`, for macOS focus.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    index: usize,
    bounds: Rect,
    viewport: (f32, f32),
    pixels: Option<Pixels>,
}

impl WindowState for Overlay {
    fn inbox(&self) -> &Inbox<Self> {
        &self.inbox
    }
}

impl Drop for Overlay {
    /// An overlay closed by anything but a finished selection cancels the capture.
    fn drop(&mut self) {
        if let Ok(mut selection) = self.selection.try_borrow_mut() {
            selection.cancel();
        }
    }
}

impl Overlay {
    /// The pointer of a drag, in desktop units. Windows and Linux read the desktop pixel
    /// itself, which stays exact when the drag crosses onto a monitor of another scale.
    fn desktop_point(&self, drag: &Drag) -> ImagePoint {
        #[cfg(not(target_os = "macos"))]
        if let Some((x, y)) = drag.screen_point() {
            return ImagePoint::new(x as f32, y as f32);
        }
        capture_overlay::to_desktop(self.bounds, self.viewport, drag.x, drag.y)
    }

    fn drag(&mut self, drag: &mut Drag, tx: &mut Tx) {
        let point = self.desktop_point(drag);
        let mut selection = self.selection.borrow_mut();
        match drag.phase {
            Phase::Down => {
                #[cfg(target_os = "macos")]
                if let Some(window) = selection.windows.get(self.index) {
                    crate::macos::focus_capture(window);
                }
                if !selection.completed {
                    selection.start = Some(point);
                    selection.end = point;
                }
            }
            Phase::Move => selection.move_to(point, tx.mods().shift),
            // Escape during a drag reaches the overlay as a cancelled drag.
            Phase::Up if drag.cancelled() => selection.cancel(),
            Phase::Up => {
                selection.move_to(point, tx.mods().shift);
                selection.finish();
            }
        }
        selection.notify();
    }
}

/// Build one overlay state per screen around a shared selection. `open` gives each a window.
fn overlays(
    screens: Vec<Screen>,
    finish: Finish,
    editor: &Inbox<Editor>,
    escape: &EscapeSession,
) -> Vec<Overlay> {
    let overlays = screens
        .iter()
        .map(|screen| {
            (
                screen.layout.bounds,
                capture_overlay::initial_viewport(&screen.layout, screen.scale_factor),
                screen.frame.as_ref().map(|frame| {
                    let pixels = Pixels::new();
                    let (width, height, rgba) = crate::editor::display_pixels(&frame.image);
                    pixels.set(width, height, rgba);
                    pixels
                }),
            )
        })
        .collect::<Vec<_>>();
    let selection = Rc::new(RefCell::new(Selection {
        screens,
        windows: Vec::new(),
        overlays: Vec::new(),
        editor: editor.clone(),
        finish,
        escape: escape.clone(),
        start: None,
        end: ImagePoint::default(),
        completed: false,
    }));
    let overlays = overlays
        .into_iter()
        .enumerate()
        .map(|(index, (bounds, viewport, pixels))| Overlay {
            inbox: Inbox::default(),
            selection: selection.clone(),
            index,
            bounds,
            viewport,
            pixels,
        })
        .collect::<Vec<_>>();
    selection.borrow_mut().overlays = overlays.iter().map(|o| o.inbox.clone()).collect();
    overlays
}

/// Open an overlay on every screen. Escape anywhere cancels them all.
fn open(screens: Vec<Screen>, finish: Finish, editor: &Inbox<Editor>, escape: &EscapeSession) {
    let monitors = screens
        .iter()
        .map(|screen| (screen.layout.clone(), screen.frame.is_none()))
        .collect::<Vec<_>>();
    let overlays = overlays(screens, finish, editor, escape);
    let Some(selection) = overlays.first().map(|overlay| overlay.selection.clone()) else {
        return;
    };
    for (overlay, (monitor, live)) in overlays.into_iter().zip(monitors) {
        let window = capture_overlay::open_overlay(
            &monitor,
            capture_overlay::Window {
                title: "Sniplet — Capture area",
                size: (monitor.bounds.w, monitor.bounds.h),
                view,
                on_key,
                options: monitor.window_options(live),
            },
            overlay,
        );
        shell::attach_escape_window(escape, window.clone());
        selection.borrow_mut().windows.push(window);
    }
}

/// Select an area of the monitor in `frame` for a scrolling capture.
pub(crate) fn open_scroll(
    frame: sniplet_platform::CapturedFrame,
    command: Command,
    editor: &Inbox<Editor>,
    escape: &EscapeSession,
) {
    let monitor = match frame.source {
        sniplet_platform::CaptureSource::Monitor { index, .. }
        | sniplet_platform::CaptureSource::MonitorRegion { index, .. } => index,
        sniplet_platform::CaptureSource::Window { .. } => 0,
    };
    let finish = Finish::Scroll {
        command,
        monitor,
        scale_factor: frame.scale_factor,
    };
    open(vec![Screen::new(frame)], finish, editor, escape);
}

fn on_key(s: &mut Overlay, event: &KeyEvent, _: &mut Tx) -> bool {
    let mut selection = s.selection.borrow_mut();
    match event.key {
        Key::Escape => selection.cancel(),
        Key::Enter => selection.finish(),
        _ => return false,
    }
    selection.notify();
    true
}

fn view(s: &Overlay, _: &mut Cx<Overlay>) -> El {
    let selection = s.selection.borrow();
    let (width, height) = s.viewport;
    let selected = selection.bounds();
    let scale = selected.map_or(1.0, |rect| selection_scale(&selection.screens, rect));
    let local = selected.map(|rect| capture_overlay::to_local(s.bounds, s.viewport, rect));
    let mut overlay = el()
        .key("capture-overlay")
        .w_full()
        .h_full()
        .clip_rounded(0.0)
        .cursor(Cursor::Crosshair)
        .on_layout(|s: &mut Overlay, rect: [f32; 4]| s.viewport = (rect[2], rect[3]))
        .on_drag(|s: &mut Overlay, drag: &mut Drag, tx: &mut Tx| s.drag(drag, tx))
        .on_secondary_press(|s: &mut Overlay, _, _| s.selection.borrow_mut().cancel());
    #[cfg(target_os = "macos")]
    {
        overlay = overlay.on_pointer_move(|s: &mut Overlay, _| {
            // The pointer can move to another display while one overlay has the keys.
            if let Some(window) = s.selection.borrow().windows.get(s.index) {
                crate::macos::focus_capture_under_pointer(window);
            }
        });
    }
    if let Some(pixels) = &s.pixels {
        overlay = overlay.child(el().absolute(0.0, 0.0).w_full().h_full().pixels(pixels));
    }
    let viewport = Rect::new(0.0, 0.0, width, height);
    for dim in capture_overlay::subtract_rectangles(viewport, local.as_slice()) {
        overlay = overlay.child(
            el().absolute(dim.x, dim.y)
                .size(dim.w, dim.h)
                .fill(capture_overlay::dim(0x65)),
        );
    }
    if let (Some(rect), Some(local)) = (selected, local)
        && rect.overlaps(&s.bounds)
    {
        overlay = overlay
            .child(
                el().key("capture-selection")
                    .absolute(local.x, local.y)
                    .size(local.w, local.h)
                    .border(1.0, rune::Color::WHITE),
            )
            .child(
                el().key("capture-size")
                    .absolute(local.x.max(8.0), (local.y - 35.0).max(8.0))
                    .px(12.0)
                    .py(4.0)
                    .radius(6.0)
                    .fill(capture_overlay::pill())
                    .child(
                        text(format!("{:.0} × {:.0} px", rect.w * scale, rect.h * scale))
                            .font(BODY)
                            .color(rune::Color::WHITE)
                            .single_line(),
                    ),
            );
    }
    overlay.child(capture_overlay::hint(
        "Drag to capture an area · Esc to cancel",
    ))
}

#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(x, y, width, height)
    }

    fn screen(id: u32, bounds: Rect, scale: f32) -> Screen {
        let width = (bounds.w * scale) as u32;
        let height = (bounds.h * scale) as u32;
        Screen {
            frame: Some(PixelsInRegion {
                image: RgbaImage::from_fn(width, height, |x, y| {
                    Rgba([id as u8, x as u8, y as u8, 255])
                }),
                bounds,
            }),
            layout: Monitor {
                id,
                bounds,
                pixels: (width, height),
            },
            scale_factor: scale,
        }
    }

    #[test]
    fn capture_on_external_display_preserves_source_pixels() {
        let screens = vec![
            screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
            screen(2, rect(-100.0, -80.0, 100.0, 80.0), 2.0),
        ];
        let (image, scale) = compose(&screens, rect(-95.5, -70.5, 20.0, 15.0)).unwrap();
        assert_eq!(scale, 2.0);
        assert_eq!(
            image,
            image::imageops::crop_imm(&screens[1].frame.as_ref().unwrap().image, 9, 19, 40, 30)
                .to_image()
        );
    }

    #[test]
    fn spanning_capture_keeps_display_positions_and_scales() {
        let screens = vec![
            screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
            screen(2, rect(-20.0, -80.0, 120.0, 80.0), 1.0),
        ];
        let (image, scale) = compose(&screens, rect(-10.0, -10.0, 40.0, 30.0)).unwrap();
        assert_eq!(scale, 2.0);
        assert_eq!(image.dimensions(), (80, 60));
        assert_eq!(image.get_pixel(0, 0), &Rgba([2, 10, 70, 255]));
        assert_eq!(image.get_pixel(1, 1), &Rgba([2, 10, 70, 255]));
        assert_eq!(image.get_pixel(20, 20), &Rgba([1, 0, 0, 255]));
        assert_eq!(image.get_pixel(79, 59), &Rgba([1, 59, 39, 255]));
        assert_eq!(image.get_pixel(0, 20), &Rgba([0, 0, 0, 255]));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn selected_region_pixels_match_full_frames_across_different_display_scales() {
        let mut screens = vec![
            screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
            screen(2, rect(-20.0, -80.0, 120.0, 80.0), 1.0),
        ];
        let bounds = rect(-10.25, -10.5, 40.5, 30.75);
        let expected = compose(&screens, bounds).unwrap();
        for screen in &mut screens {
            let region = capture_bounds(screen, bounds);
            let source = screen.frame.as_ref().unwrap();
            let scale = screen.scale();
            let image = image::imageops::crop_imm(
                &source.image,
                ((region.x - source.bounds.x) * scale).round() as u32,
                ((region.y - source.bounds.y) * scale).round() as u32,
                (region.w * scale).round() as u32,
                (region.h * scale).round() as u32,
            )
            .to_image();
            screen.frame = Some(PixelsInRegion {
                image,
                bounds: region,
            });
        }
        assert_eq!(compose(&screens, bounds).unwrap(), expected);
    }

    #[test]
    fn repeat_accepts_reordered_displays_but_rejects_a_changed_layout() {
        let mut screens = vec![
            screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
            screen(2, rect(100.0, 0.0, 100.0, 100.0), 1.0),
        ];
        let saved = region(&screens, rect(80.0, 20.0, 40.0, 60.0));
        screens.swap(0, 1);
        assert!(layout_matches(&saved, &screens));
        screens[0].layout.bounds.x = -100.0;
        assert!(!layout_matches(&saved, &screens));
    }

    const ENV: Env = Env {
        system_dark: true,
        scale: 2.0,
    };

    fn editor() -> Runtime<Editor> {
        let settings = sniplet_platform::Settings {
            auto_copy: false,
            ..Default::default()
        };
        let editor = Editor::new(Some(Document::new(RgbaImage::new(80, 60))), settings, 1.0);
        Runtime::new(editor, crate::editor::view, (1280.0, 850.0), ENV)
    }

    /// Run the editor's inbox once a worker has reported to it.
    fn settle(editor: &mut Runtime<Editor>) {
        for _ in 0..400 {
            editor.update(|editor, tx| {
                Inbox::drain(editor, |editor| &editor.inbox, tx);
            });
            if editor.state.inbox.is_idle() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the capture never reported to the editor");
    }

    /// Two 100 × 100 point displays side by side: a 2× one and a 1× one.
    fn selection(editor: &Runtime<Editor>, escape: &EscapeSession) -> Vec<Runtime<Overlay>> {
        let screens = vec![
            screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
            screen(2, rect(100.0, 0.0, 100.0, 100.0), 1.0),
        ];
        overlays(
            screens,
            Finish::Area(Command::Area),
            &editor.state.inbox,
            escape,
        )
        .into_iter()
        .map(|overlay| {
            let mut rt = Runtime::new(overlay, view, (100.0, 100.0), ENV);
            rt.on_key = Some(on_key);
            rt.frame(0.0);
            rt
        })
        .collect()
    }

    #[test]
    fn drag_continues_outside_the_starting_display_in_both_directions() {
        for (index, start, end) in [
            (0, (60.0, 40.0), (120.0, 80.0)),
            (1, (20.0, 80.0), (-40.0, 40.0)),
        ] {
            let mut editor = editor();
            let mut overlays = selection(&editor, &EscapeSession::detached());
            let overlay = &mut overlays[index];
            overlay.pointer_down(start.0, start.1);
            overlay.pointer_move(end.0, end.1);
            overlay.pointer_up(end.0, end.1);
            overlay.frame(1.0 / 60.0);
            assert!(overlay.state.selection.borrow().completed);
            settle(&mut editor);
            let editor = &editor.state;
            let image = editor.document.as_ref().unwrap().original();
            assert_eq!(image.dimensions(), (120, 80));
            assert_eq!(image.get_pixel(0, 0), &Rgba([1, 120, 80, 255]));
            assert_eq!(image.get_pixel(119, 79), &Rgba([2, 19, 79, 255]));
            assert!(editor.last_capture.is_some());
        }
    }

    #[test]
    fn the_other_display_draws_the_selection_too() {
        let editor = editor();
        let mut overlays = selection(&editor, &EscapeSession::detached());
        overlays[0].pointer_down(60.0, 40.0);
        overlays[0].pointer_move(150.0, 80.0);
        overlays[0].frame(1.0 / 60.0);
        // The shared selection reaches the other display through its inbox.
        overlays[1].update(|overlay, tx| {
            Inbox::drain(overlay, |overlay| &overlay.inbox, tx);
        });
        overlays[1].frame(1.0 / 60.0);
        assert!(overlays[1].find("capture-selection").is_some());
        assert!(overlays[1].find("capture-size").is_some());
    }

    #[test]
    fn escape_or_right_click_on_either_display_cancels_the_whole_selection() {
        for (index, right_click) in [(0, false), (1, false), (0, true)] {
            let editor = editor();
            let escape = EscapeSession::detached();
            let mut overlays = selection(&editor, &escape);
            let overlay = &mut overlays[index];
            if right_click {
                overlay.secondary_down(50.0, 50.0);
                overlay.secondary_up(50.0, 50.0);
            } else {
                overlay.pointer_down(20.0, 20.0);
                overlay.pointer_move(40.0, 40.0);
                overlay.key(KeyEvent {
                    key: Key::Escape,
                    mods: Default::default(),
                });
            }
            overlay.frame(1.0 / 60.0);
            assert!(escape.is_cancelled());
            assert!(overlays[0].state.selection.borrow().completed);
            assert_eq!(
                editor
                    .state
                    .document
                    .as_ref()
                    .unwrap()
                    .original()
                    .dimensions(),
                (80, 60)
            );
            assert!(editor.state.last_capture.is_none());
        }
    }

    #[test]
    fn closing_an_overlay_cancels_its_capture() {
        let editor = editor();
        let escape = EscapeSession::detached();
        drop(selection(&editor, &escape));
        assert!(escape.is_cancelled());
    }
}
