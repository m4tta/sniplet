use std::time::Duration;

use gpui_kit::{prelude::*, *};

use crate::{
    capture_overlay::{CaptureSurface, subtract_rectangles},
    editor::Editor,
};

type CaptureWindow = fn(u32, Entity<Editor>, AnyWindowHandle, &mut App);

fn border_strips(bounds: Bounds<Pixels>, thickness: f32) -> Vec<Bounds<Pixels>> {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let thickness = thickness.min(width / 2.0).min(height / 2.0);
    if thickness <= 0.0 {
        return Vec::new();
    }
    let vertical_height = (height - thickness * 2.0).max(0.0);
    let mut strips = vec![
        Bounds::new(point(px(left), px(top)), size(px(width), px(thickness))),
        Bounds::new(
            point(px(left), px(top + height - thickness)),
            size(px(width), px(thickness)),
        ),
    ];
    if vertical_height > 0.0 {
        strips.extend([
            Bounds::new(
                point(px(left), px(top + thickness)),
                size(px(thickness), px(vertical_height)),
            ),
            Bounds::new(
                point(px(left + width - thickness), px(top + thickness)),
                size(px(thickness), px(vertical_height)),
            ),
        ]);
    }
    strips
}

fn non_empty(bounds: Bounds<Pixels>) -> bool {
    f32::from(bounds.size.width) > 0.0 && f32::from(bounds.size.height) > 0.0
}

struct WindowCaptureOverlay {
    surface: CaptureSurface,
    windows: Vec<sniplet_platform::WindowInfo>,
    own_process_id: u32,
    hovered: Option<u32>,
    pointer: Option<Point<Pixels>>,
    editor: Entity<Editor>,
    editor_window: AnyWindowHandle,
    escape: crate::runtime::EscapeSession,
    focus: FocusHandle,
    capture: CaptureWindow,
    preserve_escape_on_release: bool,
}

impl WindowCaptureOverlay {
    #[allow(clippy::too_many_arguments)]
    fn new(
        frame: sniplet_platform::CapturedFrame,
        windows: Vec<sniplet_platform::WindowInfo>,
        own_process_id: u32,
        editor: Entity<Editor>,
        editor_window: AnyWindowHandle,
        escape: crate::runtime::EscapeSession,
        capture: CaptureWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.on_release(|overlay, cx| {
            if !overlay.preserve_escape_on_release {
                crate::runtime::cancel_escape_session(&overlay.escape, cx);
            }
        })
        .detach();
        Self {
            surface: CaptureSurface::new(frame),
            windows,
            own_process_id,
            hovered: None,
            pointer: None,
            editor,
            editor_window,
            escape,
            focus,
            capture,
            preserve_escape_on_release: false,
        }
    }

    fn window_at(&self, position: Point<Pixels>) -> Option<u32> {
        sniplet_platform::window_at_point(
            &self.windows,
            self.surface.desktop_point(position),
            self.own_process_id,
        )
        .map(|window| window.id)
    }

    fn hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hovered = self.window_at(position);
        if self.hovered != hovered || self.pointer != Some(position) {
            self.hovered = hovered;
            self.pointer = Some(position);
            cx.notify();
        }
    }

    fn select_at(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.window_at(position) else {
            self.hover(position, cx);
            return;
        };
        self.preserve_escape_on_release = true;
        crate::runtime::detach_escape_window(&self.escape, cx);
        crate::runtime::end_escape_session(&self.escape, cx);

        let capture = self.capture;
        let editor = self.editor.clone();
        let editor_window = self.editor_window;
        window.remove_window();
        cx.defer(move |cx| capture(id, editor, editor_window, cx));
    }

    fn cancel(&self, window: &mut Window, cx: &mut Context<Self>) {
        crate::runtime::cancel_escape_session(&self.escape, cx);
        window.remove_window();
    }

    fn selected(
        &self,
        viewport: Size<Pixels>,
    ) -> Option<(
        &sniplet_platform::WindowInfo,
        Bounds<Pixels>,
        Bounds<Pixels>,
    )> {
        let selected = self
            .hovered
            .and_then(|id| self.windows.iter().find(|window| window.id == id))?;
        let viewport = Bounds::new(Point::default(), viewport);
        let window_bounds = self.surface.window_bounds(selected);
        let bounds = window_bounds.intersect(&viewport);
        (f32::from(bounds.size.width) >= 1.0 && f32::from(bounds.size.height) >= 1.0).then_some((
            selected,
            window_bounds,
            bounds,
        ))
    }
}

impl Render for WindowCaptureOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let selected = self.selected(viewport);
        let mut overlay = div()
            .id("window-capture-overlay")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .cursor_crosshair()
            .child(
                img(self.surface.image.clone())
                    .absolute()
                    .inset_0()
                    .size_full(),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.hover(event.position, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.select_at(event.position, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel(window, cx);
                }
            }));

        if let Some((selected, window_bounds, bounds)) = selected {
            let viewport_bounds = Bounds::new(Point::default(), viewport);
            let occluders = self
                .windows
                .iter()
                .filter(|window| {
                    window.z_order > selected.z_order
                        && window.is_capture_candidate(self.own_process_id)
                })
                .map(|window| {
                    self.surface
                        .window_bounds(window)
                        .intersect(&viewport_bounds)
                })
                .filter(|bounds| non_empty(*bounds))
                .collect::<Vec<_>>();
            let visible = subtract_rectangles(bounds, &occluders);
            let mut dimmed = subtract_rectangles(viewport_bounds, &[bounds]);
            dimmed.extend(subtract_rectangles(bounds, &visible));
            for bounds in dimmed {
                overlay = overlay.child(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .bg(rgba(0x00000058)),
                );
            }
            for bounds in &visible {
                overlay = overlay.child(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .bg(rgba(0x168cf342)),
                );
            }
            for bounds in border_strips(window_bounds, 2.0)
                .into_iter()
                .map(|strip| strip.intersect(&viewport_bounds))
                .filter(|strip| non_empty(*strip))
                .flat_map(|strip| subtract_rectangles(strip, &occluders))
            {
                overlay = overlay.child(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .bg(rgba(0x3298ffff)),
                );
            }
            let id = selected.id;
            let label = if selected.title.trim() == selected.app_name.trim() {
                selected.app_name.clone()
            } else {
                format!("{} · {}", selected.app_name, selected.title)
            };
            let highlight = div()
                .id(("window-capture-highlight", id as usize))
                .absolute()
                .left(bounds.origin.x)
                .top(bounds.origin.y)
                .w(bounds.size.width)
                .h(bounds.size.height);
            #[cfg(feature = "ui-tests")]
            let highlight = {
                use gpui_kit::test::TestSupportExt;
                highlight.test_support()
            };
            overlay = overlay.child(highlight);
            if let Some(label_bounds) = visible.iter().copied().max_by(|left, right| {
                (f32::from(left.size.width) * f32::from(left.size.height))
                    .total_cmp(&(f32::from(right.size.width) * f32::from(right.size.height)))
            }) && f32::from(label_bounds.size.width) >= 100.0
                && f32::from(label_bounds.size.height) >= 40.0
            {
                overlay = overlay.child(
                    div()
                        .absolute()
                        .left(label_bounds.origin.x + px(8.0))
                        .top(label_bounds.origin.y + px(8.0))
                        .max_w(label_bounds.size.width - px(16.0))
                        .truncate()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgba(0x075ca8eb))
                        .text_color(rgb(0xffffff))
                        .text_sm()
                        .child(label),
                );
            }
        } else {
            overlay = overlay.child(div().absolute().inset_0().bg(rgba(0x00000058)));
        }

        if let Some(pointer) = self.pointer {
            let camera = div()
                .id("window-capture-camera")
                .absolute()
                .left(pointer.x + px(12.0))
                .top(pointer.y + px(12.0))
                .w(px(26.0))
                .h(px(18.0))
                .rounded_md()
                .border_1()
                .border_color(rgba(0xffffffff))
                .bg(rgba(0x111827ed))
                .shadow_lg()
                .child(
                    div()
                        .absolute()
                        .left(px(9.0))
                        .top(px(5.0))
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded_full()
                        .border_2()
                        .border_color(rgba(0xffffffff)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(4.0))
                        .top(px(-4.0))
                        .w(px(9.0))
                        .h(px(5.0))
                        .rounded_t_md()
                        .bg(rgba(0x111827ed))
                        .border_1()
                        .border_color(rgba(0xffffffff)),
                );
            #[cfg(feature = "ui-tests")]
            let camera = {
                use gpui_kit::test::TestSupportExt;
                camera.test_support()
            };
            overlay = overlay.child(camera);
        }

        let overlay = overlay.child(
            div()
                .absolute()
                .bottom(px(36.0))
                .left(px((width / 2.0 - 185.0).max(12.0)))
                .px_4()
                .py_2()
                .rounded_lg()
                .bg(rgba(0x202020e8))
                .text_color(rgb(0xffffff))
                .text_sm()
                .child("Point at a window and click to capture · Esc to cancel"),
        );
        #[cfg(feature = "ui-tests")]
        let overlay = {
            use gpui_kit::test::TestSupportExt;
            overlay.test_support()
        };
        overlay
    }
}

pub(crate) fn open(
    monitor: usize,
    editor: Entity<Editor>,
    editor_window: AnyWindowHandle,
    cx: &mut App,
) -> Result<(), String> {
    let escape = crate::runtime::begin_escape_session(cx);
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(220))
            .await;
        if escape.is_cancelled() {
            cx.update(|cx| crate::runtime::end_escape_session(&escape, cx));
            return;
        }
        let snapshot = cx
            .background_executor()
            .spawn(async move {
                let frame = sniplet_platform::capture_monitor(monitor)?;
                let windows = sniplet_platform::list_windows()?;
                Ok::<_, sniplet_platform::PlatformError>((frame, windows))
            })
            .await;
        cx.update(|cx| {
            if escape.is_cancelled() {
                crate::runtime::end_escape_session(&escape, cx);
                return;
            }
            match snapshot {
                Ok((frame, windows)) => {
                    if let Err(error) = open_snapshot(
                        frame,
                        windows,
                        std::process::id(),
                        editor.clone(),
                        editor_window,
                        escape.clone(),
                        start_capture,
                        cx,
                    ) {
                        fail_open(&editor, editor_window, &escape, error, cx);
                    }
                }
                Err(error) => {
                    fail_open(&editor, editor_window, &escape, error.to_string(), cx);
                }
            }
        });
    })
    .detach();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn open_snapshot(
    frame: sniplet_platform::CapturedFrame,
    windows: Vec<sniplet_platform::WindowInfo>,
    own_process_id: u32,
    editor: Entity<Editor>,
    editor_window: AnyWindowHandle,
    escape: crate::runtime::EscapeSession,
    capture: CaptureWindow,
    cx: &mut App,
) -> Result<AnyWindowHandle, String> {
    let (bounds, display_id) = crate::capture_overlay::captured_frame_placement(&frame, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        app_id: Some("fish.boxjelly.sniplet".into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    };
    let overlay_escape = escape.clone();
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        window.set_window_title("Sniplet — Capture window");
        cx.new(|cx| {
            WindowCaptureOverlay::new(
                frame,
                windows,
                own_process_id,
                editor,
                editor_window,
                overlay_escape,
                capture,
                window,
                cx,
            )
        })
    })
    .map_err(|error| error.to_string())?;
    let handle = opened.0;
    crate::runtime::attach_escape_window(&escape, handle, cx);
    let _ = handle.update(cx, |_, window, _| window.activate_window());
    Ok(handle)
}

fn fail_open(
    editor: &Entity<Editor>,
    editor_window: AnyWindowHandle,
    escape: &crate::runtime::EscapeSession,
    error: String,
    cx: &mut App,
) {
    editor.update(cx, |editor, cx| {
        editor.status = format!("Could not start window capture: {error}");
        cx.notify();
    });
    crate::runtime::end_escape_session(escape, cx);
    let _ = editor_window.update(cx, |_, window, _| window.activate_window());
}

fn start_capture(id: u32, editor: Entity<Editor>, editor_window: AnyWindowHandle, cx: &mut App) {
    let capture_editor = editor.clone();
    let _ = editor_window.update(cx, move |_, owner_window, cx| {
        editor.update(cx, |_, editor_cx| {
            crate::capture::window(id, capture_editor, owner_window, editor_cx);
        });
    });
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use std::{cell::Cell, prelude::v1::test};

    use gpui_kit::{TestAppContext, test::TestWindowExt};
    use sniplet_core::Document;

    use super::*;

    thread_local! {
        static SELECTED: Cell<Option<(u32, usize)>> = const { Cell::new(None) };
    }

    fn observe_selection(id: u32, _: Entity<Editor>, _: AnyWindowHandle, cx: &mut App) {
        SELECTED.set(Some((id, cx.windows().len())));
    }

    fn editor(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Editor>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::runtime::install_test_services(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Some(Document::new(sniplet_core::demo_image(400, 300))),
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
        })
    }

    fn frame() -> sniplet_platform::CapturedFrame {
        sniplet_platform::CapturedFrame {
            image: sniplet_core::demo_image(800, 600),
            origin: sniplet_platform::ScreenPoint { x: -100, y: -50 },
            scale_factor: 1.0,
            source: sniplet_platform::CaptureSource::Monitor { index: 0, id: 7 },
        }
    }

    fn fixture(id: u32, x: i32, y: i32, z_order: i32) -> sniplet_platform::WindowInfo {
        sniplet_platform::WindowInfo {
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

    fn open_test_overlay(
        editor: Entity<Editor>,
        editor_window: AnyWindowHandle,
        windows: Vec<sniplet_platform::WindowInfo>,
        escape: crate::runtime::EscapeSession,
        cx: &mut App,
    ) -> AnyWindowHandle {
        open_snapshot(
            frame(),
            windows,
            std::process::id(),
            editor,
            editor_window,
            escape,
            observe_selection,
            cx,
        )
        .unwrap()
    }

    #[gpui_kit::test]
    fn moving_across_overlapping_windows_highlights_the_frontmost(cx: &mut TestAppContext) {
        let (editor_window, editor) = editor(cx);
        let overlay = cx.update(|cx| {
            open_test_overlay(
                editor,
                editor_window,
                vec![fixture(10, 0, 0, 2), fixture(20, 40, 30, 8)],
                crate::runtime::EscapeSession::detached(),
                cx,
            )
        });

        cx.update_window(overlay, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_event(
                MouseMoveEvent {
                    pressed_button: None,
                    position: point(px(170.0), px(130.0)),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert!(window.find(("window-capture-highlight", 20usize)).visible());
            assert!(window.find("window-capture-camera").visible());
            assert!(
                window
                    .try_find(("window-capture-highlight", 10usize))
                    .is_none()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn click_closes_overlay_before_signalling_real_window_capture(cx: &mut TestAppContext) {
        SELECTED.set(None);
        let (editor_window, editor) = editor(cx);
        let overlay = cx.update(|cx| {
            open_test_overlay(
                editor,
                editor_window,
                vec![fixture(11, 0, 0, 1)],
                crate::runtime::EscapeSession::detached(),
                cx,
            )
        });

        cx.update_window(overlay, |_, window, cx| {
            window.render_frame(cx);
            window.click_at("window-capture-overlay", point(px(150.0), px(100.0)), cx);
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(SELECTED.get(), Some((11, 1)));
        assert!(cx.update_window(overlay, |_, _, _| {}).is_err());
    }

    #[gpui_kit::test]
    fn escape_cancels_without_capture_or_reactivating_the_editor(cx: &mut TestAppContext) {
        SELECTED.set(None);
        let (editor_window, editor) = editor(cx);
        let escape = cx.update(crate::runtime::begin_escape_session);
        let observed_escape = escape.clone();
        let overlay = cx.update(|cx| {
            open_test_overlay(
                editor,
                editor_window,
                vec![fixture(12, 0, 0, 1)],
                escape,
                cx,
            )
        });

        cx.update_window(overlay, |_, window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();

        assert!(observed_escape.is_cancelled());
        assert_eq!(SELECTED.get(), None);
        assert!(cx.update_window(overlay, |_, _, _| {}).is_err());
        cx.update(|cx| assert_ne!(cx.active_window(), Some(editor_window)));
    }
}
