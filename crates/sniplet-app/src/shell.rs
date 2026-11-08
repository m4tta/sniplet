//! The app around the editor: tray icon, global shortcuts, the capture Escape key, and showing,
//! hiding and quitting. Every Rune window runs on one UI thread, so these services live in a
//! thread-local there. Tray and shortcut events arrive on other threads and reach the editor
//! through its inbox.
use std::{cell::RefCell, str::FromStr, time::Duration};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use rune::{AppHandle, Tx, WindowHandle};
use sniplet_platform::HotkeySettings;
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::editor::{Command, Editor};

const TRAY_TOOLTIP: &str = "Sniplet";

struct Services {
    editor: Option<AppHandle<Editor>>,
    manager: Option<GlobalHotKeyManager>,
    tray: Option<TrayIcon>,
    #[cfg(target_os = "linux")]
    native_menu: Option<crate::menu_bar::NativeMenu>,
    /// The shortcuts registered now, and the settings they came from.
    registered: Vec<(HotKey, Command)>,
    hotkeys: Option<HotkeySettings>,
    escape: Option<EscapeRegistration>,
    next_escape_id: u64,
    #[cfg(target_os = "windows")]
    tray_light_theme: bool,
}

thread_local! {
    static SERVICES: RefCell<Services> = const { RefCell::new(Services {
        editor: None,
        manager: None,
        tray: None,
        #[cfg(target_os = "linux")]
        native_menu: None,
        registered: Vec::new(),
        hotkeys: None,
        escape: None,
        next_escape_id: 1,
        #[cfg(target_os = "windows")]
        tray_light_theme: true,
    }) };
}

fn with<T>(f: impl FnOnce(&mut Services) -> T) -> T {
    SERVICES.with(|services| f(&mut services.borrow_mut()))
}

/// Give the shell the editor window's handle, before the app runs.
pub fn set_editor(handle: AppHandle<Editor>) {
    with(|services| services.editor = Some(handle));
}

pub fn has_tray() -> bool {
    with(|services| services.tray.is_some())
}

/// Hide the editor without dropping its document or leaving a taskbar or Dock icon.
pub fn hide_editor() {
    with(|services| {
        let Some(editor) = &services.editor else {
            return;
        };
        if services.tray.is_some() {
            editor.hide();
            editor.set_dock_icon(false);
        } else {
            editor.minimize();
        }
    });
}

pub fn show_editor() {
    with(|services| {
        if let Some(editor) = &services.editor {
            editor.set_dock_icon(true);
            editor.open();
        }
    });
}

/// Move the editor out of the way of a capture. Returns how long it needs to leave the
/// screen. Mac selectors use live capture, which can leave the editor in place.
pub fn hide_for_capture() -> Duration {
    if cfg!(target_os = "macos") {
        return Duration::ZERO;
    }
    with(|services| {
        if let Some(editor) = &services.editor {
            editor.minimize();
        }
    });
    Duration::from_millis(220)
}

/// Hide the tray tooltip while the tray panel is open, so it cannot cover the panel.
#[cfg(not(target_os = "linux"))]
pub(crate) fn show_tray_tooltip(show: bool) {
    with(|services| {
        if let Some(tray) = &services.tray {
            let _ = tray.set_tooltip(show.then_some(TRAY_TOOLTIP));
        }
    });
}

/// Create the tray and register the shortcuts. Runs once from the editor's setup, on the UI
/// thread, after the event loop exists.
pub fn install(editor: &mut Editor) {
    let inbox = editor.inbox.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            inbox.send(move |editor, tx| hotkey_pressed(event.id, editor, tx));
        }
    }));
    #[cfg(not(target_os = "linux"))]
    {
        let inbox = editor.inbox.clone();
        tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
            if let tray_icon::TrayIconEvent::Click {
                rect,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            } = event
            {
                inbox.send(move |editor, _| crate::tray_panel::toggle(rect, editor));
            }
        }));
    }
    #[cfg(target_os = "linux")]
    {
        let inbox = editor.inbox.clone();
        tray_icon::menu::MenuEvent::set_event_handler(Some(
            move |event: tray_icon::menu::MenuEvent| {
                inbox.send(move |editor, tx| {
                    let command = with(|services| {
                        services
                            .native_menu
                            .as_ref()
                            .and_then(|menu| menu.command(&event.id))
                    });
                    if let Some(command) = command {
                        run(command, editor, tx);
                    }
                });
            },
        ));
    }
    let manager = GlobalHotKeyManager::new().ok();
    with(|services| services.manager = manager);
    match create_tray(&editor.settings.hotkeys) {
        Ok(tray) => with(|services| services.tray = Some(tray)),
        Err(error) => {
            editor.status = format!(
                "Menu bar unavailable: {error}. Close the editor or use Ctrl/Cmd Q to quit"
            );
        }
    }
    sync_hotkeys(editor);
}

fn create_tray(hotkeys: &HotkeySettings) -> anyhow::Result<TrayIcon> {
    #[cfg(target_os = "windows")]
    let light = {
        let light = crate::menu_bar::taskbar_uses_light_theme();
        with(|services| services.tray_light_theme = light);
        light
    };
    #[cfg(not(target_os = "windows"))]
    let light = true;
    let _ = hotkeys;
    let tray = TrayIconBuilder::new().with_tooltip(TRAY_TOOLTIP);
    // Linux trays report no icon position for the panel, so they keep the native menu.
    #[cfg(target_os = "linux")]
    let menu = crate::menu_bar::NativeMenu::new(hotkeys)?;
    #[cfg(target_os = "linux")]
    let tray = tray.with_menu(Box::new(menu.menu.clone()));
    let icon = crate::menu_bar::tray_icon(light)?;
    #[cfg(target_os = "macos")]
    let tray = tray.with_icon_templated(icon);
    #[cfg(not(target_os = "macos"))]
    let tray = tray.with_icon(icon);
    let tray = tray.build()?;
    #[cfg(target_os = "linux")]
    with(|services| services.native_menu = Some(menu));
    Ok(tray)
}

/// Follow the taskbar's light or dark theme with the tray icon's ink. The editor's poll calls
/// it about once a second.
#[cfg(target_os = "windows")]
pub fn refresh_tray_theme() {
    let light = crate::menu_bar::taskbar_uses_light_theme();
    with(|services| {
        if light != services.tray_light_theme
            && let Some(tray) = &services.tray
            && let Ok(icon) = crate::menu_bar::tray_icon(light)
            && tray.set_icon(Some(icon)).is_ok()
        {
            services.tray_light_theme = light;
        }
    });
}

/// Register the editor's shortcuts again when they changed, or clear them while Settings
/// records a new one. Errors go to the editor's status.
pub fn sync_hotkeys(editor: &mut Editor) {
    let mut hotkeys = editor.settings.hotkeys.clone();
    if editor.hotkeys_paused {
        hotkeys.slots_mut().into_iter().for_each(String::clear);
    }
    let errors = with(|services| {
        if services.hotkeys.as_ref() == Some(&hotkeys) {
            return None;
        }
        if let Some(manager) = &services.manager {
            for (key, _) in &services.registered {
                let _ = manager.unregister(*key);
            }
        }
        let (bindings, mut errors) = register_hotkeys(services.manager.as_ref(), &hotkeys);
        services.registered = bindings;
        #[cfg(target_os = "linux")]
        if let Some(menu) = &mut services.native_menu
            && let Err(error) = menu.update_hotkeys(&hotkeys)
        {
            errors.push(format!("Menu shortcuts could not be updated: {error}"));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = &mut errors;
        services.hotkeys = Some(hotkeys);
        Some(errors)
    });
    let Some(errors) = errors else {
        return;
    };
    editor.hotkey_error = errors.first().cloned();
    if let Some(error) = errors.into_iter().last() {
        editor.status = error;
    }
}

fn hotkey_pressed(id: u32, editor: &mut Editor, tx: &mut Tx) {
    let escape = with(|services| {
        services
            .escape
            .as_ref()
            .and_then(|registration| registration.key.as_ref())
            .is_some_and(|key| key.id() == id)
    });
    if escape {
        cancel_active_escape();
        return;
    }
    let command = with(|services| {
        services
            .registered
            .iter()
            .find(|(key, _)| key.id() == id)
            .map(|(_, command)| *command)
    });
    if let Some(command) = command {
        run(command, editor, tx);
    }
}

/// Run a command from the tray or a global shortcut, bringing the editor forward first for
/// commands that load into it.
pub(crate) fn run(command: Command, editor: &mut Editor, tx: &mut Tx) {
    if matches!(command, Command::Open | Command::LoadClipboard) {
        show_editor();
    }
    editor.command(command, tx);
}

pub fn parse_hotkey(text: &str) -> Result<HotKey, String> {
    let text = text.replace(
        "CommandOrControl",
        if cfg!(target_os = "macos") {
            "Super"
        } else {
            "Ctrl"
        },
    );
    HotKey::from_str(&text).map_err(|error| error.to_string())
}

/// Returns the index of another shortcut that is the same key combination as
/// `values[index]`. Aliases such as Ctrl and Control count as the same keys.
pub fn conflicting_hotkey(values: &[&str], index: usize) -> Result<Option<usize>, String> {
    let id = parse_hotkey(values[index])?.id();
    Ok(values.iter().enumerate().position(|(other, text)| {
        other != index && !text.is_empty() && parse_hotkey(text).is_ok_and(|key| key.id() == id)
    }))
}

fn capture_bindings(settings: &HotkeySettings) -> [(&str, Command); 8] {
    [
        (&settings.capture_area, Command::Area),
        (&settings.capture_screen, Command::Screen),
        (&settings.capture_window, Command::Window),
        (&settings.scrolling_capture, Command::Scroll),
        (&settings.repeat_area, Command::Repeat),
        (&settings.active_window, Command::ActiveWindow),
        (&settings.capture_ocr, Command::CaptureOcr),
        (&settings.show_editor, Command::ShowEditor),
    ]
}

fn register_hotkeys(
    manager: Option<&GlobalHotKeyManager>,
    settings: &HotkeySettings,
) -> (Vec<(HotKey, Command)>, Vec<String>) {
    let Some(manager) = manager else {
        return (
            Vec::new(),
            vec![
                "Global shortcuts are unavailable in this desktop session; use the Sniplet menu"
                    .into(),
            ],
        );
    };
    let mut bindings = Vec::new();
    let mut errors = Vec::new();
    for (text, command) in capture_bindings(settings)
        .into_iter()
        .filter(|(text, _)| !text.is_empty())
    {
        match parse_hotkey(text).and_then(|key| {
            manager.register(key).map_err(|error| {
                eprintln!("Could not register shortcut {text}: {error}");
                "It may be in use by another app. Choose a different shortcut.".to_owned()
            })?;
            Ok(key)
        }) {
            Ok(key) => bindings.push((key, command)),
            Err(error) => {
                let label = text.replace(
                    "CommandOrControl",
                    if cfg!(target_os = "macos") {
                        "Cmd"
                    } else {
                        "Ctrl"
                    },
                );
                errors.push(format!("Shortcut {label} could not be enabled. {error}"));
            }
        }
    }
    (bindings, errors)
}

impl Editor {
    /// Quit, finishing a pending settings save first so no change is lost.
    pub(crate) fn quit(&mut self, tx: &mut Tx) {
        if self.settings_save_running
            && let Ok(store) = self.settings_store()
        {
            let _ = store.save(&self.settings);
        }
        tx.quit();
    }
}

/// One capture's Escape key: pressing Escape anywhere cancels the capture and closes its
/// overlays, even while another app has focus.
#[derive(Clone)]
pub struct EscapeSession {
    id: u64,
    cancellation: sniplet_platform::ScrollCancellation,
}

impl EscapeSession {
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn cancellation(&self) -> &sniplet_platform::ScrollCancellation {
        &self.cancellation
    }

    #[cfg(test)]
    pub fn detached() -> Self {
        Self {
            id: 0,
            cancellation: sniplet_platform::ScrollCancellation::new(),
        }
    }
}

struct EscapeRegistration {
    session: EscapeSession,
    key: Option<HotKey>,
    overlays: Vec<WindowHandle>,
}

/// Start a capture's Escape session, cancelling any earlier one and closing its overlays.
pub fn begin_escape_session() -> EscapeSession {
    let cancellation = sniplet_platform::ScrollCancellation::new();
    let previous = with(|services| take_escape_registration(services, None));
    if let Some(previous) = previous {
        previous.session.cancellation.cancel();
        for overlay in previous.overlays {
            overlay.close();
        }
    }
    with(|services| {
        let id = services.next_escape_id;
        services.next_escape_id = services.next_escape_id.wrapping_add(1).max(1);
        let session = EscapeSession { id, cancellation };
        let key = HotKey::from_str("Escape").ok().and_then(|key| {
            services
                .manager
                .as_ref()
                .and_then(|manager| manager.register(key).ok().map(|()| key))
        });
        services.escape = Some(EscapeRegistration {
            session: session.clone(),
            key,
            overlays: Vec::new(),
        });
        session
    })
}

/// Close `overlay` when the session is cancelled.
pub fn attach_escape_window(session: &EscapeSession, overlay: WindowHandle) {
    with(|services| {
        if let Some(registration) = &mut services.escape
            && registration.session.id == session.id
        {
            registration.overlays.push(overlay);
        }
    });
}

/// The session's overlays closed on their own; Escape no longer needs to close them.
pub fn detach_escape_window(session: &EscapeSession) {
    with(|services| {
        if let Some(registration) = &mut services.escape
            && registration.session.id == session.id
        {
            registration.overlays.clear();
        }
    });
}

pub fn cancel_escape_session(session: &EscapeSession) {
    let registration = with(|services| take_escape_registration(services, Some(session.id)));
    if registration.is_some() || session.id == 0 {
        session.cancellation.cancel();
    }
}

pub fn end_escape_session(session: &EscapeSession) {
    with(|services| take_escape_registration(services, Some(session.id)));
}

pub(crate) fn cancel_active_escape() {
    let Some(registration) = with(|services| take_escape_registration(services, None)) else {
        return;
    };
    registration.session.cancellation.cancel();
    for overlay in registration.overlays {
        overlay.close();
    }
}

fn take_escape_registration(
    services: &mut Services,
    expected_id: Option<u64>,
) -> Option<EscapeRegistration> {
    if expected_id.is_some_and(|id| {
        services
            .escape
            .as_ref()
            .is_none_or(|registration| registration.session.id != id)
    }) {
        return None;
    }
    let registration = services.escape.take()?;
    if let Some(key) = registration.key
        && let Some(manager) = &services.manager
    {
        let _ = manager.unregister(key);
    }
    Some(registration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_reject_alias_collisions_and_allow_disabled_bindings() {
        assert_eq!(conflicting_hotkey(&["", "Ctrl+Shift+1"], 1), Ok(None));
        assert_eq!(
            conflicting_hotkey(&["Ctrl+Shift+1", "Control+Shift+1"], 1),
            Ok(Some(0))
        );
        assert!(conflicting_hotkey(&["ThisIsNotAKey"], 0).is_err());
        let settings = HotkeySettings::default();
        let values = capture_bindings(&settings).map(|(text, _)| text);
        for index in 0..values.len() {
            assert_eq!(conflicting_hotkey(&values, index), Ok(None));
        }
    }

    #[test]
    fn old_capture_cleanup_cannot_clear_a_new_escape_session() {
        let first = begin_escape_session();
        let second = begin_escape_session();
        assert!(first.is_cancelled(), "a new session cancels the old one");
        end_escape_session(&first);
        cancel_active_escape();
        assert!(
            second.is_cancelled(),
            "the old cleanup left the new session active"
        );
    }
}
