use std::{sync::Arc, time::Instant};

use gpui_kit::{prelude::*, *};
use image::{Rgba, RgbaImage};
use sniplet_core::{Document, ImageRect, Point as ImagePoint};
#[cfg(any(not(target_os = "macos"), test))]
use sniplet_platform::{CaptureSource, CapturedFrame};

use crate::{
    capture, capture_overlay,
    editor::{Command, Editor, display_image},
    runtime,
};

#[derive(Clone, Debug, PartialEq)]
struct DisplayLayout {
    id: u32,
    bounds: Bounds<Pixels>,
    pixels: (u32, u32),
}

/// Desktop coordinates and display geometry are retained for Repeat Capture.
#[derive(Clone)]
pub(crate) struct Region {
    bounds: Bounds<Pixels>,
    displays: Vec<DisplayLayout>,
}

#[derive(Clone)]
struct PixelsInRegion {
    image: RgbaImage,
    bounds: Bounds<Pixels>,
}

#[derive(Clone)]
struct Screen {
    frame: Option<PixelsInRegion>,
    layout: DisplayLayout,
    window_bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    #[cfg(target_os = "macos")]
    overlay_id: Option<u32>,
}

impl Screen {
    #[cfg(not(target_os = "macos"))]
    fn new(frame: CapturedFrame, cx: &App) -> Self {
        let (window_bounds, display_id) = capture_overlay::captured_frame_placement(&frame, cx);
        let bounds = desktop_bounds(&frame, window_bounds);
        let id = match frame.source {
            CaptureSource::Monitor { id, .. } => id,
            _ => unreachable!("area capture uses whole display images"),
        };
        Self {
            layout: DisplayLayout {
                id,
                bounds,
                pixels: frame.image.dimensions(),
            },
            frame: Some(PixelsInRegion {
                image: frame.image,
                bounds,
            }),
            window_bounds,
            display_id,
        }
    }

    #[cfg(target_os = "macos")]
    fn live(info: sniplet_platform::MonitorInfo, cx: &App) -> Self {
        let origin = sniplet_platform::ScreenPoint {
            x: info.x,
            y: info.y,
        };
        let displays = cx
            .displays()
            .into_iter()
            .map(|display| capture_overlay::DisplayGeometry {
                id: display.id(),
                bounds: display.bounds(),
            })
            .collect::<Vec<_>>();
        let (window_bounds, display_id) = capture_overlay::overlay_placement(
            info.id,
            origin,
            (info.width, info.height),
            info.scale_factor,
            &displays,
            true,
            false,
        );
        Self {
            frame: None,
            layout: DisplayLayout {
                id: info.id,
                bounds: Bounds::new(
                    point(px(info.x as f32), px(info.y as f32)),
                    size(
                        px(info.width as f32 / info.scale_factor),
                        px(info.height as f32 / info.scale_factor),
                    ),
                ),
                pixels: (info.width, info.height),
            },
            window_bounds,
            display_id,
            overlay_id: None,
        }
    }

    fn scale(&self) -> f32 {
        self.layout.pixels.0 as f32 / f32::from(self.layout.bounds.size.width)
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn desktop_bounds(frame: &CapturedFrame, placement: Bounds<Pixels>) -> Bounds<Pixels> {
    // GPUI's Mac display bounds are local to each screen. Quartz capture
    // origins use one desktop coordinate space, including negative positions.
    if cfg!(target_os = "macos") {
        Bounds::new(
            point(px(frame.origin.x as f32), px(frame.origin.y as f32)),
            placement.size,
        )
    } else {
        placement
    }
}

fn overlaps(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    let intersection = a.intersect(&b);
    intersection.size.width > px(0.0) && intersection.size.height > px(0.0)
}

fn selection_scale(screens: &[Screen], bounds: Bounds<Pixels>) -> f32 {
    screens
        .iter()
        .filter(|screen| overlaps(screen.layout.bounds, bounds))
        .map(Screen::scale)
        .fold(1.0, f32::max)
}

fn region(screens: &[Screen], bounds: Bounds<Pixels>) -> Region {
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

/// Compose in desktop points at the highest selected display scale. Empty
/// desktop space stays black; each display samples its own physical pixels.
fn compose(screens: &[Screen], bounds: Bounds<Pixels>) -> Option<(RgbaImage, f32)> {
    let desktop = screens
        .iter()
        .map(|screen| screen.layout.bounds)
        .reduce(|a, b| a.union(&b))?;
    let bounds = bounds.intersect(&desktop);
    if !screens
        .iter()
        .any(|screen| overlaps(screen.layout.bounds, bounds))
    {
        return None;
    }
    let scale = selection_scale(screens, bounds);
    let left = (f32::from(bounds.left()) * scale).floor();
    let top = (f32::from(bounds.top()) * scale).floor();
    let width = ((f32::from(bounds.right()) * scale).ceil() - left) as u32;
    let height = ((f32::from(bounds.bottom()) * scale).ceil() - top) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut image = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
    for screen in screens {
        if !overlaps(screen.layout.bounds, bounds) {
            continue;
        }
        let frame = screen.frame.as_ref()?;
        let display = frame.bounds;
        let x0 = (f32::from(display.left()) * scale - left)
            .round()
            .clamp(0.0, width as f32) as u32;
        let y0 = (f32::from(display.top()) * scale - top)
            .round()
            .clamp(0.0, height as f32) as u32;
        let x1 = (f32::from(display.right()) * scale - left)
            .round()
            .clamp(0.0, width as f32) as u32;
        let y1 = (f32::from(display.bottom()) * scale - top)
            .round()
            .clamp(0.0, height as f32) as u32;
        let sx = frame.image.width() as f32 / f32::from(display.size.width);
        let sy = frame.image.height() as f32 / f32::from(display.size.height);
        for y in y0..y1 {
            let source_y = (((top + y as f32 + 0.5) / scale - f32::from(display.top())) * sy)
                .floor()
                .clamp(0.0, (frame.image.height() - 1) as f32) as u32;
            for x in x0..x1 {
                let source_x = (((left + x as f32 + 0.5) / scale - f32::from(display.left())) * sx)
                    .floor()
                    .clamp(0.0, (frame.image.width() - 1) as f32)
                    as u32;
                image.put_pixel(x, y, *frame.image.get_pixel(source_x, source_y));
            }
        }
    }
    Some((image, scale))
}

/// Round outward to each display's pixel grid. Keeping its desktop origin
/// preserves Retina pixels and selections spanning displays with different scales.
#[cfg(target_os = "macos")]
fn capture_bounds(screen: &Screen, selection: Bounds<Pixels>) -> Bounds<Pixels> {
    let bounds = selection.intersect(&screen.layout.bounds);
    let scale = screen.scale();
    let left = f32::from(screen.layout.bounds.left());
    let top = f32::from(screen.layout.bounds.top());
    let x0 = ((f32::from(bounds.left()) - left) * scale).floor() / scale + left;
    let y0 = ((f32::from(bounds.top()) - top) * scale).floor() / scale + top;
    let x1 = ((f32::from(bounds.right()) - left) * scale).ceil() / scale + left;
    let y1 = ((f32::from(bounds.bottom()) - top) * scale).ceil() / scale + top;
    Bounds::new(point(px(x0), px(y0)), size(px(x1 - x0), px(y1 - y0)))
        .intersect(&screen.layout.bounds)
}

fn capture_selected(
    screens: Vec<Screen>,
    bounds: Bounds<Pixels>,
    #[cfg(target_os = "macos")] monitors: &[sniplet_platform::MonitorInfo],
) -> Result<(RgbaImage, f32), String> {
    #[cfg(target_os = "macos")]
    let mut screens = screens;
    #[cfg(target_os = "macos")]
    if screens.iter().any(|screen| screen.frame.is_none()) {
        let excluded = screens
            .iter()
            .filter_map(|screen| screen.overlay_id)
            .collect::<Vec<_>>();
        for screen in &mut screens {
            if screen.frame.is_some() || !overlaps(screen.layout.bounds, bounds) {
                continue;
            }
            let layout = &screen.layout;
            if !monitors.iter().any(|info| {
                info.id == layout.id
                    && (info.width, info.height) == layout.pixels
                    && info.x as f32 == f32::from(layout.bounds.left())
                    && info.y as f32 == f32::from(layout.bounds.top())
                    && (info.scale_factor - screen.scale()).abs() < 0.001
            }) {
                return Err("Capture stopped because the display layout changed".into());
            }
            let region = capture_bounds(screen, bounds);
            let image = sniplet_platform::capture_desktop_region(
                ImageRect::new(
                    f32::from(region.left()),
                    f32::from(region.top()),
                    f32::from(region.size.width),
                    f32::from(region.size.height),
                ),
                &excluded,
            )
            .map_err(|error| error.to_string())?;
            if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                eprintln!(
                    "capture: live region display={}; bounds={region:?}; pixels={:?}",
                    layout.id,
                    image.dimensions()
                );
            }
            screen.frame = Some(PixelsInRegion {
                image,
                bounds: region,
            });
        }
    }
    compose(&screens, bounds).ok_or_else(|| "The capture area is outside the displays".into())
}

pub(crate) fn start(
    command: Command,
    previous: Option<Region>,
    editor: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> Result<(), String> {
    if matches!(command, Command::Repeat) && previous.is_none() {
        return Err("Capture an area before using Repeat Capture".into());
    }
    let started = Instant::now();
    let owner = window.window_handle();
    let escape = runtime::begin_escape_session(cx);
    let delay = runtime::hide_for_capture(window);
    cx.spawn(async move |_, cx| {
        if !delay.is_zero() {
            cx.background_executor().timer(delay).await;
        }
        if escape.is_cancelled() {
            return;
        }
        #[cfg(target_os = "macos")]
        let available = cx.update(|_| sniplet_platform::list_monitors());
        #[cfg(not(target_os = "macos"))]
        let available = cx
            .background_executor()
            .spawn(async { sniplet_platform::capture_monitors() })
            .await;
        cx.update(|cx| {
            if escape.is_cancelled() {
                runtime::end_escape_session(&escape, cx);
                return;
            }
            let result = available
                .map_err(|error| error.to_string())
                .and_then(|available| {
                    #[cfg(target_os = "macos")]
                    let screens: Vec<_> = available
                        .into_iter()
                        .map(|info| Screen::live(info, cx))
                        .collect();
                    #[cfg(not(target_os = "macos"))]
                    let screens: Vec<_> = available
                        .into_iter()
                        .map(|frame| Screen::new(frame, cx))
                        .collect();
                    if screens.is_empty() {
                        return Err("No displays are available for capture".into());
                    }
                    if matches!(command, Command::Repeat) {
                        let previous = previous.unwrap();
                        if !layout_matches(&previous, &screens) {
                            return Err(
                                "Repeat Capture stopped because the display layout changed".into(),
                            );
                        }
                        capture_and_load(
                            screens,
                            previous.bounds,
                            editor.clone(),
                            owner,
                            command,
                            escape.clone(),
                            cx,
                        );
                    } else {
                        let selection =
                            open(screens, editor.clone(), owner, command, escape.clone(), cx)?;
                        if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                            for handle in &selection.read(cx).windows.clone() {
                                let _ = handle.update(cx, |_, window, _| {
                                    window.on_next_frame(move |_, _| {
                                        eprintln!(
                                            "capture: area first frame in {:?}",
                                            started.elapsed()
                                        )
                                    });
                                });
                            }
                        }
                    }
                    Ok(())
                });
            if let Err(error) = result {
                editor.update(cx, |editor, cx| {
                    editor.status = error;
                    cx.notify();
                });
                runtime::end_escape_session(&escape, cx);
                capture::restore(owner, cx);
            }
            if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                eprintln!(
                    "capture: desktop selection ready in {:?}",
                    started.elapsed()
                );
            }
        });
    })
    .detach();
    Ok(())
}

fn capture_and_load(
    screens: Vec<Screen>,
    bounds: Bounds<Pixels>,
    editor: Entity<Editor>,
    owner: AnyWindowHandle,
    command: Command,
    escape: runtime::EscapeSession,
    cx: &mut App,
) {
    let saved = region(&screens, bounds);
    let started = Instant::now();
    // xcap reads AppKit display names. Keep metadata access on the UI thread.
    #[cfg(target_os = "macos")]
    let monitors = sniplet_platform::list_monitors().map_err(|error| error.to_string());
    cx.spawn(async move |cx| {
        let cancelled = escape.clone();
        let result = cx.background_executor().spawn(async move {
            if cancelled.is_cancelled() { return Err("Capture cancelled".into()); }
            capture_selected(
                screens,
                bounds,
                #[cfg(target_os = "macos")]
                &monitors?,
            )
        }).await;
        cx.update(|cx| {
            if escape.is_cancelled() {
                runtime::end_escape_session(&escape, cx);
                return;
            }
            let _ = owner.update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    match result {
                        Ok((image, scale)) => {
                            if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                                eprintln!("capture: selected {bounds:?}; output={:?}; scale={scale}; pixels ready in {:?}", image.dimensions(), started.elapsed());
                            }
                            capture::copy_capture_if_enabled(editor, &image);
                            editor.scroll_region = None;
                            editor.scroll_frames.clear();
                            if matches!(command, Command::Area) { editor.last_capture = Some(saved); }
                            if matches!(command, Command::AddCapture) {
                                editor.add_image(image, "Added capture", true, cx);
                            } else {
                                let status = if matches!(command, Command::Repeat) {
                                    "Repeated area capture"
                                } else { "Area captured" };
                                editor.load(Document::new(image), status, cx);
                                editor.set_measure_scale(scale);
                                if matches!(command, Command::CaptureOcr) {
                                    editor.command(Command::Ocr, window, cx);
                                }
                            }
                        }
                        Err(error) => { editor.status = error; cx.notify(); }
                    }
                });
                runtime::show_editor(window, cx);
                if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                    window.on_next_frame(move |_, _| eprintln!("capture: area editor first frame after release in {:?}", started.elapsed()));
                }
            });
            runtime::end_escape_session(&escape, cx);
        });
    }).detach();
}

struct Selection {
    screens: Vec<Screen>,
    windows: Vec<AnyWindowHandle>,
    editor: Entity<Editor>,
    owner: AnyWindowHandle,
    command: Command,
    escape: runtime::EscapeSession,
    start: Option<ImagePoint>,
    end: ImagePoint,
    completed: bool,
}

impl Selection {
    fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.start.map(|start| {
            let rect = ImageRect::from_corners(start, self.end);
            Bounds::new(
                point(px(rect.x), px(rect.y)),
                size(px(rect.width), px(rect.height)),
            )
        })
    }

    fn move_to(&mut self, end: ImagePoint, shift: bool, cx: &mut Context<Self>) {
        if self.start.is_none() || self.completed {
            return;
        }
        self.end = if shift {
            capture::square_end(self.start.unwrap(), end)
        } else {
            end
        };
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.completed = true;
        let windows = std::mem::take(&mut self.windows);
        // The initiating window may still be dispatching its mouse event.
        cx.defer(move |cx| {
            for handle in windows {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        });
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.completed {
            return;
        }
        runtime::cancel_escape_session(&self.escape, cx);
        self.close(cx);
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        if self.completed || self.escape.is_cancelled() {
            return;
        }
        let Some(bounds) = self.bounds() else {
            return;
        };
        if bounds.size.width < px(2.0) || bounds.size.height < px(2.0) {
            return;
        }
        // Mark completion before releasing panels so their release handlers do
        // not cancel the capture. macOS excludes them from the pixel readback.
        self.close(cx);
        capture_and_load(
            std::mem::take(&mut self.screens),
            bounds,
            self.editor.clone(),
            self.owner,
            self.command,
            self.escape.clone(),
            cx,
        );
    }
}

fn open(
    screens: Vec<Screen>,
    editor: Entity<Editor>,
    owner: AnyWindowHandle,
    command: Command,
    escape: runtime::EscapeSession,
    cx: &mut App,
) -> Result<Entity<Selection>, String> {
    let selection = cx.new(|_| Selection {
        screens,
        windows: Vec::new(),
        editor,
        owner,
        command,
        escape: escape.clone(),
        start: None,
        end: ImagePoint::default(),
        completed: false,
    });
    let count = selection.read(cx).screens.len();
    for index in 0..count {
        let screen = &selection.read(cx).screens[index];
        let bounds = screen.layout.bounds;
        let window_bounds = screen.window_bounds;
        let display_id = screen.display_id;
        let image = screen
            .frame
            .as_ref()
            .map(|frame| display_image(frame.image.clone()));
        let live = image.is_none();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(window_bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id,
            window_background: if image.is_none() {
                WindowBackgroundAppearance::Transparent
            } else {
                WindowBackgroundAppearance::Opaque
            },
            show: !cfg!(target_os = "macos"),
            ..Default::default()
        };
        let result = gpui_kit::open_window(options, cx, |window, cx| {
            #[cfg(target_os = "macos")]
            crate::macos::prepare_capture_overlay(window, display_id);
            window.set_window_title("Sniplet — Capture area");
            cx.new(|cx| {
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                cx.observe(&selection, |_, _, cx| cx.notify()).detach();
                cx.on_release(|this: &mut Overlay, cx| {
                    this.selection
                        .update(cx, |selection, cx| selection.cancel(cx));
                })
                .detach();
                Overlay {
                    image,
                    selection: selection.clone(),
                    bounds,
                    focus,
                    #[cfg(target_os = "macos")]
                    _cursor: crate::macos::CaptureCursor::new(window),
                }
            })
        });
        match result {
            Ok((handle, _)) => {
                runtime::attach_escape_window(&escape, handle, cx);
                selection.update(cx, |selection, _| selection.windows.push(handle));
                let _ = handle.update(cx, |_, window, cx| {
                    if live {
                        // Component roots have a theme background by default.
                        // The selector must leave the desktop visible beneath it.
                        gpui_kit::base::Root::update(window, cx, |root, _, cx| {
                            root.style().background = Some(rgba(0x00000000).into());
                            cx.notify();
                        });
                    }
                    #[cfg(target_os = "macos")]
                    {
                        crate::macos::show_capture_overlay(window);
                        selection.update(cx, |selection, _| {
                            selection.screens[index].overlay_id =
                                crate::macos::capture_window_id(window);
                        });
                    }
                    #[cfg(not(target_os = "macos"))]
                    window.activate_window();
                });
                if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                    eprintln!("capture: display {display_id:?} overlay {bounds:?}");
                    #[cfg(target_os = "macos")]
                    cx.spawn(async move |cx| {
                        for delay in [200, 1300] {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(delay))
                                .await;
                            let _ = handle.update(cx, |_, window, _| {
                                crate::macos::trace_capture_cursor(window)
                            });
                        }
                    })
                    .detach();
                }
            }
            Err(error) => {
                selection.update(cx, |selection, cx| selection.cancel(cx));
                return Err(error.to_string());
            }
        }
    }
    #[cfg(target_os = "macos")]
    for handle in &selection.read(cx).windows.clone() {
        let _ = handle.update(cx, |_, window, _| {
            crate::macos::focus_capture_under_pointer(window);
        });
    }
    Ok(selection)
}

struct Overlay {
    image: Option<Arc<RenderImage>>,
    selection: Entity<Selection>,
    bounds: Bounds<Pixels>,
    focus: FocusHandle,
    #[cfg(target_os = "macos")]
    _cursor: Option<crate::macos::CaptureCursor>,
}

fn desktop_point(bounds: Bounds<Pixels>, position: Point<Pixels>) -> ImagePoint {
    ImagePoint::new(
        f32::from(bounds.origin.x + position.x),
        f32::from(bounds.origin.y + position.y),
    )
}

impl Render for Overlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selection = self.selection.read(cx);
        let bounds = self.bounds;
        let selected = selection.bounds();
        let scale = selected
            .map(|rect| selection_scale(&selection.screens, rect))
            .unwrap_or(1.0);
        let drag = self.selection.clone();
        let release = self.selection.clone();
        let mut overlay = div()
            .id("capture-overlay")
            .relative()
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus)
            .cursor_crosshair()
            .children(
                self.image
                    .clone()
                    .map(|image| img(image).absolute().inset_0().size_full()),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _window, cx| {
                    #[cfg(target_os = "macos")]
                    crate::macos::focus_capture(_window);
                    this.selection.update(cx, |selection, cx| {
                        selection.start = Some(desktop_point(this.bounds, e.position));
                        selection.end = selection.start.unwrap();
                        cx.notify();
                    });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.selection
                        .update(cx, |selection, cx| selection.cancel(cx));
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                this.selection
                    .update(cx, |selection, cx| match e.keystroke.key.as_str() {
                        "escape" => selection.cancel(cx),
                        "enter" => selection.finish(cx),
                        _ => {}
                    });
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        // The pointer can be on another display while this
                        // overlay is active. Keep the capture cursor there too.
                        window.set_window_cursor_style(CursorStyle::Crosshair);
                        // Native drag events stay with the initiating window even when
                        // the pointer leaves it. A hitbox listener would drop them.
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _window, cx| {
                            #[cfg(target_os = "macos")]
                            if phase == DispatchPhase::Bubble && e.pressed_button.is_none() {
                                crate::macos::focus_capture_under_pointer(_window);
                            }
                            if phase == DispatchPhase::Bubble
                                && e.pressed_button == Some(MouseButton::Left)
                            {
                                #[cfg(target_os = "macos")]
                                if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                                    crate::macos::trace_capture_cursor(_window);
                                }
                                drag.update(cx, |selection, cx| {
                                    selection.move_to(
                                        desktop_point(bounds, e.position),
                                        e.modifiers.shift,
                                        cx,
                                    )
                                });
                            }
                        });
                        window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                                release.update(cx, |selection, cx| {
                                    selection.move_to(
                                        desktop_point(bounds, e.position),
                                        e.modifiers.shift,
                                        cx,
                                    );
                                    selection.finish(cx);
                                });
                            }
                        });
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            );
        let cuts = selected.into_iter().collect::<Vec<_>>();
        for dim in capture_overlay::subtract_rectangles(bounds, &cuts) {
            overlay = overlay.child(
                div()
                    .absolute()
                    .left(dim.left() - bounds.left())
                    .top(dim.top() - bounds.top())
                    .w(dim.size.width)
                    .h(dim.size.height)
                    .bg(rgba(0x00000065)),
            );
        }
        if let Some(rect) = selected.filter(|rect| overlaps(*rect, bounds)) {
            overlay = overlay
                .child(
                    div()
                        .absolute()
                        .left(rect.left() - bounds.left())
                        .top(rect.top() - bounds.top())
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .border_1()
                        .border_color(rgb(0xffffff)),
                )
                .child(
                    div()
                        .absolute()
                        .left((rect.left() - bounds.left()).max(px(8.0)))
                        .top((rect.top() - bounds.top() - px(35.0)).max(px(8.0)))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgba(0x202020e8))
                        .text_color(rgb(0xffffff))
                        .text_sm()
                        .child(format!(
                            "{:.0} × {:.0} px",
                            f32::from(rect.size.width) * scale,
                            f32::from(rect.size.height) * scale
                        )),
                );
        }
        overlay.child(
            div()
                .absolute()
                .bottom(px(36.0))
                .left(bounds.size.width / 2.0 - px(185.0))
                .px_4()
                .py_2()
                .rounded_lg()
                .bg(rgba(0x202020e8))
                .text_color(rgb(0xffffff))
                .text_sm()
                .child("Drag to capture an area · Esc to cancel"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "ui-tests")]
    use std::prelude::v1::test;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    fn screen(id: u32, bounds: Bounds<Pixels>, scale: f32) -> Screen {
        let width = (f32::from(bounds.size.width) * scale) as u32;
        let height = (f32::from(bounds.size.height) * scale) as u32;
        Screen {
            frame: Some(PixelsInRegion {
                image: RgbaImage::from_fn(width, height, |x, y| {
                    Rgba([id as u8, x as u8, y as u8, 255])
                }),
                bounds,
            }),
            layout: DisplayLayout {
                id,
                bounds,
                pixels: (width, height),
            },
            display_id: None,
            window_bounds: bounds,
            #[cfg(target_os = "macos")]
            overlay_id: None,
        }
    }

    #[cfg(target_os = "macos")]
    #[::core::prelude::v1::test]
    fn mac_desktop_origin_is_independent_of_gpui_display_local_bounds() {
        let external = screen(2, rect(-476.0, -1692.0, 3008.0, 1692.0), 1.0);
        let placement = rect(0.0, 0.0, 3008.0, 1692.0);
        let frame = CapturedFrame {
            image: external.frame.unwrap().image,
            origin: sniplet_platform::ScreenPoint { x: -476, y: -1692 },
            scale_factor: 1.0,
            source: CaptureSource::Monitor { index: 1, id: 2 },
        };
        assert_eq!(desktop_bounds(&frame, placement), external.layout.bounds);
    }

    #[::core::prelude::v1::test]
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

    #[::core::prelude::v1::test]
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
    #[::core::prelude::v1::test]
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
                ((f32::from(region.left() - source.bounds.left())) * scale).round() as u32,
                ((f32::from(region.top() - source.bounds.top())) * scale).round() as u32,
                (f32::from(region.size.width) * scale).round() as u32,
                (f32::from(region.size.height) * scale).round() as u32,
            )
            .to_image();
            screen.frame = Some(PixelsInRegion {
                image,
                bounds: region,
            });
        }
        assert_eq!(compose(&screens, bounds).unwrap(), expected);
    }

    #[::core::prelude::v1::test]
    fn repeat_accepts_reordered_displays_but_rejects_a_changed_layout() {
        let mut screens = vec![
            screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
            screen(2, rect(100.0, 0.0, 100.0, 100.0), 1.0),
        ];
        let saved = region(&screens, rect(80.0, 20.0, 40.0, 60.0));
        screens.swap(0, 1);
        assert!(layout_matches(&saved, &screens));
        screens[0].layout.bounds.origin.x = px(-100.0);
        assert!(!layout_matches(&saved, &screens));
    }

    #[cfg(feature = "ui-tests")]
    fn editor(cx: &mut App) -> (AnyWindowHandle, Entity<Editor>) {
        gpui_kit::init(cx);
        runtime::install_test_services(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Editor::new(
                    Some(Document::new(RgbaImage::new(80, 60))),
                    sniplet_platform::Settings {
                        auto_copy: false,
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn drag_continues_outside_the_starting_display_in_both_directions(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt;
        let (owner, editor) = cx.update(editor);
        for (index, start, end) in [
            (0, point(px(60.0), px(40.0)), point(px(120.0), px(80.0))),
            (1, point(px(20.0), px(80.0)), point(px(-40.0), px(40.0))),
        ] {
            let selection = cx.update(|cx| {
                let escape = runtime::begin_escape_session(cx);
                open(
                    vec![
                        screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
                        screen(2, rect(100.0, 0.0, 100.0, 100.0), 1.0),
                    ],
                    editor.clone(),
                    owner,
                    Command::Area,
                    escape,
                    cx,
                )
                .unwrap()
            });
            let windows = cx.update(|cx| selection.read(cx).windows.clone());
            cx.update_window(windows[index], |_, window, cx| {
                window.render_frame(cx);
                window.drag(start, end, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                let editor = editor.read(cx);
                let image = editor.document.as_ref().unwrap().original();
                assert_eq!(image.dimensions(), (120, 80));
                assert_eq!(image.get_pixel(0, 0), &Rgba([1, 120, 80, 255]));
                assert_eq!(image.get_pixel(119, 79), &Rgba([2, 19, 79, 255]));
                assert!(editor.last_capture.is_some());
                assert_eq!(cx.active_window(), Some(owner));
            });
            assert!(windows.iter().all(|window| !cx.windows().contains(window)));
        }
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn escape_on_either_display_closes_the_entire_selection(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt;
        let (owner, editor) = cx.update(editor);
        for index in 0..2 {
            let selection = cx.update(|cx| {
                let escape = runtime::begin_escape_session(cx);
                open(
                    vec![
                        screen(1, rect(0.0, 0.0, 100.0, 100.0), 2.0),
                        screen(2, rect(0.0, -100.0, 100.0, 100.0), 2.0),
                    ],
                    editor.clone(),
                    owner,
                    Command::Area,
                    escape,
                    cx,
                )
                .unwrap()
            });
            let windows = cx.update(|cx| selection.read(cx).windows.clone());
            cx.update_window(windows[index], |_, window, cx| {
                window.render_frame(cx);
                window.press("escape", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                assert!(selection.read(cx).escape.is_cancelled());
                assert_eq!(
                    editor
                        .read(cx)
                        .document
                        .as_ref()
                        .unwrap()
                        .original()
                        .dimensions(),
                    (80, 60)
                );
                assert!(editor.read(cx).last_capture.is_none());
                assert_ne!(cx.active_window(), Some(owner));
            });
            assert!(windows.iter().all(|window| !cx.windows().contains(window)));
        }
    }
}
