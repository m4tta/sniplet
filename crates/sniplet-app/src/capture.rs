use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{prelude::*, *};
use sniplet_core::{Document, ImageRect, Point as ImagePoint};

use crate::editor::{Command, Editor, display_image};

pub struct CaptureRequest {
    pub command: Command,
    pub monitor: usize,
    pub region: Option<ImageRect>,
    pub last_capture: Option<crate::area_capture::Region>,
}

#[derive(Debug, PartialEq, Eq)]
enum CaptureTarget {
    Monitor(usize),
    UnderPointer,
}

impl CaptureTarget {
    fn for_command(command: Command, monitor: usize) -> Self {
        if matches!(command, Command::Screen | Command::Delayed) {
            Self::UnderPointer
        } else {
            Self::Monitor(monitor)
        }
    }
}

pub fn start(
    request: CaptureRequest,
    editor: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> Result<(), String> {
    if matches!(
        request.command,
        Command::Area | Command::AddCapture | Command::CaptureOcr | Command::Repeat
    ) {
        return crate::area_capture::start(
            request.command,
            request.last_capture,
            editor,
            window,
            cx,
        );
    }
    let capture_started = Instant::now();
    let trace_capture = std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some();
    let CaptureRequest {
        command,
        monitor,
        region,
        ..
    } = request;
    let handle = window.window_handle();
    let escape = crate::runtime::begin_escape_session(cx);
    let manual_region = if matches!(command, Command::ManualScroll) {
        region
    } else {
        None
    };
    let target = CaptureTarget::for_command(command, monitor);
    let hide_delay = crate::runtime::hide_for_capture(window);
    cx.spawn(async move |_, cx| {
        if trace_capture { eprintln!("capture: task started in {:?}", capture_started.elapsed()); }
        let delay = if matches!(command, Command::Delayed) { Duration::from_secs(3) } else { hide_delay };
        if !delay.is_zero() { cx.background_executor().timer(delay).await; }
        if escape.is_cancelled() {
            cx.update(|cx| {
                crate::runtime::end_escape_session(&escape, cx);
            });
            return;
        }
        let frame = cx.background_executor().spawn(async move {
            if trace_capture { eprintln!("capture: backend started in {:?}", capture_started.elapsed()); }
            match target {
                CaptureTarget::UnderPointer => sniplet_platform::capture_monitor_under_pointer(),
                CaptureTarget::Monitor(index) => sniplet_platform::capture_monitor(index),
            }
        }).await;
        if trace_capture { eprintln!("capture: pixels ready in {:?}", capture_started.elapsed()); }
        cx.update(|cx| {
            if escape.is_cancelled() {
                crate::runtime::end_escape_session(&escape, cx);
                return;
            }
            match frame {
            Ok(frame) => {
                if let Some(rect) = manual_region {
                    let image = crop_image(&frame.image, rect);
                    editor.update(cx, |this, cx| {
                        this.scroll_frames.push(image);
                        match sniplet_core::stitch_vertical(&this.scroll_frames, Default::default()) {
                            Ok(image) => {
                                let count = this.scroll_frames.len();
                                copy_capture_if_enabled(this, &image);
                                this.load(Document::new(image), &format!("Scrolling capture · {count} frames · scroll, then choose Append scrolling frame from the Sniplet menu"), cx);
                                this.set_measure_scale(frame.scale_factor);
                            },
                            Err(error) => { this.scroll_frames.pop(); this.status = format!("Could not stitch frame: {error}. Scroll less so frames overlap."); cx.notify(); },
                        }
                    });
                    crate::runtime::end_escape_session(&escape, cx);
                    restore(handle, cx);
                } else if matches!(command, Command::Screen | Command::Delayed) {
                    if trace_capture {
                        eprintln!("capture: screen source={:?}; image={:?}; origin={:?}; scale={}", frame.source, frame.image.dimensions(), frame.origin, frame.scale_factor);
                    }
                    editor.update(cx, |this, cx| {
                        copy_capture_if_enabled(this, &frame.image);
                        this.load(Document::new(frame.image), "Screen captured", cx);
                        this.set_measure_scale(frame.scale_factor);
                    });
                    crate::runtime::end_escape_session(&escape, cx);
                    restore(handle, cx);
                    if trace_capture {
                        eprintln!("capture: screen editor loaded in {:?}", capture_started.elapsed());
                        let _ = handle.update(cx, |_, window, _| {
                            window.on_next_frame(move |_, _| eprintln!("capture: screen first frame in {:?}", capture_started.elapsed()));
                        });
                    }
                } else {
                    let (bounds, display_id) =
                        crate::capture_overlay::captured_frame_placement(&frame, cx);
                    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), titlebar: None, kind: WindowKind::PopUp,
                        is_movable: false, is_resizable: false, is_minimizable: false, display_id, show: !cfg!(target_os = "macos"), ..Default::default() };
                    let overlay_escape = escape.clone();
                    match gpui_kit::open_window(options, cx, |window, cx| {
                        #[cfg(target_os = "macos")]
                        crate::macos::prepare_capture_overlay(window, display_id);
                        window.set_window_title("Sniplet — Capture area");
                        cx.new(|cx| Overlay::new(frame, editor, handle, command, overlay_escape, window, cx))
                    }) {
                        Ok((overlay, _)) => {
                            crate::runtime::attach_escape_window(&escape, overlay, cx);
                            let _ = overlay.update(cx, |_, window, _| window.activate_window());
                            if trace_capture {
                                let _ = overlay.update(cx, |_, window, _| {
                                    eprintln!("capture: overlay created in {:?}; requested={bounds:?}; actual={:?}; viewport={:?}", capture_started.elapsed(), window.bounds(), window.viewport_size());
                                    #[cfg(target_os = "macos")]
                                    crate::macos::trace_capture_geometry(window);
                                    window.on_next_frame(move |_, _| eprintln!("capture: next display frame in {:?}", capture_started.elapsed()));
                                });
                            }
                        }
                        Err(error) => {
                            eprintln!("Could not open capture overlay: {error}");
                            crate::runtime::end_escape_session(&escape, cx);
                            restore(handle, cx);
                        }
                    }
                }
            }
            Err(error) => {
                editor.update(cx, |this, cx| { this.status = error.to_string(); cx.notify(); });
                crate::runtime::end_escape_session(&escape, cx);
                restore(handle, cx);
            },
        }
        });
    }).detach();
    Ok(())
}

pub fn active_window(
    editor: &Editor,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> Result<(), String> {
    let windows = sniplet_platform::list_windows().map_err(|error| error.to_string())?;
    let active = windows
        .into_iter()
        .find(|candidate| {
            candidate.is_focused
                && (cfg!(target_os = "macos") || candidate.process_id != std::process::id())
        })
        .ok_or_else(|| "No focused window is available for capture".to_owned())?;
    let id = active.id;
    #[cfg(target_os = "macos")]
    let id = if active.process_id == std::process::id() {
        // xcap marks every window of the active Mac app as focused. A system
        // capture-status window can sit above the editor, so use its native ID.
        crate::macos::capture_window_id(window).unwrap_or(id)
    } else {
        id
    };
    self::window(id, editor, window, cx);
    Ok(())
}

pub fn window(id: u32, editor: &Editor, window: &mut Window, cx: &mut Context<Editor>) {
    let style = editor.settings.window_capture.clone();
    let editor = cx.entity();
    let handle = window.window_handle();
    let escape = crate::runtime::begin_escape_session(cx);
    let hide_delay = crate::runtime::hide_for_capture(window);
    cx.spawn(async move |_, cx| {
        if !hide_delay.is_zero() {
            cx.background_executor().timer(hide_delay).await;
        }
        if escape.is_cancelled() {
            cx.update(|cx| {
                crate::runtime::end_escape_session(&escape, cx);
            });
            return;
        }
        let frame = cx
            .background_executor()
            .spawn(async move { sniplet_platform::capture_window_with_background(id, &style) })
            .await;
        cx.update(|cx| {
            if escape.is_cancelled() {
                crate::runtime::end_escape_session(&escape, cx);
                return;
            }
            editor.update(cx, |this, cx| match frame {
                Ok((frame, warning)) => {
                    if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                        eprintln!(
                            "window capture: captured id={id}; image={:?}; origin={:?}",
                            frame.image.dimensions(),
                            frame.origin
                        );
                    }
                    copy_capture_if_enabled(this, &frame.image);
                    this.load(
                        Document::new(frame.image),
                        warning.as_deref().unwrap_or("Window captured"),
                        cx,
                    );
                    this.set_measure_scale(frame.scale_factor);
                }
                Err(error) => {
                    this.status = error.to_string();
                    cx.notify();
                }
            });
            crate::runtime::end_escape_session(&escape, cx);
            restore(handle, cx);
        });
    })
    .detach();
}

pub(crate) fn restore(handle: AnyWindowHandle, cx: &mut App) {
    let _ = handle.update(cx, |_, w, cx| crate::runtime::show_editor(w, cx));
}

pub(crate) fn copy_capture_if_enabled(editor: &Editor, image: &image::RgbaImage) {
    if editor.settings.auto_copy
        && let Ok(mut clipboard) = sniplet_platform::Clipboard::new()
    {
        let _ = clipboard.set_image(image);
    }
}

fn crop_image(image: &image::RgbaImage, rect: ImageRect) -> image::RgbaImage {
    let rect = rect.clipped(image.width(), image.height());
    let x = rect.x.floor() as u32;
    let y = rect.y.floor() as u32;
    let width = (rect.width.ceil() as u32).min(image.width() - x).max(1);
    let height = (rect.height.ceil() as u32).min(image.height() - y).max(1);
    image::imageops::crop_imm(image, x, y, width, height).to_image()
}

fn physical_capture_rect(rect: ImageRect, width: u32, height: u32) -> Option<ImageRect> {
    let rect = rect.clipped(width, height);
    let left = rect.x.floor();
    let top = rect.y.floor();
    let right = (rect.x + rect.width).ceil().min(width as f32);
    let bottom = (rect.y + rect.height).ceil().min(height as f32);
    (right > left && bottom > top).then(|| ImageRect::new(left, top, right - left, bottom - top))
}

pub(crate) fn square_end(start: ImagePoint, end: ImagePoint) -> ImagePoint {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let side = dx.abs().max(dy.abs());
    ImagePoint::new(start.x + side.copysign(dx), start.y + side.copysign(dy))
}

struct Overlay {
    source: image::RgbaImage,
    image: Arc<RenderImage>,
    editor: Entity<Editor>,
    editor_window: AnyWindowHandle,
    command: Command,
    monitor: usize,
    scale_factor: f32,
    escape: crate::runtime::EscapeSession,
    start: Option<ImagePoint>,
    end: ImagePoint,
    focus: FocusHandle,
    preserve_escape_on_release: bool,
}

impl Overlay {
    fn new(
        frame: sniplet_platform::CapturedFrame,
        editor: Entity<Editor>,
        editor_window: AnyWindowHandle,
        command: Command,
        escape: crate::runtime::EscapeSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let monitor = match frame.source {
            sniplet_platform::CaptureSource::Monitor { index, .. }
            | sniplet_platform::CaptureSource::MonitorRegion { index, .. } => index,
            sniplet_platform::CaptureSource::Window { .. } => {
                unreachable!("area overlays use monitor captures")
            }
        };
        let source = frame.image;
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.on_release(|overlay, cx| {
            if !overlay.preserve_escape_on_release {
                crate::runtime::cancel_escape_session(&overlay.escape, cx);
            }
        })
        .detach();
        Self {
            image: display_image(source.clone()),
            source,
            editor,
            editor_window,
            monitor,
            scale_factor: frame.scale_factor,
            command,
            escape,
            start: None,
            end: ImagePoint::default(),
            focus,
            preserve_escape_on_release: false,
        }
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.start else {
            return;
        };
        let size = window.viewport_size();
        let rect = ImageRect::from_corners(start, self.end);
        if rect.width < 2.0 || rect.height < 2.0 {
            return;
        }
        let sx = self.source.width() as f32 / f32::from(size.width);
        let sy = self.source.height() as f32 / f32::from(size.height);
        let rect = ImageRect::new(rect.x * sx, rect.y * sy, rect.width * sx, rect.height * sy);
        let Some(rect) = physical_capture_rect(rect, self.source.width(), self.source.height())
        else {
            return;
        };
        let image = crop_image(&self.source, rect);
        if matches!(self.command, Command::Scroll | Command::ScrollUp) {
            let settings = &self.editor.read(cx).settings;
            let mut options = sniplet_platform::AutomaticScrollOptions {
                max_frames: settings.scroll_max_frames.clamp(2, 200),
                settle_delay: Duration::from_millis(settings.scroll_settle_ms.clamp(100, 700)),
                ..Default::default()
            };
            if matches!(self.command, Command::ScrollUp) {
                options.scroll_clicks = -options.scroll_clicks;
            }
            let scale_factor = self.scale_factor;
            self.preserve_escape_on_release = true;
            crate::runtime::detach_escape_window(&self.escape, cx);
            let escape = self.escape.clone();
            let cancel = escape.cancellation().clone();
            let editor = self.editor.clone();
            let handle = self.editor_window;
            let monitor = self.monitor;
            window.remove_window();
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(Duration::from_millis(220)).await;
                let result = cx.background_executor().spawn(async move {
                    sniplet_platform::capture_scrolling_region(monitor, rect.x as u32, rect.y as u32, rect.width as u32, rect.height as u32, options, &cancel)
                }).await;
                cx.update(|cx| {
                    let cancelled = escape.is_cancelled();
                    crate::runtime::end_escape_session(&escape, cx);
                    if cancelled {
                        return;
                    }
                    editor.update(cx, |editor, cx| match result { Ok(capture) => {
                        copy_capture_if_enabled(editor, &capture.image);
                        editor.load(Document::new(capture.image), &format!("Scrolling capture · {} frames · {:?}", capture.frame_count, capture.stop), cx);
                        editor.set_measure_scale(scale_factor);
                    }, Err(error) => { editor.status = format!("Scrolling capture failed: {error}. Try manual scrolling from the Sniplet menu."); cx.notify(); } });
                    restore(handle, cx);
                });
            }).detach();
            return;
        }
        let command = self.command;
        let scroll = matches!(command, Command::Scroll | Command::ManualScroll);
        let append = matches!(command, Command::AddCapture);
        let recognize = matches!(command, Command::CaptureOcr);
        let editor = self.editor.clone();
        let scale_factor = self.scale_factor;
        let updated = self.editor_window.update(cx, |_, editor_window, cx| {
            editor.update(cx, |editor, cx| {
                copy_capture_if_enabled(editor, &image);
                if scroll {
                    editor.scroll_region = Some(rect);
                    editor.scroll_frames = vec![image.clone()];
                } else {
                    editor.scroll_region = None;
                    editor.scroll_frames.clear();
                }
                if append {
                    editor.add_image(image, "Added capture", true, cx);
                } else {
                    editor.load(
                        Document::new(image),
                        if scroll {
                            "Scrolling capture · scroll the source, then choose Append scrolling frame from the Sniplet menu"
                        } else {
                            "Area captured"
                        },
                        cx,
                    );
                    editor.set_measure_scale(scale_factor);
                    if recognize {
                        editor.command(Command::Ocr, editor_window, cx);
                    }
                }
            });
            crate::runtime::show_editor(editor_window, cx);
        });
        if updated.is_err() {
            return;
        }
        crate::runtime::end_escape_session(&self.escape, cx);
        window.remove_window();
    }

    fn cancel(&self, window: &mut Window, cx: &mut Context<Self>) {
        crate::runtime::cancel_escape_session(&self.escape, cx);
        window.remove_window();
    }
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let width = f32::from(size.width);
        let height = f32::from(size.height);
        let rect = self
            .start
            .map(|start| ImageRect::from_corners(start, self.end));
        let mut overlay = div()
            .id("capture-overlay")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .cursor_crosshair()
            .child(img(self.image.clone()).absolute().inset_0().size_full())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.start = Some(ImagePoint::new(e.position.x.into(), e.position.y.into()));
                    this.end = this.start.unwrap();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                let end = ImagePoint::new(e.position.x.into(), e.position.y.into());
                this.end = if e.modifiers.shift {
                    this.start.map_or(end, |start| square_end(start, end))
                } else {
                    end
                };
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| this.finish(w, cx)),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" {
                    this.cancel(w, cx);
                } else if e.keystroke.key == "enter" {
                    this.finish(w, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
            );
        if let Some(rect) = rect {
            for (x, y, w, h) in [
                (0.0, 0.0, width, rect.y),
                (
                    0.0,
                    rect.y + rect.height,
                    width,
                    height - rect.y - rect.height,
                ),
                (0.0, rect.y, rect.x, rect.height),
                (
                    rect.x + rect.width,
                    rect.y,
                    width - rect.x - rect.width,
                    rect.height,
                ),
            ] {
                overlay = overlay.child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w.max(0.0)))
                        .h(px(h.max(0.0)))
                        .bg(rgba(0x00000065)),
                );
            }
            overlay = overlay
                .child(
                    div()
                        .absolute()
                        .left(px(rect.x))
                        .top(px(rect.y))
                        .w(px(rect.width))
                        .h(px(rect.height))
                        .border_1()
                        .border_color(rgb(0xffffff)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(rect.x))
                        .top(px((rect.y - 35.0).max(8.0)))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgba(0x202020e8))
                        .text_color(rgb(0xffffff))
                        .text_sm()
                        .child(format!(
                            "{:.0} × {:.0} px",
                            rect.width * self.source.width() as f32 / width,
                            rect.height * self.source.height() as f32 / height
                        )),
                );
        } else {
            overlay = overlay.child(div().absolute().inset_0().bg(rgba(0x00000045)));
        }
        overlay.child(
            div()
                .absolute()
                .bottom(px(36.0))
                .left(px(width / 2.0 - 185.0))
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

pub fn pin(image: image::RgbaImage, window: &mut Window, cx: &mut Context<Editor>) {
    let aspect = image.height() as f32 / image.width() as f32;
    let bounds = Bounds::centered(None, size(px(420.0), px((420.0 * aspect).min(650.0))), cx);
    let editor = cx.entity();
    let editor_window = window.window_handle();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        kind: WindowKind::PopUp,
        is_resizable: false,
        is_minimizable: false,
        app_owns_titlebar_drag: true,
        window_background: WindowBackgroundAppearance::Transparent,
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    };
    let _ = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Pinned::new(image, editor, editor_window, window, cx))
    });
}

struct Pinned {
    source: image::RgbaImage,
    image: Arc<RenderImage>,
    editor: Entity<Editor>,
    editor_window: AnyWindowHandle,
    focus: FocusHandle,
    opacity: f32,
    hovered: bool,
}

impl Pinned {
    fn new(
        source: image::RgbaImage,
        editor: Entity<Editor>,
        editor_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            image: display_image(source.clone()),
            source,
            editor,
            editor_window,
            focus,
            opacity: 1.0,
            hovered: false,
        }
    }

    fn adjust_opacity(&mut self, amount: f32, cx: &mut Context<Self>) {
        self.opacity = pin_opacity(self.opacity, amount);
        cx.notify();
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.source.clone();
        let editor = self.editor.clone();
        if self
            .editor_window
            .update(cx, |_, editor_window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.load(Document::new(source), "Pinned image opened for editing", cx);
                });
                crate::runtime::show_editor(editor_window, cx);
            })
            .is_ok()
        {
            window.remove_window();
        }
    }
}

fn pin_opacity(current: f32, amount: f32) -> f32 {
    ((current + amount) * 10.0).round().clamp(2.0, 10.0) / 10.0
}

fn pin_size_after_scroll(current: Size<Pixels>, amount: f32) -> Size<Pixels> {
    let width = f32::from(current.width).max(1.0);
    let height = f32::from(current.height).max(1.0);
    let desired = (amount * 0.06).exp();
    let minimum = (80.0 / width).max(60.0 / height);
    let maximum = (1600.0 / width).min(1200.0 / height);
    let factor = desired.clamp(minimum.min(maximum), maximum.max(minimum));
    size(px(width * factor), px(height * factor))
}

impl Render for Pinned {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .id("pinned-image")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .child(
                img(self.image.clone())
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .opacity(self.opacity),
            )
            .on_mouse_down(MouseButton::Left, |_, w, _| w.start_window_move())
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, w, _| {
                if e.keystroke.key == "escape" {
                    w.remove_window();
                }
            }))
            .on_mouse_down(MouseButton::Right, |_, w, _| w.remove_window())
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let amount = match event.delta {
                    ScrollDelta::Lines(p) => p.y,
                    ScrollDelta::Pixels(p) => f32::from(p.y) / 30.0,
                };
                if event.modifiers.secondary() {
                    this.adjust_opacity(amount.signum() * 0.1, cx);
                } else {
                    window.resize(pin_size_after_scroll(window.viewport_size(), amount));
                }
                cx.stop_propagation();
            }));

        if self.hovered {
            root = root.child(
                div()
                    .absolute()
                    .top(px(10.0))
                    .right(px(10.0))
                    .flex()
                    .items_center()
                    .gap_1()
                    .p_1()
                    .rounded_lg()
                    .bg(rgba(0x18181be8))
                    .shadow_lg()
                    .text_color(rgb(0xffffff))
                    .text_sm()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .id("pin-opacity-down")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(0xffffff28)))
                            .child("−")
                            .on_click(cx.listener(|this, _, _, cx| this.adjust_opacity(-0.1, cx))),
                    )
                    .child(
                        div()
                            .w(px(42.0))
                            .text_center()
                            .child(format!("{:.0}%", self.opacity * 100.0)),
                    )
                    .child(
                        div()
                            .id("pin-opacity-up")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(0xffffff28)))
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| this.adjust_opacity(0.1, cx))),
                    )
                    .child(
                        div()
                            .id("pin-edit")
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(28.0))
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(0xffffff28)))
                            .child("✎ Edit")
                            .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
                    ),
            );
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // GPUI's test macro expands to Rust's built-in test attribute.
    #[cfg(feature = "ui-tests")]
    use std::prelude::v1::test;

    #[::core::prelude::v1::test]
    fn screen_capture_uses_pointer_instead_of_cached_primary_display() {
        for command in [Command::Screen, Command::Delayed] {
            assert_eq!(
                CaptureTarget::for_command(command, 0),
                CaptureTarget::UnderPointer
            );
        }
        assert_eq!(
            CaptureTarget::for_command(Command::ManualScroll, 2),
            CaptureTarget::Monitor(2)
        );
    }

    #[cfg(feature = "ui-tests")]
    fn monitor_frame(image: image::RgbaImage) -> sniplet_platform::CapturedFrame {
        sniplet_platform::CapturedFrame {
            image,
            origin: sniplet_platform::ScreenPoint { x: 0, y: 0 },
            scale_factor: 1.0,
            source: sniplet_platform::CaptureSource::Monitor { index: 0, id: 7 },
        }
    }

    #[::core::prelude::v1::test]
    fn pin_opacity_and_resize_keep_their_user_facing_limits() {
        assert_eq!(pin_opacity(1.0, 0.1), 1.0);
        assert_eq!(pin_opacity(0.2, -0.1), 0.2);
        assert_eq!(pin_opacity(0.56, 0.1), 0.7);

        let resized = pin_size_after_scroll(size(px(400.0), px(200.0)), 2.0);
        let ratio = f32::from(resized.width) / f32::from(resized.height);
        assert!((ratio - 2.0).abs() < 0.001);
    }

    #[::core::prelude::v1::test]
    fn shift_constraint_uses_the_longest_drag_axis() {
        assert_eq!(
            square_end(ImagePoint::new(10.0, 10.0), ImagePoint::new(35.0, 20.0)),
            ImagePoint::new(35.0, 35.0)
        );
        assert_eq!(
            square_end(ImagePoint::new(10.0, 10.0), ImagePoint::new(5.0, -20.0)),
            ImagePoint::new(-20.0, -20.0)
        );
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn area_overlay_drag_produces_exact_source_pixels(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt;
        let (owner, editor) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Editor::new(
                        None,
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
        });
        let source = sniplet_core::demo_image(800, 600);
        let expected = image::imageops::crop_imm(&source, 100, 80, 230, 120).to_image();
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.0), px(0.0)),
                        size(px(800.0), px(600.0)),
                    ))),
                    titlebar: None,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Overlay::new(
                            monitor_frame(source),
                            editor.clone(),
                            owner,
                            Command::Area,
                            crate::runtime::EscapeSession::detached(),
                            window,
                            cx,
                        )
                    })
                },
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.drag(point(px(100.0), px(80.0)), point(px(330.0), px(200.0)), cx);
            assert_eq!(cx.active_window(), Some(owner));
            assert_eq!(
                editor.read(cx).document.as_ref().unwrap().original(),
                &expected
            );
        })
        .unwrap();
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn cancelling_overlay_keeps_the_existing_document(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt;
        let (owner, editor) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Some(Document::new(sniplet_core::demo_image(80, 60))),
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
        });
        let escape = crate::runtime::EscapeSession::detached();
        let observed_escape = escape.clone();
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Overlay::new(
                        monitor_frame(sniplet_core::demo_image(800, 600)),
                        editor.clone(),
                        owner,
                        Command::Area,
                        escape,
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            window.render_frame(cx);
            window.press("escape", cx);
            assert!(observed_escape.is_cancelled());
            assert_ne!(cx.active_window(), Some(owner));
            assert_eq!(editor.read(cx).document.as_ref().unwrap().width(), 80);
            assert_eq!(editor.read(cx).document.as_ref().unwrap().height(), 64);
        })
        .unwrap();
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn removing_overlay_releases_its_escape_session(cx: &mut TestAppContext) {
        let (owner, editor) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Some(Document::new(sniplet_core::demo_image(80, 60))),
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
        });
        let escape = crate::runtime::EscapeSession::detached();
        let observed_escape = escape.clone();
        let (overlay, _) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Overlay::new(
                        monitor_frame(sniplet_core::demo_image(800, 600)),
                        editor.clone(),
                        owner,
                        Command::Area,
                        escape,
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });

        cx.update_window(overlay, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();

        assert!(observed_escape.is_cancelled());
        cx.update(|cx| {
            assert_ne!(cx.active_window(), Some(owner));
            assert_eq!(editor.read(cx).document.as_ref().unwrap().width(), 80);
            assert_eq!(editor.read(cx).document.as_ref().unwrap().height(), 64);
        });
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn editing_a_pin_returns_the_unmodified_rgba_pixels(cx: &mut TestAppContext) {
        let (owner, editor) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Editor::new(
                        None,
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
        });
        let source = image::RgbaImage::from_pixel(3, 2, image::Rgba([12, 34, 56, 78]));
        let expected = source.clone();
        let (handle, pinned) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Pinned::new(source, editor.clone(), owner, window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            pinned.update(cx, |pinned, cx| pinned.edit(window, cx));
        })
        .unwrap();

        cx.update(|cx| {
            assert_eq!(
                editor.read(cx).document.as_ref().unwrap().original(),
                &expected
            );
        });
    }
}
