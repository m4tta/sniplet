//! Screen, window, delayed and scrolling captures, and pinned images. Area selection lives in
//! `area_capture`, and the window picker in `window_picker`.
use std::{sync::Arc, time::Duration};

use image::RgbaImage;
use rune::{
    Cx, El, Key, KeyEvent, Pixels, SurfaceFit, Tx, WheelEvent, WindowOptions, el, row, text,
};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use sniplet_core::{Document, ImageRect, Point as ImagePoint};

use crate::{
    capture_overlay::{self, WindowState},
    editor::{Command, Editor},
    shell::{self, EscapeSession},
    ui::{self, Inbox, icons},
};

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

/// What the capture worker hands back: the frame, a stitched manual scrolling capture, or
/// the screen as a document.
type Captured = (
    sniplet_platform::CapturedFrame,
    Option<Stitched>,
    Option<Document>,
);
type Stitched = (Vec<Arc<RgbaImage>>, sniplet_core::Result<Document>, u64);

/// Start the capture `request` names. Results reach the editor through its inbox.
pub fn start(request: CaptureRequest, editor: &mut Editor) -> Result<(), String> {
    let CaptureRequest {
        command,
        monitor,
        region,
        last_capture,
    } = request;
    if matches!(
        command,
        Command::Area | Command::AddCapture | Command::CaptureOcr | Command::Repeat
    ) {
        return crate::area_capture::start(command, last_capture, editor);
    }
    let escape = shell::begin_escape_session();
    let manual = region
        .filter(|_| matches!(command, Command::ManualScroll))
        .map(|rect| (rect, editor.scroll_frames.clone(), editor.load_revision));
    let target = CaptureTarget::for_command(command, monitor);
    let hide_delay = shell::hide_for_capture();
    let delay = if matches!(command, Command::Delayed) {
        Duration::from_secs(3)
    } else {
        hide_delay
    };
    let screen = matches!(command, Command::Screen | Command::Delayed);
    let session = escape.clone();
    editor.inbox.spawn(
        move || {
            std::thread::sleep(delay);
            if session.is_cancelled() {
                return None;
            }
            Some((|| {
                let mut frame = match target {
                    CaptureTarget::UnderPointer => sniplet_platform::capture_monitor_under_pointer(),
                    CaptureTarget::Monitor(index) => sniplet_platform::capture_monitor(index),
                }?;
                let stitched = manual.map(|(rect, mut frames, revision)| {
                    frames.push(Arc::new(crop_image(&frame.image, rect)));
                    let refs = frames.iter().map(Arc::as_ref).collect::<Vec<_>>();
                    let result = sniplet_core::stitch_vertical_refs(&refs, Default::default())
                        .map(Document::new);
                    (frames, result, revision)
                });
                let document = screen.then(|| Document::new(std::mem::take(&mut frame.image)));
                Ok::<Captured, sniplet_platform::PlatformError>((frame, stitched, document))
            })())
        },
        move |editor, result, _| {
            let Some(result) = result.filter(|_| !escape.is_cancelled()) else {
                shell::end_escape_session(&escape);
                return;
            };
            match result {
                Ok((frame, Some((frames, result, revision)), _)) => {
                    if editor.load_revision == revision {
                        match result {
                            Ok(document) => {
                                let count = frames.len();
                                editor.scroll_frames = frames;
                                copy_capture_if_enabled(editor, &document);
                                editor.load(
                                    document,
                                    &format!(
                                        "Scrolling capture · {count} frames · scroll, then choose Append scrolling frame from the Sniplet menu"
                                    ),
                                );
                                editor.set_measure_scale(frame.scale_factor);
                            }
                            Err(error) => {
                                editor.status = format!(
                                    "Could not stitch frame: {error}. Scroll less so frames overlap."
                                )
                            }
                        }
                    }
                }
                Ok((frame, None, Some(document))) => {
                    copy_capture_if_enabled(editor, &document);
                    editor.load(document, "Screen captured");
                    editor.set_measure_scale(frame.scale_factor);
                }
                Ok((frame, None, None)) => {
                    // Scrolling captures start by selecting the area that scrolls.
                    crate::area_capture::open_scroll(frame, command, &editor.inbox, &escape);
                    return;
                }
                Err(error) => editor.status = error.to_string(),
            }
            shell::end_escape_session(&escape);
            shell::show_editor();
        },
    );
    Ok(())
}

/// The area a scrolling capture selected, in pixels of the monitor image `source`.
pub(crate) struct ScrollSelection<'a> {
    pub command: Command,
    pub monitor: usize,
    pub scale_factor: f32,
    pub source: &'a RgbaImage,
    pub rect: ImageRect,
}

/// The scrolling area is selected: scroll and stitch it automatically, or keep the first
/// frame of a manual scrolling capture.
pub(crate) fn scroll_selected(
    selection: ScrollSelection,
    editor: &Inbox<Editor>,
    escape: &EscapeSession,
) {
    let ScrollSelection {
        command,
        monitor,
        scale_factor,
        source,
        rect,
    } = selection;
    let Some(rect) = physical_capture_rect(rect, source.width(), source.height()) else {
        return;
    };
    let escape = escape.clone();
    if !matches!(command, Command::Scroll | Command::ScrollUp) {
        let document = Document::new(crop_image(source, rect));
        editor.send(move |editor, _| {
            copy_capture_if_enabled(editor, &document);
            editor.scroll_region = Some(rect);
            editor.scroll_frames = vec![document.shared_original()];
            editor.load(
                document,
                "Scrolling capture · scroll the source, then choose Append scrolling frame from the Sniplet menu",
            );
            editor.set_measure_scale(scale_factor);
            shell::show_editor();
            shell::end_escape_session(&escape);
        });
        return;
    }
    // The overlays are closing; Escape still stops the scrolling.
    shell::detach_escape_window(&escape);
    editor.send(move |editor, _| {
        let settings = &editor.settings;
        let mut options = sniplet_platform::AutomaticScrollOptions {
            max_frames: settings.scroll_max_frames.clamp(2, 200),
            settle_delay: Duration::from_millis(settings.scroll_settle_ms.clamp(100, 700)),
            ..Default::default()
        };
        if matches!(command, Command::ScrollUp) {
            options.scroll_clicks = -options.scroll_clicks;
        }
        let cancel = escape.cancellation().clone();
        editor.inbox.spawn(
            move || {
                // Let the overlays leave the screen before the first frame.
                std::thread::sleep(Duration::from_millis(220));
                sniplet_platform::capture_scrolling_region(
                    monitor,
                    rect.x as u32,
                    rect.y as u32,
                    rect.width as u32,
                    rect.height as u32,
                    options,
                    &cancel,
                )
                .map(|capture| {
                    (
                        Document::new(capture.image),
                        capture.frame_count,
                        capture.stop,
                    )
                })
            },
            move |editor, result, _| {
                let cancelled = escape.is_cancelled();
                shell::end_escape_session(&escape);
                if cancelled {
                    return;
                }
                match result {
                    Ok((document, frame_count, stop)) => {
                        copy_capture_if_enabled(editor, &document);
                        editor.load(
                            document,
                            &format!("Scrolling capture · {frame_count} frames · {stop:?}"),
                        );
                        editor.set_measure_scale(scale_factor);
                    }
                    Err(error) => {
                        editor.status = format!(
                            "Scrolling capture failed: {error}. Try manual scrolling from the Sniplet menu."
                        )
                    }
                }
                shell::show_editor();
            },
        );
    });
}

/// Capture the focused window of another app.
pub fn active_window(editor: &mut Editor) -> Result<(), String> {
    let windows = sniplet_platform::list_windows().map_err(|error| error.to_string())?;
    let active = windows
        .into_iter()
        .find(|candidate| {
            candidate.is_focused
                && (cfg!(target_os = "macos") || candidate.process_id != std::process::id())
        })
        .ok_or_else(|| "No focused window is available for capture".to_owned())?;
    window(active.id, editor);
    Ok(())
}

/// Capture the window `id` with the window capture style from Settings.
pub(crate) fn window(id: u32, editor: &mut Editor) {
    let style = editor.settings.window_capture.clone();
    let escape = shell::begin_escape_session();
    let delay = shell::hide_for_capture();
    let session = escape.clone();
    editor.inbox.spawn(
        move || {
            std::thread::sleep(delay);
            (!session.is_cancelled()).then(|| {
                sniplet_platform::capture_window_with_background(id, &style).map(
                    |(frame, warning)| (Document::new(frame.image), frame.scale_factor, warning),
                )
            })
        },
        move |editor, result, _| {
            let Some(result) = result.filter(|_| !escape.is_cancelled()) else {
                shell::end_escape_session(&escape);
                return;
            };
            match result {
                Ok((document, scale_factor, warning)) => {
                    copy_capture_if_enabled(editor, &document);
                    editor.load(document, warning.as_deref().unwrap_or("Window captured"));
                    editor.set_measure_scale(scale_factor);
                }
                Err(error) => editor.status = error.to_string(),
            }
            shell::end_escape_session(&escape);
            shell::show_editor();
        },
    );
}

/// Copy a new capture's original pixels when Settings asks for it.
pub(crate) fn copy_capture_if_enabled(editor: &Editor, document: &Document) {
    if editor.settings.auto_copy {
        editor.inbox.spawn(
            crate::clipboard_jobs::copy(crate::clipboard_jobs::Source::Original(document.clone())),
            |_, _, _| {},
        );
    }
}

fn crop_image(image: &RgbaImage, rect: ImageRect) -> RgbaImage {
    let rect = rect.clipped(image.width(), image.height());
    let x = rect.x.floor() as u32;
    let y = rect.y.floor() as u32;
    let width = (rect.width.ceil() as u32).min(image.width() - x).max(1);
    let height = (rect.height.ceil() as u32).min(image.height() - y).max(1);
    image::imageops::crop_imm(image, x, y, width, height).to_image()
}

/// `rect` rounded outward to whole pixels inside a `width` × `height` image.
fn physical_capture_rect(rect: ImageRect, width: u32, height: u32) -> Option<ImageRect> {
    let rect = rect.clipped(width, height);
    let left = rect.x.floor();
    let top = rect.y.floor();
    let right = (rect.x + rect.width).ceil().min(width as f32);
    let bottom = (rect.y + rect.height).ceil().min(height as f32);
    (right > left && bottom > top).then(|| ImageRect::new(left, top, right - left, bottom - top))
}

/// The end of a Shift drag: a square along the longest drag axis.
pub(crate) fn square_end(start: ImagePoint, end: ImagePoint) -> ImagePoint {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let side = dx.abs().max(dy.abs());
    ImagePoint::new(start.x + side.copysign(dx), start.y + side.copysign(dy))
}

/// An image pinned on screen in its own always-on-top window. Drag moves it, the wheel
/// resizes it, Ctrl/Cmd and the wheel change its opacity, and Escape or a right-click
/// closes it.
pub(crate) struct Pinned {
    inbox: Inbox<Pinned>,
    editor: Inbox<Editor>,
    source: RgbaImage,
    pixels: Pixels,
    opacity: f32,
    hovered: bool,
    size: (f32, f32),
}

impl WindowState for Pinned {
    fn inbox(&self) -> &Inbox<Self> {
        &self.inbox
    }
}

impl Pinned {
    fn new(source: RgbaImage, editor: &Inbox<Editor>) -> Self {
        let pixels = Pixels::new();
        let (width, height, rgba) = crate::editor::display_pixels(&source);
        pixels.set(width, height, rgba);
        let aspect = source.height() as f32 / source.width().max(1) as f32;
        Self {
            inbox: Inbox::default(),
            editor: editor.clone(),
            source,
            pixels,
            opacity: 1.0,
            hovered: false,
            size: (420.0, (420.0 * aspect).min(650.0)),
        }
    }

    fn adjust_opacity(&mut self, amount: f32) {
        self.opacity = pin_opacity(self.opacity, amount);
    }

    /// Open the pinned pixels in the editor and close the pin.
    fn edit(&mut self, tx: &mut Tx) {
        let source = self.source.clone();
        self.editor.send(move |editor, _| {
            editor.load(Document::new(source), "Pinned image opened for editing");
            shell::show_editor();
        });
        tx.close_window();
    }

    fn wheel(&mut self, event: WheelEvent, tx: &mut Tx) {
        // A wheel notch is 40 points; a trackpad scrolls by its own points.
        let amount = event.dy / if event.precise { 30.0 } else { 40.0 };
        if event.mods.cmd || event.mods.ctrl {
            self.adjust_opacity(amount.signum() * 0.1);
        } else {
            self.size = pin_size_after_scroll(self.size, amount);
            tx.resize_window_instant(self.size);
        }
    }
}

/// Pin `image` on screen in its own always-on-top window.
pub fn pin(image: RgbaImage, editor: &Inbox<Editor>) {
    let pinned = Pinned::new(image, editor);
    let window = capture_overlay::open(
        capture_overlay::Window {
            title: "Sniplet — Pinned image",
            size: pinned.size,
            view: pin_view,
            on_key: pin_key,
            options: WindowOptions {
                transparent: true,
                borderless: true,
                always_on_top: true,
                resizable: false,
                min_size: Some((40.0, 30.0)),
                ..Default::default()
            },
        },
        pinned,
        |_: &rune::WindowHandle| {},
    );
    window.center();
}

fn pin_opacity(current: f32, amount: f32) -> f32 {
    ((current + amount) * 10.0).round().clamp(2.0, 10.0) / 10.0
}

fn pin_size_after_scroll(current: (f32, f32), amount: f32) -> (f32, f32) {
    let width = current.0.max(1.0);
    let height = current.1.max(1.0);
    let desired = (amount * 0.06).exp();
    let minimum = (80.0 / width).max(60.0 / height);
    let maximum = (1600.0 / width).min(1200.0 / height);
    let factor = desired.clamp(minimum.min(maximum), maximum.max(minimum));
    (width * factor, height * factor)
}

fn pin_key(_: &mut Pinned, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.key != Key::Escape {
        return false;
    }
    tx.close_window();
    true
}

fn pin_view(s: &Pinned, _: &mut Cx<Pinned>) -> El {
    let root = el()
        .key("pinned-image")
        .w_full()
        .h_full()
        .on_layout(|s: &mut Pinned, rect: [f32; 4]| s.size = (rect[2], rect[3]))
        .on_hover_change(|s: &mut Pinned, inside: bool, _: &mut Tx| s.hovered = inside)
        .on_wheel(|s: &mut Pinned, event: WheelEvent, tx: &mut Tx| {
            s.wheel(event, tx);
            true
        })
        .on_secondary_press(|_: &mut Pinned, _, tx: &mut Tx| tx.close_window())
        .child(
            el().w_full()
                .h_full()
                .window_drag()
                .pixels_fit(&s.pixels, SurfaceFit::Fit)
                .opacity(s.opacity),
        );
    if !s.hovered {
        return root;
    }
    // The toolbar floats over any image, so it keeps the dark kit in both appearances.
    let k = ui::hud();
    let step = |key: &str, glyph, label: &str, by: f32| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::Ghost)
            .label(label)
            .on_press(move |s: &mut Pinned| s.adjust_opacity(by))
            .build(&k)
            .key(key)
    };
    root.child(
        ui::surface::<Pinned>(&k, row().center_y().gap(2.0).padding(4.0))
            .key("pin-toolbar")
            .pin(None, Some(10.0), Some(10.0), None)
            .child(step("pin-opacity-down", &icons::MINUS, "Less opaque", -0.1))
            .child(
                el().w(42.0).center().child(
                    text(format!("{:.0}%", s.opacity * 100.0))
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .single_line(),
                ),
            )
            .child(step("pin-opacity-up", &icons::PLUS, "More opaque", 0.1))
            .child(
                Button::new("Edit")
                    .variant(ButtonVariant::Ghost)
                    .icon(&icons::PENCIL)
                    .on_press_tx(|s: &mut Pinned, tx| s.edit(tx))
                    .build(&k)
                    .key("pin-edit"),
            ),
    )
}

#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use super::*;

    #[test]
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

    #[test]
    fn pin_opacity_and_resize_keep_their_user_facing_limits() {
        assert_eq!(pin_opacity(1.0, 0.1), 1.0);
        assert_eq!(pin_opacity(0.2, -0.1), 0.2);
        assert_eq!(pin_opacity(0.56, 0.1), 0.7);

        let resized = pin_size_after_scroll((400.0, 200.0), 2.0);
        assert!((resized.0 / resized.1 - 2.0).abs() < 0.001);
    }

    #[test]
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

    #[test]
    fn capture_rects_round_outward_to_whole_pixels_inside_the_image() {
        assert_eq!(
            physical_capture_rect(ImageRect::new(10.4, 5.6, 20.2, 10.0), 100, 100),
            Some(ImageRect::new(10.0, 5.0, 21.0, 11.0))
        );
        assert_eq!(
            physical_capture_rect(ImageRect::new(95.0, 0.0, 20.0, 10.0), 100, 100),
            Some(ImageRect::new(95.0, 0.0, 5.0, 10.0))
        );
        assert_eq!(
            physical_capture_rect(ImageRect::new(120.0, 0.0, 20.0, 10.0), 100, 100),
            None
        );
    }

    #[test]
    fn editing_a_pin_returns_the_unmodified_rgba_pixels_and_hover_shows_its_controls() {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let editor = Editor::new(None, Default::default(), 1.0);
        let mut editor = Runtime::new(editor, crate::editor::view, (1280.0, 850.0), env);
        let source = RgbaImage::from_pixel(3, 2, image::Rgba([12, 34, 56, 78]));
        let mut pin = Runtime::new(
            Pinned::new(source.clone(), &editor.state.inbox),
            pin_view,
            (420.0, 280.0),
            env,
        );
        pin.frame(0.0);
        assert!(pin.find("pin-edit").is_none());
        pin.pointer_move(200.0, 140.0);
        pin.frame(1.0 / 60.0);
        let (x, y) = pin
            .find("pin-opacity-down")
            .expect("hover shows the controls");
        pin.pointer_down(x, y);
        pin.pointer_up(x, y);
        pin.frame(1.0 / 60.0);
        assert_eq!(pin.state.opacity, 0.9);

        let (x, y) = pin.find("pin-edit").unwrap();
        pin.pointer_down(x, y);
        pin.pointer_up(x, y);
        pin.frame(1.0 / 60.0);
        editor.update(|editor, tx| {
            Inbox::drain(editor, |editor| &editor.inbox, tx);
        });
        assert_eq!(editor.state.document.as_ref().unwrap().original(), &source);
    }
}
