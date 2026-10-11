use std::time::{Duration, Instant};

use gpui_kit::{prelude::*, *};

use crate::{
    editor::Editor,
    quick_menu::{self, Action},
};

/// Distance kept between the panel, the tray icon, and the display edges.
const MARGIN: f32 = 8.0;
/// A click on the tray icon first deactivates an open panel, closing it, and
/// arrives here shortly after. That click must not reopen the panel.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lifecycle<W> {
    Closed { closed_at: Option<Instant> },
    Open(W),
}

impl Global for Lifecycle<AnyWindowHandle> {}

#[derive(Debug, PartialEq)]
enum Click<W> {
    Open,
    Close(W),
    Ignore,
}

impl<W: Copy> Lifecycle<W> {
    fn click(&self, now: Instant) -> Click<W> {
        match *self {
            Self::Open(window) => Click::Close(window),
            Self::Closed {
                closed_at: Some(closed_at),
            } if now.saturating_duration_since(closed_at) < REOPEN_GUARD => Click::Ignore,
            Self::Closed { .. } => Click::Open,
        }
    }
}

fn lifecycle(cx: &App) -> Lifecycle<AnyWindowHandle> {
    cx.try_global::<Lifecycle<AnyWindowHandle>>()
        .copied()
        .unwrap_or(Lifecycle::Closed { closed_at: None })
}

/// Toggle the panel for a tray icon click at `icon`, in physical pixels.
pub(crate) fn toggle(
    icon: tray_icon::Rect,
    editor_window: AnyWindowHandle,
    editor: &Entity<Editor>,
    cx: &mut App,
) {
    match lifecycle(cx).click(Instant::now()) {
        Click::Ignore => {}
        Click::Close(panel) => {
            let _ = panel.update(cx, |_, window, cx| dismiss(window, cx));
        }
        Click::Open => {
            if let Some((bounds, display)) = placement(
                icon,
                &screens(cx),
                size(px(quick_menu::WIDTH), px(quick_menu::TRAY.height())),
            ) {
                open(bounds, display, editor_window, editor.clone(), cx);
            }
        }
    }
}

fn dismiss(window: &mut Window, cx: &mut App) {
    if lifecycle(cx) == Lifecycle::Open(window.window_handle()) {
        cx.set_global(Lifecycle::<AnyWindowHandle>::Closed {
            closed_at: Some(Instant::now()),
        });
        crate::runtime::show_tray_tooltip(true, cx);
    }
    window.remove_window();
}

pub(crate) fn open(
    bounds: Bounds<Pixels>,
    display: DisplayId,
    editor_window: AnyWindowHandle,
    editor: Entity<Editor>,
    cx: &mut App,
) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(display),
        window_background: WindowBackgroundAppearance::Transparent,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some(sniplet_platform::APP_ID.into()),
        ..Default::default()
    };
    let Ok((panel, _)) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Sniplet");
        cx.new(|cx| TrayPanel::new(editor, editor_window, window, cx))
    }) else {
        return;
    };
    cx.set_global(Lifecycle::Open(panel));
    crate::runtime::show_tray_tooltip(false, cx);
    cx.activate(true);
    let _ = panel.update(cx, |_, window, cx| {
        // The root paints the theme background, which would square off the panel's corners.
        base::Root::update(window, cx, |root, _, _| {
            root.style().background = Some(transparent_black().into());
        });
        window.activate_window();
    });
}

struct Screen {
    id: DisplayId,
    bounds: Bounds<Pixels>,
    scale: f32,
}

fn screens(cx: &App) -> Vec<Screen> {
    let monitors = sniplet_platform::list_monitors().unwrap_or_default();
    cx.displays()
        .into_iter()
        .map(|display| Screen {
            id: display.id(),
            bounds: display.bounds(),
            scale: monitors
                .iter()
                .find(|monitor| monitor.id == u64::from(display.id()) as u32)
                .map_or(1.0, |monitor| monitor.scale_factor),
        })
        .collect()
}

/// Place the panel beside the tray icon, on the side facing the screen centre.
fn placement(
    icon: tray_icon::Rect,
    screens: &[Screen],
    panel: Size<Pixels>,
) -> Option<(Bounds<Pixels>, DisplayId)> {
    let physical = Bounds::new(
        point(icon.position.x as f32, icon.position.y as f32),
        size(icon.size.width as f32, icon.size.height as f32),
    );
    let logical = |scale: f32| {
        Bounds::new(
            point(px(physical.origin.x / scale), px(physical.origin.y / scale)),
            size(
                px(physical.size.width / scale),
                px(physical.size.height / scale),
            ),
        )
    };
    let screen = screens
        .iter()
        .find(|screen| screen.bounds.contains(&logical(screen.scale).center()))
        .or(screens.first())?;
    let icon = logical(screen.scale);
    let display = screen.bounds;
    let y = if icon.center().y > display.center().y {
        icon.top() - px(MARGIN) - panel.height
    } else {
        icon.bottom() + px(MARGIN)
    };
    let x = icon.center().x - panel.width / 2.0;
    let x = x.clamp(
        display.left() + px(MARGIN),
        (display.right() - px(MARGIN) - panel.width).max(display.left() + px(MARGIN)),
    );
    let y = y.clamp(
        display.top() + px(MARGIN),
        (display.bottom() - px(MARGIN) - panel.height).max(display.top() + px(MARGIN)),
    );
    Some((Bounds::new(point(x, y), panel), screen.id))
}

struct TrayPanel {
    focus: FocusHandle,
    editor: Entity<Editor>,
    editor_window: AnyWindowHandle,
    _activation: Subscription,
}

impl TrayPanel {
    fn new(
        editor: Entity<Editor>,
        editor_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let activation = cx.observe_window_activation(window, |_, window, cx| {
            if !window.is_window_active() {
                dismiss(window, cx);
            }
        });
        Self {
            focus,
            editor,
            editor_window,
            _activation: activation,
        }
    }
}

impl Render for TrayPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (editor_window, editor) = (self.editor_window, self.editor.clone());
        let choose = move |action, window: &mut Window, cx: &mut App| {
            let Action::Run(command) = action else {
                return;
            };
            dismiss(window, cx);
            let editor = editor.clone();
            App::defer(cx, move |cx| {
                crate::runtime::invoke(editor_window, &editor, command, cx)
            });
        };
        quick_menu::view(
            &quick_menu::TRAY,
            &self.editor.read(cx).settings.hotkeys,
            choose,
            cx,
        )
        .track_focus(&self.focus)
        .size_full()
        .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                dismiss(window, cx);
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    use tray_icon::dpi::{PhysicalPosition, PhysicalSize};

    fn icon(x: f64, y: f64, width: u32, height: u32) -> tray_icon::Rect {
        tray_icon::Rect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    fn screen(id: u64, x: f32, y: f32, width: f32, height: f32, scale: f32) -> Screen {
        Screen {
            id: DisplayId::new(id),
            bounds: Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
            scale,
        }
    }

    fn place(icon: tray_icon::Rect, screens: &[Screen]) -> (f32, f32, u64) {
        let (bounds, id) = placement(icon, screens, size(px(340.0), px(200.0))).unwrap();
        (
            f32::from(bounds.left()),
            f32::from(bounds.top()),
            u64::from(id),
        )
    }

    #[test]
    fn taskbar_icon_opens_the_panel_above_it_centred() {
        let screens = [screen(1, 0.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(
            place(icon(1600.0, 1040.0, 32, 40), &screens),
            (1446.0, 832.0, 1)
        );
    }

    #[test]
    fn menu_bar_icon_opens_the_panel_below_it() {
        let screens = [screen(1, 0.0, 0.0, 1440.0, 900.0, 2.0)];
        assert_eq!(place(icon(2000.0, 0.0, 44, 48), &screens), (841.0, 32.0, 1));
    }

    #[test]
    fn panel_stays_inside_the_display_near_the_corner() {
        let screens = [screen(1, 0.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(
            place(icon(1890.0, 1040.0, 24, 40), &screens),
            (1572.0, 832.0, 1)
        );
        assert_eq!(place(icon(0.0, 0.0, 24, 24), &screens), (8.0, 32.0, 1));
    }

    #[test]
    fn scaled_secondary_display_uses_its_own_logical_coordinates() {
        let screens = [
            screen(1, 0.0, 0.0, 1920.0, 1080.0, 1.0),
            screen(2, 1280.0, 0.0, 1280.0, 720.0, 1.5),
        ];
        assert_eq!(
            place(icon(3000.0, 1020.0, 36, 60), &screens),
            (1842.0, 472.0, 2)
        );
    }

    #[test]
    fn a_click_right_after_the_panel_closed_does_not_reopen_it() {
        let closed_at = Instant::now();
        let closed = Lifecycle::<u32>::Closed {
            closed_at: Some(closed_at),
        };
        assert_eq!(
            closed.click(closed_at + Duration::from_millis(100)),
            Click::Ignore
        );
        assert_eq!(
            closed.click(closed_at + Duration::from_millis(300)),
            Click::Open
        );
        assert_eq!(
            Lifecycle::<u32>::Closed { closed_at: None }.click(closed_at),
            Click::Open
        );
        assert_eq!(Lifecycle::Open(7_u32).click(closed_at), Click::Close(7));
    }
}
