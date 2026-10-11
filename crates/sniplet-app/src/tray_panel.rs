use std::time::{Duration, Instant};

use gpui_kit::{
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};

use crate::editor::{Command, Editor, tool_icon};

const CAPTURES: [(&str, &str, Command); 4] = [
    ("Screen", "monitor", Command::Screen),
    ("Area", "scan", Command::Area),
    ("Scrolling", "chevrons-down", Command::Scroll),
    ("Text & QR", "scan-text", Command::CaptureOcr),
];

const MORE: [(&str, &str, Command); 7] = [
    ("Repeat Area", "repeat", Command::Repeat),
    ("Active Window", "app-window", Command::ActiveWindow),
    ("Pick Window", "square-mouse-pointer", Command::Window),
    ("Delayed 3s", "timer", Command::Delayed),
    ("Scroll Up", "chevrons-up", Command::ScrollUp),
    ("Open File", "folder-open", Command::Open),
    (
        "Paste From Clipboard",
        "clipboard-paste",
        Command::LoadClipboard,
    ),
];

const FOOTER: [(&str, &str, Command); 3] = [
    ("Open Editor", "scissors", Command::ShowEditor),
    ("Settings", "settings", Command::Settings),
    ("Quit", "power", Command::Quit),
];

const WIDTH: f32 = 352.0;
const PADDING: f32 = 8.0;
const GAP: f32 = 8.0;
const TILE_HEIGHT: f32 = 80.0;
const ROW_HEIGHT: f32 = 32.0;
const HEIGHT: f32 = 2.0 * PADDING + TILE_HEIGHT + 2.0 * ROW_HEIGHT + 2.0 + 4.0 * GAP;
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
            if let Some((bounds, display)) =
                placement(icon, &screens(cx), size(px(WIDTH), px(HEIGHT)))
            {
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

/// A readable shortcut such as `Ctrl+Shift+2`, from a settings string such as
/// `CommandOrControl+Shift+2`.
fn shortcut_label(key: &str, mac: bool) -> String {
    key.split('+')
        .map(|part| match part {
            "CommandOrControl" | "CmdOrCtrl" if mac => "Cmd",
            "CommandOrControl" | "CmdOrCtrl" | "Control" => "Ctrl",
            "Super" | "Command" | "Meta" if mac => "Cmd",
            "Super" | "Command" | "Meta" => "Win",
            "Alt" if mac => "Option",
            "Comma" => ",",
            part => part
                .strip_prefix("Key")
                .or_else(|| part.strip_prefix("Digit"))
                .unwrap_or(part),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn shortcut(command: Command, cx: &App, editor: &Entity<Editor>) -> Option<String> {
    crate::menu_bar::shortcut(command, &editor.read(cx).settings.hotkeys)
        .map(|key| shortcut_label(key, cfg!(target_os = "macos")))
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

    fn run(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        dismiss(window, cx);
        let (editor_window, editor) = (self.editor_window, self.editor.clone());
        App::defer(cx, move |cx| {
            crate::runtime::invoke(editor_window, &editor, command, cx)
        });
    }

    fn capture_tile(
        &self,
        index: usize,
        (label, icon, command): (&'static str, &'static str, Command),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let hover = cx.theme().secondary_hover;
        div()
            .id(("tray-capture", index))
            .flex_1()
            .h(px(TILE_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .rounded_md()
            .bg(cx.theme().secondary)
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .child(tool_icon(icon).size(px(24.0)))
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .children(shortcut(command, cx, &self.editor)),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.run(command, window, cx)))
    }

    fn divider(cx: &App) -> Div {
        div().h(px(1.0)).bg(cx.theme().border)
    }
}

impl Render for TrayPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tiles = CAPTURES
            .into_iter()
            .enumerate()
            .map(|(index, item)| self.capture_tile(index, item, cx))
            .collect::<Vec<_>>();
        let more = MORE
            .into_iter()
            .enumerate()
            .map(|(index, (label, icon, command))| {
                let tooltip = match shortcut(command, cx, &self.editor) {
                    Some(key) => format!("{label} · {key}"),
                    None => label.to_owned(),
                };
                Button::new(("tray-more", index))
                    .ghost()
                    .icon(tool_icon(icon))
                    .tooltip(tooltip)
                    .w(px(40.0))
                    .h(px(ROW_HEIGHT))
                    .on_click(cx.listener(move |this, _, window, cx| this.run(command, window, cx)))
            })
            .collect::<Vec<_>>();
        let [open_editor, settings, quit]: [Button; 3] = std::array::from_fn(|index| {
            let (label, icon, command) = FOOTER[index];
            Button::new(("tray-footer", index))
                .ghost()
                .icon(tool_icon(icon))
                .label(label)
                .small()
                .h(px(ROW_HEIGHT))
                .on_click(cx.listener(move |this, _, window, cx| this.run(command, window, cx)))
        });
        div()
            .id("tray-panel")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .p(px(PADDING))
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    dismiss(window, cx);
                }
            }))
            .child(div().flex().gap(px(GAP)).children(tiles))
            .child(Self::divider(cx))
            .child(div().flex().justify_between().children(more))
            .child(Self::divider(cx))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(div().flex().child(open_editor).child(settings))
                    .child(quit),
            )
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

    #[test]
    fn shortcut_labels_are_readable_on_each_platform() {
        assert_eq!(
            shortcut_label("CommandOrControl+Shift+2", false),
            "Ctrl+Shift+2"
        );
        assert_eq!(
            shortcut_label("CommandOrControl+Shift+2", true),
            "Cmd+Shift+2"
        );
        assert_eq!(shortcut_label("Control+Alt+KeyA", false), "Ctrl+Alt+A");
        assert_eq!(shortcut_label("Super+Alt+Digit9", true), "Cmd+Option+9");
        assert_eq!(shortcut_label("CommandOrControl+Comma", false), "Ctrl+,");
    }

    #[test]
    fn every_panel_icon_is_bundled() {
        for (_, icon, _) in CAPTURES.iter().chain(&MORE).chain(&FOOTER) {
            assert!(
                AssetSource::load(&gpui_kit::assets::AllAssets, &format!("icons/{icon}.svg"))
                    .unwrap()
                    .is_some(),
                "{icon}"
            );
        }
    }
}
