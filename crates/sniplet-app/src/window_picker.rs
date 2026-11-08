//! The window capture picker: a frozen image of every monitor that highlights the frontmost
//! window under the pointer. A click captures that window; Escape or a right-click cancels.
use std::{cell::RefCell, rc::Rc};

use rune::{
    Color, Cursor, Cx, El, Key, KeyEvent, Pixels, PointerPress, Tx, WindowHandle, el, text,
};
use sniplet_platform::{CapturedFrame, ScreenPoint, WindowInfo};

use crate::{
    capture_overlay::{self, Monitor, Rect, WindowState, subtract_rectangles},
    editor::Editor,
    shell::{self, EscapeSession},
    ui::{BODY, Inbox, rgb},
};

/// What a click on a window does with its ID.
type CaptureWindow = fn(u32, &Inbox<Editor>);

/// The picking every monitor's overlay shares.
struct Selection {
    windows: Vec<WindowHandle>,
    overlays: Vec<Inbox<Picker>>,
    hovered: Option<u32>,
    active_screen: Option<usize>,
    completed: bool,
    escape: EscapeSession,
    editor: Inbox<Editor>,
    capture: CaptureWindow,
}

impl Selection {
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

    /// Close every overlay, then capture `id`. The capture hides nothing more, and starts
    /// once the overlays have left the screen.
    fn finish(&mut self, id: u32) {
        if self.completed || self.escape.is_cancelled() {
            return;
        }
        shell::detach_escape_window(&self.escape);
        shell::end_escape_session(&self.escape);
        self.close();
        (self.capture)(id, &self.editor);
    }

    fn cancel(&mut self) {
        if self.completed {
            return;
        }
        shell::cancel_escape_session(&self.escape);
        self.close();
    }
}

fn start_capture(id: u32, editor: &Inbox<Editor>) {
    editor.send(move |editor, _| crate::capture::window(id, editor));
}

/// One monitor's picker window.
pub(crate) struct Picker {
    inbox: Inbox<Picker>,
    selection: Rc<RefCell<Selection>>,
    screen: usize,
    bounds: Rect,
    viewport: (f32, f32),
    pixels: Pixels,
    windows: Rc<Vec<WindowInfo>>,
    own_process_id: u32,
    pointer: Option<(f32, f32)>,
}

impl WindowState for Picker {
    fn inbox(&self) -> &Inbox<Self> {
        &self.inbox
    }
}

impl Drop for Picker {
    /// A picker closed by anything but a click cancels the capture.
    fn drop(&mut self) {
        if let Ok(mut selection) = self.selection.try_borrow_mut() {
            selection.cancel();
        }
    }
}

impl Picker {
    fn window_at(&self, x: f32, y: f32) -> Option<u32> {
        let point = capture_overlay::to_desktop(self.bounds, self.viewport, x, y);
        sniplet_platform::window_at_point(
            &self.windows,
            ScreenPoint {
                x: point.x.floor() as i32,
                y: point.y.floor() as i32,
            },
            self.own_process_id,
        )
        .map(|window| window.id)
    }

    fn hover(&mut self, x: f32, y: f32) {
        let hovered = self.window_at(x, y);
        self.pointer = Some((x, y));
        let mut selection = self.selection.borrow_mut();
        selection.hovered = hovered;
        selection.active_screen = Some(self.screen);
        selection.notify();
    }

    fn select_at(&mut self, x: f32, y: f32) {
        match self.window_at(x, y) {
            Some(id) => self.selection.borrow_mut().finish(id),
            None => self.hover(x, y),
        }
    }

    /// A window's frame on this overlay, in points.
    fn local(&self, window: &WindowInfo) -> Rect {
        capture_overlay::to_local(
            self.bounds,
            self.viewport,
            Rect::new(
                window.x as f32,
                window.y as f32,
                window.width as f32,
                window.height as f32,
            ),
        )
    }
}

/// Open the picker: hide the editor, then freeze every monitor and list its windows.
pub(crate) fn open(editor: &mut Editor) -> Result<(), String> {
    let escape = shell::begin_escape_session();
    let delay = shell::hide_for_capture();
    let session = escape.clone();
    editor.inbox.spawn(
        move || {
            std::thread::sleep(delay);
            (!session.is_cancelled()).then(|| {
                Ok::<_, sniplet_platform::PlatformError>((
                    sniplet_platform::capture_monitors()?,
                    sniplet_platform::list_windows()?,
                ))
            })
        },
        move |editor, snapshot, _| {
            let Some(snapshot) = snapshot.filter(|_| !escape.is_cancelled()) else {
                shell::end_escape_session(&escape);
                return;
            };
            let result =
                snapshot
                    .map_err(|error| error.to_string())
                    .and_then(|(frames, windows)| {
                        // Live Mac capture can include the editor; other desktops hide it before
                        // taking the snapshot.
                        let own_process_id = if cfg!(target_os = "macos") {
                            0
                        } else {
                            std::process::id()
                        };
                        let pickers = pickers(
                            frames,
                            windows,
                            own_process_id,
                            &editor.inbox,
                            &escape,
                            start_capture,
                        )?;
                        open_pickers(pickers, &escape);
                        Ok(())
                    });
            if let Err(error) = result {
                editor.status = format!("Could not start window capture: {error}");
                shell::end_escape_session(&escape);
                shell::show_editor();
            }
        },
    );
    Ok(())
}

/// One picker state per monitor around a shared selection.
fn pickers(
    frames: Vec<CapturedFrame>,
    windows: Vec<WindowInfo>,
    own_process_id: u32,
    editor: &Inbox<Editor>,
    escape: &EscapeSession,
    capture: CaptureWindow,
) -> Result<Vec<(Monitor, Picker)>, String> {
    if frames.is_empty() {
        return Err("No displays are available".into());
    }
    let windows = Rc::new(windows);
    let selection = Rc::new(RefCell::new(Selection {
        windows: Vec::new(),
        overlays: Vec::new(),
        hovered: None,
        active_screen: None,
        completed: false,
        escape: escape.clone(),
        editor: editor.clone(),
        capture,
    }));
    let pickers = frames
        .into_iter()
        .enumerate()
        .map(|(screen, frame)| {
            let monitor = Monitor::from_frame(&frame);
            let pixels = Pixels::new();
            let (width, height, rgba) = crate::editor::display_pixels(&frame.image);
            pixels.set(width, height, rgba);
            let picker = Picker {
                inbox: Inbox::default(),
                selection: selection.clone(),
                screen,
                bounds: monitor.bounds,
                viewport: capture_overlay::initial_viewport(&monitor, frame.scale_factor),
                pixels,
                windows: windows.clone(),
                own_process_id,
                pointer: None,
            };
            (monitor, picker)
        })
        .collect::<Vec<_>>();
    selection.borrow_mut().overlays = pickers.iter().map(|(_, p)| p.inbox.clone()).collect();
    Ok(pickers)
}

fn open_pickers(pickers: Vec<(Monitor, Picker)>, escape: &EscapeSession) {
    for (monitor, picker) in pickers {
        let selection = picker.selection.clone();
        let window = capture_overlay::open_overlay(
            &monitor,
            capture_overlay::Window {
                title: "Sniplet — Capture window",
                size: (monitor.bounds.w, monitor.bounds.h),
                view,
                on_key,
                options: monitor.window_options(false),
            },
            picker,
        );
        shell::attach_escape_window(escape, window.clone());
        selection.borrow_mut().windows.push(window);
    }
}

fn on_key(s: &mut Picker, event: &KeyEvent, _: &mut Tx) -> bool {
    if event.key != Key::Escape {
        return false;
    }
    s.selection.borrow_mut().cancel();
    true
}

fn border_strips(bounds: Rect, thickness: f32) -> Vec<Rect> {
    let thickness = thickness.min(bounds.w / 2.0).min(bounds.h / 2.0);
    if thickness <= 0.0 {
        return Vec::new();
    }
    let vertical = (bounds.h - thickness * 2.0).max(0.0);
    let mut strips = vec![
        Rect::new(bounds.x, bounds.y, bounds.w, thickness),
        Rect::new(bounds.x, bounds.bottom() - thickness, bounds.w, thickness),
    ];
    if vertical > 0.0 {
        strips.extend([
            Rect::new(bounds.x, bounds.y + thickness, thickness, vertical),
            Rect::new(
                bounds.right() - thickness,
                bounds.y + thickness,
                thickness,
                vertical,
            ),
        ]);
    }
    strips
}

fn rect(r: Rect, color: Color) -> El {
    el().absolute(r.x, r.y).size(r.w, r.h).fill(color)
}

fn view(s: &Picker, _: &mut Cx<Picker>) -> El {
    let selection = s.selection.borrow();
    let (width, height) = s.viewport;
    let viewport = Rect::new(0.0, 0.0, width, height);
    let mut overlay = el()
        .key("window-capture-overlay")
        .w_full()
        .h_full()
        .clip_rounded(0.0)
        .cursor(Cursor::Crosshair)
        .on_layout(|s: &mut Picker, rect: [f32; 4]| s.viewport = (rect[2], rect[3]))
        .on_pointer_move(|s: &mut Picker, p: PointerPress| s.hover(p.x, p.y))
        .on_drag(|s: &mut Picker, drag: &mut rune::Drag, _: &mut Tx| {
            if drag.phase == rune::Phase::Down {
                s.select_at(drag.x, drag.y);
            }
        })
        .on_secondary_press(|s: &mut Picker, _, _| s.selection.borrow_mut().cancel())
        .child(el().absolute(0.0, 0.0).w_full().h_full().pixels(&s.pixels));
    let selected = selection
        .hovered
        .and_then(|id| s.windows.iter().find(|window| window.id == id))
        .map(|window| (window, s.local(window)))
        .filter(|(_, frame)| {
            let visible = frame.intersect(&viewport);
            visible.w >= 1.0 && visible.h >= 1.0
        });
    if let Some((selected, frame)) = selected {
        let bounds = frame.intersect(&viewport);
        let occluders = s
            .windows
            .iter()
            .filter(|window| {
                window.z_order > selected.z_order && window.is_capture_candidate(s.own_process_id)
            })
            .map(|window| s.local(window).intersect(&viewport))
            .filter(|bounds| !bounds.is_empty())
            .collect::<Vec<_>>();
        let visible = subtract_rectangles(bounds, &occluders);
        let mut dimmed = subtract_rectangles(viewport, &[bounds]);
        dimmed.extend(subtract_rectangles(bounds, &visible));
        for piece in dimmed {
            overlay = overlay.child(rect(piece, capture_overlay::dim(0x58)));
        }
        for piece in &visible {
            overlay = overlay.child(rect(*piece, rgb(0x168cf3).alpha(0x42 as f32 / 255.0)));
        }
        for strip in border_strips(frame, 2.0)
            .into_iter()
            .map(|strip| strip.intersect(&viewport))
            .filter(|strip| !strip.is_empty())
            .flat_map(|strip| subtract_rectangles(strip, &occluders))
        {
            overlay = overlay.child(rect(strip, rgb(0x3298ff)));
        }
        overlay = overlay.child(
            el().key(format!("window-capture-highlight-{}", selected.id))
                .absolute(bounds.x, bounds.y)
                .size(bounds.w, bounds.h),
        );
        let label = if selected.title.trim() == selected.app_name.trim() {
            selected.app_name.clone()
        } else {
            format!("{} · {}", selected.app_name, selected.title)
        };
        if let Some(room) = visible
            .iter()
            .copied()
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .filter(|room| room.w >= 100.0 && room.h >= 40.0)
        {
            overlay = overlay.child(
                el().absolute(room.x + 8.0, room.y + 8.0)
                    .max_w(room.w - 16.0)
                    .px(12.0)
                    .py(4.0)
                    .radius(6.0)
                    .fill(rgb(0x075ca8).alpha(0xeb as f32 / 255.0))
                    .child(
                        text(label)
                            .font(BODY)
                            .color(Color::WHITE)
                            .single_line()
                            .truncate(),
                    ),
            );
        }
    } else {
        overlay = overlay.child(rect(viewport, capture_overlay::dim(0x58)));
    }
    if let Some((x, y)) = s.pointer
        && selection.active_screen == Some(s.screen)
    {
        overlay = overlay.child(camera(x, y));
    }
    overlay.child(capture_overlay::hint(
        "Point at a window and click to capture · Esc to cancel",
    ))
}

/// A small camera beside the pointer.
fn camera(x: f32, y: f32) -> El {
    let body = rgb(0x111827).alpha(0xed as f32 / 255.0);
    el().key("window-capture-camera")
        .absolute(x + 12.0, y + 12.0)
        .size(26.0, 18.0)
        .radius(6.0)
        .border(1.0, Color::WHITE)
        .fill(body)
        .shadow(4.0, 12.0, Color::BLACK.alpha(0.3))
        .child(
            el().absolute(9.0, 5.0)
                .size(8.0, 8.0)
                .radius(4.0)
                .border(2.0, Color::WHITE),
        )
        .child(
            el().absolute(4.0, -4.0)
                .size(9.0, 5.0)
                .radius(2.0)
                .fill(body)
                .border(1.0, Color::WHITE),
        )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use rune::{Env, Runtime};
    use sniplet_core::Document;

    use super::*;

    thread_local! {
        static SELECTED: Cell<Option<u32>> = const { Cell::new(None) };
    }

    fn observe_selection(id: u32, _: &Inbox<Editor>) {
        SELECTED.set(Some(id));
    }

    fn frame(origin: (i32, i32), index: usize) -> CapturedFrame {
        CapturedFrame {
            image: sniplet_core::demo_image(800, 600),
            origin: ScreenPoint {
                x: origin.0,
                y: origin.1,
            },
            scale_factor: 1.0,
            source: sniplet_platform::CaptureSource::Monitor {
                index,
                id: 7 + index as u32,
            },
        }
    }

    fn fixture(id: u32, x: i32, y: i32, z_order: i32) -> WindowInfo {
        WindowInfo {
            id,
            process_id: 1000 + id,
            title: format!("Window {id}"),
            app_name: format!("App {id}"),
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

    /// Picker runtimes for 800 × 600 monitors at `origins`, in desktop units at 1×.
    fn open_test(
        origins: &[(i32, i32)],
        windows: Vec<WindowInfo>,
        escape: &EscapeSession,
    ) -> Vec<Runtime<Picker>> {
        SELECTED.set(None);
        let editor = Editor::new(
            Some(Document::new(sniplet_core::demo_image(400, 300))),
            Default::default(),
            1.0,
        );
        let frames = origins
            .iter()
            .enumerate()
            .map(|(index, origin)| frame(*origin, index))
            .collect();
        pickers(
            frames,
            windows,
            std::process::id(),
            &editor.inbox,
            escape,
            observe_selection,
        )
        .unwrap()
        .into_iter()
        .map(|(_, picker)| {
            let env = Env {
                system_dark: true,
                scale: 1.0,
            };
            let mut rt = Runtime::new(picker, view, (800.0, 600.0), env);
            rt.on_key = Some(on_key);
            rt.frame(0.0);
            rt
        })
        .collect()
    }

    /// Run queued input, then let every picker take what the others sent it.
    fn redraw(pickers: &mut [Runtime<Picker>]) {
        for picker in pickers.iter_mut() {
            picker.frame(1.0 / 60.0);
        }
        for picker in pickers {
            picker.update(|picker, tx| {
                Inbox::drain(picker, |picker| &picker.inbox, tx);
            });
            picker.frame(1.0 / 60.0);
        }
    }

    #[test]
    fn moving_across_overlapping_windows_highlights_the_frontmost() {
        let mut pickers = open_test(
            &[(-100, -50)],
            vec![fixture(10, 0, 0, 2), fixture(20, 40, 30, 8)],
            &EscapeSession::detached(),
        );
        pickers[0].pointer_move(170.0, 130.0);
        redraw(&mut pickers);
        assert!(pickers[0].find("window-capture-highlight-20").is_some());
        assert!(pickers[0].find("window-capture-camera").is_some());
        assert!(pickers[0].find("window-capture-highlight-10").is_none());
    }

    #[test]
    fn spanning_window_highlights_both_displays_with_one_camera() {
        let mut pickers = open_test(
            &[(-100, -50), (-100, -650)],
            vec![fixture(70, 40, -100, 1)],
            &EscapeSession::detached(),
        );
        pickers[1].pointer_move(150.0, 590.0);
        redraw(&mut pickers);
        for picker in &pickers {
            assert!(picker.find("window-capture-highlight-70").is_some());
        }
        assert!(pickers[0].find("window-capture-camera").is_none());
        assert!(pickers[1].find("window-capture-camera").is_some());
    }

    #[test]
    fn click_closes_every_display_before_signalling_window_capture() {
        let mut pickers = open_test(
            &[(-100, -50), (-500, -350)],
            vec![fixture(42, -480, -330, 1)],
            &EscapeSession::detached(),
        );
        pickers[1].pointer_move(50.0, 50.0);
        pickers[1].pointer_down(50.0, 50.0);
        pickers[1].pointer_up(50.0, 50.0);
        pickers[1].frame(1.0 / 60.0);
        assert_eq!(SELECTED.get(), Some(42));
        assert!(pickers[0].state.selection.borrow().completed);
    }

    #[test]
    fn escape_or_right_click_cancels_without_capture() {
        for right_click in [false, true] {
            let escape = EscapeSession::detached();
            let mut pickers = open_test(&[(-100, -50)], vec![fixture(12, 0, 0, 1)], &escape);
            if right_click {
                pickers[0].secondary_down(150.0, 100.0);
                pickers[0].secondary_up(150.0, 100.0);
            } else {
                pickers[0].key(KeyEvent {
                    key: Key::Escape,
                    mods: Default::default(),
                });
            }
            pickers[0].frame(1.0 / 60.0);
            assert!(escape.is_cancelled());
            assert_eq!(SELECTED.get(), None);
            assert!(pickers[0].state.selection.borrow().completed);
        }
    }
}
