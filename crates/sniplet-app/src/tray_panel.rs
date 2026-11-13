//! The quick menu the tray icon opens: a borderless, always-on-top window next to the icon
//! that closes when it loses focus. It holds a copy of what it shows and sends the chosen
//! command to the editor.
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use rune::{AppSpec, Cx, El, Key, KeyEvent, Tx, WindowHandle, WindowOptions, WindowSpec};
use sniplet_platform::{HotkeySettings, ThemePreference};

use crate::{
    editor::Editor,
    quick_menu::{self, Action},
    ui::{self, Inbox},
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

#[derive(Debug, PartialEq)]
enum Click<W> {
    Open,
    Close(W),
    Ignore,
}

impl<W: Clone> Lifecycle<W> {
    fn click(&self, now: Instant) -> Click<W> {
        match self {
            Self::Open(window) => Click::Close(window.clone()),
            Self::Closed {
                closed_at: Some(closed_at),
            } if now.saturating_duration_since(*closed_at) < REOPEN_GUARD => Click::Ignore,
            Self::Closed { .. } => Click::Open,
        }
    }
}

/// The open panel. Each panel gets a new id and window key, so a late close from an old
/// panel never touches a new one.
#[derive(Clone)]
struct Open {
    id: u64,
    window: WindowHandle,
}

thread_local! {
    // Every Rune window runs on the UI thread, so the panel's lifecycle lives there.
    static PANEL: RefCell<Lifecycle<Open>> = const { RefCell::new(Lifecycle::Closed { closed_at: None }) };
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// The tray icon was clicked at `rect`, in physical pixels: open the panel there, or close it.
pub(crate) fn toggle(rect: tray_icon::Rect, editor: &mut Editor) {
    match PANEL.with(|panel| panel.borrow().click(Instant::now())) {
        Click::Ignore => {}
        Click::Close(open) => {
            closed(open.id);
            open.window.close();
        }
        Click::Open => {
            open_beside(rect, editor);
        }
    }
}

/// Sniplet was opened again from the Dock, Finder or Spotlight: open the panel at the menu
/// bar icon, as a click on it does, and keep a panel that is already open. False when there is
/// no icon to open it at.
#[cfg(target_os = "macos")]
pub(crate) fn open_at_tray(editor: &mut Editor) -> bool {
    if PANEL.with(|panel| matches!(&*panel.borrow(), Lifecycle::Open(_))) {
        return true;
    }
    crate::shell::tray_rect().is_some_and(|rect| open_beside(rect, editor))
}

/// Open the panel beside the tray icon at `rect`. False when no display holds a place for it.
fn open_beside(rect: tray_icon::Rect, editor: &mut Editor) -> bool {
    let size = (quick_menu::WIDTH, quick_menu::TRAY.height());
    let Some((position, scale)) = placement(rect, &screens(), size) else {
        return false;
    };
    let physical = (
        (position.0 * scale).round() as i32,
        (position.1 * scale).round() as i32,
    );
    open(physical, size, editor);
    true
}

/// Mark panel `id` closed, if it is still the open one.
fn closed(id: u64) {
    let current = PANEL.with(|panel| {
        let mut panel = panel.borrow_mut();
        let current = matches!(&*panel, Lifecycle::Open(open) if open.id == id);
        if current {
            *panel = Lifecycle::Closed {
                closed_at: Some(Instant::now()),
            };
        }
        current
    });
    if current {
        crate::shell::show_tray_tooltip(true);
    }
}

fn open(position: (i32, i32), size: (f32, f32), editor: &Editor) {
    let id = NEXT_ID.with(|next| next.replace(next.get() + 1));
    let state = TrayPanel {
        id,
        editor: editor.inbox.clone(),
        hotkeys: editor.settings.hotkeys.clone(),
        theme: editor.settings.theme,
    };
    let spec = AppSpec {
        on_key: Some(on_key),
        ..AppSpec::new(
            "Sniplet",
            size,
            || unreachable!("the panel opens with state"),
            view,
        )
    };
    let window = rune::open_window(
        WindowSpec::new(format!("tray-panel-{id}"), spec)
            .state(state)
            .on_focus(|panel, focused, tx| {
                if !focused && !still_key(panel.id) {
                    panel.dismiss(tx);
                }
            })
            .on_created(|window| hide_from_taskbar(&window))
            .options(WindowOptions {
                transparent: true,
                borderless: true,
                always_on_top: true,
                resizable: false,
                position: Some(position),
                min_size: Some(size),
                max_size: Some(size),
                ..Default::default()
            }),
    );
    window.focus();
    PANEL.with(|panel| *panel.borrow_mut() = Lifecycle::Open(Open { id, window }));
    crate::shell::show_tray_tooltip(false);
}

/// Whether panel `id` is still AppKit's key window. Just after the panel opens, winit sends a
/// stale focus loss from its hidden source window while the panel stays key. Rune skips it
/// only for its native panels, so without this check the menu closes as soon as it opens.
#[cfg(target_os = "macos")]
fn still_key(id: u64) -> bool {
    PANEL.with(|panel| match &*panel.borrow() {
        Lifecycle::Open(open) if open.id == id => crate::macos::is_key(&open.window),
        _ => false,
    })
}

#[cfg(not(target_os = "macos"))]
fn still_key(_: u64) -> bool {
    false
}

/// Keep the panel off the taskbar, as a pop-up menu is: a tool window gets no button.
#[cfg(target_os = "windows")]
fn hide_from_taskbar(window: &WindowHandle) {
    use raw_window_handle::RawWindowHandle;
    use windows::Win32::{
        Foundation::HWND,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::{
            Shell::{ITaskbarList, TaskbarList},
            WindowsAndMessaging::{
                GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW,
                WS_EX_TOOLWINDOW,
            },
        },
    };
    let _ = window.with_raw_handle(|handle| {
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut std::ffi::c_void);
        // Rune has already shown it, still cloaked, so the taskbar has its button. Remove the
        // button directly: hiding and showing the window to apply the style would move focus
        // away, and the panel closes when it loses focus.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let style = (style | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
            if let Ok(taskbar) =
                CoCreateInstance::<_, ITaskbarList>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                && taskbar.HrInit().is_ok()
            {
                let _ = taskbar.DeleteTab(hwnd);
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn hide_from_taskbar(_: &WindowHandle) {}

/// A display's frame in its own logical coordinates: physical origin and size divided by its
/// scale, as tray icon positions are.
#[derive(Clone, Copy)]
struct Screen {
    origin: (f32, f32),
    size: (f32, f32),
    scale: f32,
}

fn screens() -> Vec<Screen> {
    sniplet_platform::list_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|monitor| {
            let scale = monitor.scale_factor.max(0.1);
            // macOS reports origins in points and sizes in pixels; elsewhere both are pixels.
            let origin_scale = if cfg!(target_os = "macos") {
                1.0
            } else {
                scale
            };
            Screen {
                origin: (
                    monitor.x as f32 / origin_scale,
                    monitor.y as f32 / origin_scale,
                ),
                size: (monitor.width as f32 / scale, monitor.height as f32 / scale),
                scale,
            }
        })
        .collect()
}

/// Place the panel beside the tray icon, on the side facing the screen centre. Returns its
/// top-left in the chosen screen's logical coordinates, and that screen's scale.
fn placement(
    icon: tray_icon::Rect,
    screens: &[Screen],
    panel: (f32, f32),
) -> Option<((f32, f32), f32)> {
    let logical = |scale: f32| {
        (
            icon.position.x as f32 / scale,
            icon.position.y as f32 / scale,
            icon.size.width as f32 / scale,
            icon.size.height as f32 / scale,
        )
    };
    let contains = |screen: &Screen, (x, y): (f32, f32)| {
        x >= screen.origin.0
            && x < screen.origin.0 + screen.size.0
            && y >= screen.origin.1
            && y < screen.origin.1 + screen.size.1
    };
    let screen = screens
        .iter()
        .find(|screen| {
            let (x, y, w, h) = logical(screen.scale);
            contains(screen, (x + w / 2.0, y + h / 2.0))
        })
        .or(screens.first())?;
    let (x, y, w, h) = logical(screen.scale);
    let (left, top) = screen.origin;
    let (right, bottom) = (left + screen.size.0, top + screen.size.1);
    let y = if y + h / 2.0 > top + screen.size.1 / 2.0 {
        y - MARGIN - panel.1
    } else {
        y + h + MARGIN
    };
    let x = x + w / 2.0 - panel.0 / 2.0;
    let x = x.clamp(left + MARGIN, (right - MARGIN - panel.0).max(left + MARGIN));
    let y = y.clamp(top + MARGIN, (bottom - MARGIN - panel.1).max(top + MARGIN));
    Some(((x, y), screen.scale))
}

/// The panel's state: a copy of what the menu shows, and the editor's inbox.
struct TrayPanel {
    id: u64,
    editor: Inbox<Editor>,
    hotkeys: HotkeySettings,
    theme: ThemePreference,
}

impl TrayPanel {
    fn dismiss(&self, tx: &mut Tx) {
        closed(self.id);
        tx.close_window();
    }

    fn choose(&mut self, action: Action, tx: &mut Tx) {
        let Action::Run(command) = action else {
            return;
        };
        self.dismiss(tx);
        self.editor
            .send(move |editor, tx| crate::shell::run(command, editor, tx));
    }
}

fn view(s: &TrayPanel, cx: &mut Cx<TrayPanel>) -> El {
    let k = ui::window_kit(cx, s.theme);
    quick_menu::view(
        &k,
        &quick_menu::TRAY,
        &s.hotkeys,
        |s: &mut TrayPanel, action, tx: &mut Tx| s.choose(action, tx),
    )
}

fn on_key(s: &mut TrayPanel, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.key != Key::Escape {
        return false;
    }
    s.dismiss(tx);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tray_icon::dpi::{PhysicalPosition, PhysicalSize};

    fn icon(x: f64, y: f64, width: u32, height: u32) -> tray_icon::Rect {
        tray_icon::Rect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    fn screen(x: f32, y: f32, width: f32, height: f32, scale: f32) -> Screen {
        Screen {
            origin: (x, y),
            size: (width, height),
            scale,
        }
    }

    fn place(icon: tray_icon::Rect, screens: &[Screen]) -> (f32, f32, f32) {
        let ((x, y), scale) = placement(icon, screens, (340.0, 200.0)).unwrap();
        (x, y, scale)
    }

    #[test]
    fn taskbar_icon_opens_the_panel_above_it_centred() {
        let screens = [screen(0.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(
            place(icon(1600.0, 1040.0, 32, 40), &screens),
            (1446.0, 832.0, 1.0)
        );
    }

    #[test]
    fn menu_bar_icon_opens_the_panel_below_it() {
        let screens = [screen(0.0, 0.0, 1440.0, 900.0, 2.0)];
        assert_eq!(
            place(icon(2000.0, 0.0, 44, 48), &screens),
            (841.0, 32.0, 2.0)
        );
    }

    #[test]
    fn panel_stays_inside_the_display_near_the_corner() {
        let screens = [screen(0.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(
            place(icon(1890.0, 1040.0, 24, 40), &screens),
            (1572.0, 832.0, 1.0)
        );
        assert_eq!(place(icon(0.0, 0.0, 24, 24), &screens), (8.0, 32.0, 1.0));
    }

    #[test]
    fn scaled_secondary_display_uses_its_own_logical_coordinates() {
        let screens = [
            screen(0.0, 0.0, 1920.0, 1080.0, 1.0),
            screen(1280.0, 0.0, 1280.0, 720.0, 1.5),
        ];
        assert_eq!(
            place(icon(3000.0, 1020.0, 36, 60), &screens),
            (1842.0, 472.0, 1.5)
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
    fn a_menu_item_closes_the_panel_and_runs_its_command_in_the_editor() {
        let env = rune::Env::new(true, 2.0);
        let editor = Editor::new(None, Default::default(), 1.0);
        let panel = TrayPanel {
            id: 1,
            editor: editor.inbox.clone(),
            hotkeys: HotkeySettings::default(),
            theme: ThemePreference::Dark,
        };
        let mut editor = rune::Runtime::new(editor, crate::editor::view, (1280.0, 850.0), env);
        let size = (quick_menu::WIDTH, quick_menu::TRAY.height());
        let mut rt = rune::Runtime::new(panel, view, size, env);
        rt.frame(0.0);
        assert!(rt.find("menu-more-6").is_some());
        assert!(rt.find("menu-image-0").is_none());
        // Quit, the footer's last item, so the editor's response is easy to see.
        let (x, y) = rt.find("menu-footer-2").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.shell_request, Some(rune::Shell::Close));
        assert_eq!(editor.shell_request, None);
        editor.update(|editor, tx| {
            Inbox::drain(editor, |e| &e.inbox, tx);
        });
        assert_eq!(editor.shell_request, Some(rune::Shell::Quit));
    }
}
