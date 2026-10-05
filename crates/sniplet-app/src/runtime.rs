use crate::{
    editor::{Command, Editor, render_options},
    menu_bar::NativeMenu,
};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use gpui_kit::*;
use sniplet_core::{
    AnnotationKind, AnnotationStyle, Color, Document, ImageRect, Point as ImagePoint,
};
use std::{
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};
use tray_icon::{TrayIcon, TrayIconBuilder, menu::MenuEvent};

struct Services {
    manager: Option<GlobalHotKeyManager>,
    _tray: Option<TrayIcon>,
    editor_window: Option<AnyWindowHandle>,
    escape: Option<EscapeRegistration>,
    next_escape_id: u64,
}
impl Global for Services {}

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
    overlay: Option<AnyWindowHandle>,
}

pub fn has_tray(cx: &App) -> bool {
    cx.has_global::<Services>() && cx.global::<Services>()._tray.is_some()
}

#[cfg(all(test, feature = "ui-tests"))]
pub(crate) fn install_test_services(cx: &mut App) {
    cx.set_global(Services {
        manager: None,
        _tray: None,
        editor_window: None,
        escape: None,
        next_escape_id: 1,
    });
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

pub fn validate_hotkeys(values: &[String]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for value in values.iter().filter(|value| !value.is_empty()) {
        let key = parse_hotkey(value).map_err(|error| format!("Invalid shortcut: {error}"))?;
        if !ids.insert(key.id()) {
            return Err(format!("Shortcut {value} is assigned more than once"));
        }
    }
    Ok(())
}

fn capture_bindings(settings: &sniplet_platform::HotkeySettings) -> [(&str, Command); 8] {
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
    settings: &sniplet_platform::HotkeySettings,
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
            manager.register(key).map_err(|error| error.to_string())?;
            Ok(key)
        }) {
            Ok(key) => bindings.push((key, command)),
            Err(error) => errors.push(format!("Shortcut {text} could not be registered: {error}")),
        }
    }
    (bindings, errors)
}

/// Hide the editor without dropping its document or sending it to the Dock.
pub fn hide_editor(window: &Window, _cx: &mut App) {
    #[cfg(target_os = "macos")]
    if crate::macos::hide_editor(window) {
        return;
    }
    window.minimize_window();
}

pub fn show_editor(window: &Window, cx: &mut App) {
    #[cfg(target_os = "macos")]
    crate::macos::show_dock_icon(window);
    window.activate_window();
    cx.activate(true);
}

/// Return the time needed for the editor to leave the captured screen.
pub fn hide_for_capture(window: &Window) -> Duration {
    #[cfg(target_os = "macos")]
    if crate::macos::hide_for_capture(window) {
        return Duration::ZERO;
    }
    window.minimize_window();
    Duration::from_millis(220)
}

pub fn reopen(cx: &mut App) {
    if cx.has_global::<Services>()
        && let Some(handle) = cx.global::<Services>().editor_window
    {
        let _ = handle.update(cx, |_, window, cx| show_editor(window, cx));
    }
}

pub fn install(
    handle: AnyWindowHandle,
    editor: Entity<Editor>,
    settings: &sniplet_platform::HotkeySettings,
    cx: &mut App,
) {
    let manager = GlobalHotKeyManager::new().ok();
    let (mut registered, errors) = register_hotkeys(manager.as_ref(), settings);
    for error in errors {
        editor.update(cx, |this, cx| {
            this.status = error;
            cx.notify();
        });
    }
    #[cfg(target_os = "windows")]
    let mut tray_light_theme = crate::menu_bar::taskbar_uses_light_theme();
    #[cfg(not(target_os = "windows"))]
    let tray_light_theme = true;
    let native = NativeMenu::new(settings).and_then(|menu| {
        let icon = crate::menu_bar::tray_icon(tray_light_theme)?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Sniplet")
            .with_menu(Box::new(menu.menu.clone()));
        #[cfg(target_os = "macos")]
        let tray = tray.with_icon_templated(icon);
        #[cfg(not(target_os = "macos"))]
        let tray = tray.with_icon(icon);
        let tray = tray.build()?;
        Ok((menu, tray))
    });
    let (mut native_menu, tray) = match native {
        Ok((menu, tray)) => (Some(menu), Some(tray)),
        Err(error) => {
            editor.update(cx, |this, cx| {
                this.status = format!(
                    "Menu bar unavailable: {error}. Close the editor or use Ctrl/Cmd Q to quit"
                );
                cx.notify();
            });
            (None, None)
        }
    };
    let mut current_hotkeys = settings.clone();
    let mut startup_checked_at = Instant::now();
    cx.set_global(Services {
        manager,
        _tray: tray,
        editor_window: Some(handle),
        escape: None,
        next_escape_id: 1,
    });
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(60))
                .await;
            cx.update(|cx| {
                let hotkeys = &editor.read(cx).settings.hotkeys;
                if *hotkeys != current_hotkeys {
                    let hotkeys = hotkeys.clone();
                    let manager = cx.global::<Services>().manager.as_ref();
                    if let Some(manager) = manager {
                        for (key, _) in &registered {
                            let _ = manager.unregister(*key);
                        }
                    }
                    let (bindings, mut errors) = register_hotkeys(manager, &hotkeys);
                    registered = bindings;
                    if let Some(menu) = &mut native_menu
                        && let Err(error) = menu.update_hotkeys(&hotkeys)
                    {
                        errors.push(format!("Menu shortcuts could not be updated: {error}"));
                    }
                    current_hotkeys = hotkeys;
                    for error in errors {
                        editor.update(cx, |this, cx| {
                            this.status = error;
                            cx.notify();
                        });
                    }
                }

                while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state != HotKeyState::Pressed {
                        continue;
                    }
                    let escape_pressed = cx
                        .global::<Services>()
                        .escape
                        .as_ref()
                        .and_then(|registration| registration.key.as_ref())
                        .is_some_and(|key| key.id() == event.id);
                    if escape_pressed {
                        cancel_active_escape(cx);
                        continue;
                    }
                    if let Some((_, command)) = registered.iter().find(|(key, _)| key.id() == event.id) {
                        invoke(handle, &editor, *command, cx);
                    }
                }
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    let Some(menu) = &native_menu else { continue };
                    if event.id == *menu.startup.id() {
                        #[cfg(target_os = "macos")]
                        {
                            let enabled = menu.startup.is_checked();
                            let result = crate::macos::set_launch_at_startup(enabled);
                            menu.startup.set_checked(crate::macos::launch_at_startup());
                            editor.update(cx, |this, cx| {
                                this.status = match result {
                                    Ok(true) => "Sniplet will launch at startup".into(),
                                    Ok(false) if !enabled => "Launch at startup is disabled".into(),
                                    Ok(false) => "Check Sniplet's setting in System Settings → General → Login Items".into(),
                                    Err(error) => format!("Could not change launch at startup: {error}"),
                                };
                                cx.notify();
                            });
                        }
                    } else if let Some(command) = menu.command(&event.id) {
                        if matches!(command, Command::Quit) {
                            cx.quit();
                            return;
                        }
                        invoke(handle, &editor, command, cx);
                    }
                }
                if startup_checked_at.elapsed() >= Duration::from_secs(1) {
                    #[cfg(target_os = "macos")]
                    if let Some(menu) = &native_menu {
                        menu.startup.set_checked(crate::macos::launch_at_startup());
                    }
                    #[cfg(target_os = "windows")]
                    {
                        let light_theme = crate::menu_bar::taskbar_uses_light_theme();
                        if light_theme != tray_light_theme
                            && let Some(tray) = &cx.global::<Services>()._tray
                            && let Ok(icon) = crate::menu_bar::tray_icon(light_theme)
                            && tray.set_icon(Some(icon)).is_ok()
                        {
                            tray_light_theme = light_theme;
                        }
                    }
                    startup_checked_at = Instant::now();
                }
            });
        }
    })
    .detach();
}

fn invoke(handle: AnyWindowHandle, editor: &Entity<Editor>, command: Command, cx: &mut App) {
    let _ = handle.update(cx, |_, window, cx| {
        if matches!(
            command,
            Command::Open | Command::LoadClipboard | Command::Settings
        ) {
            show_editor(window, cx);
        }
        editor.update(cx, |this, cx| this.command(command, window, cx))
    });
}

pub fn begin_escape_session(cx: &mut App) -> EscapeSession {
    let cancellation = sniplet_platform::ScrollCancellation::new();
    if !cx.has_global::<Services>() {
        return EscapeSession {
            id: 0,
            cancellation,
        };
    }

    let previous = {
        let services = cx.global_mut::<Services>();
        take_escape_registration(services, None)
    };
    if let Some(previous) = previous {
        previous.session.cancellation.cancel();
        if let Some(overlay) = previous.overlay {
            let _ = overlay.update(cx, |_, window, _| window.remove_window());
        }
    }

    let services = cx.global_mut::<Services>();
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
        overlay: None,
    });
    session
}

pub fn attach_escape_window(session: &EscapeSession, overlay: AnyWindowHandle, cx: &mut App) {
    if cx.has_global::<Services>()
        && let Some(registration) = &mut cx.global_mut::<Services>().escape
        && registration.session.id == session.id
    {
        registration.overlay = Some(overlay);
    }
}

pub fn detach_escape_window(session: &EscapeSession, cx: &mut App) {
    if cx.has_global::<Services>()
        && let Some(registration) = &mut cx.global_mut::<Services>().escape
        && registration.session.id == session.id
    {
        registration.overlay = None;
    }
}

pub fn cancel_escape_session(session: &EscapeSession, cx: &mut App) {
    if !cx.has_global::<Services>() {
        session.cancellation.cancel();
        return;
    }
    let registration = {
        let services = cx.global_mut::<Services>();
        take_escape_registration(services, Some(session.id))
    };
    if registration.is_some() {
        session.cancellation.cancel();
    }
}

pub fn end_escape_session(session: &EscapeSession, cx: &mut App) {
    if !cx.has_global::<Services>() {
        return;
    }
    let services = cx.global_mut::<Services>();
    let _ = take_escape_registration(services, Some(session.id));
}

pub(crate) fn cancel_active_escape(cx: &mut App) {
    if !cx.has_global::<Services>() {
        return;
    }
    let registration = {
        let services = cx.global_mut::<Services>();
        take_escape_registration(services, None)
    };
    let Some(registration) = registration else {
        return;
    };
    registration.session.cancellation.cancel();
    if let Some(overlay) = registration.overlay {
        let _ = overlay.update(cx, |_, window, _| window.remove_window());
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

pub fn demo() -> Document {
    let mut doc = Document::new(sniplet_core::demo_image(1000, 660));
    let red = AnnotationStyle {
        stroke: Color::new(255, 59, 48, 255),
        stroke_width: 4.0,
        fill: None,
    };
    doc.add_annotation(
        AnnotationKind::Text {
            origin: ImagePoint::new(70.0, 55.0),
            text: "Every detail, captured.".into(),
            font_size: 36.0,
        },
        AnnotationStyle {
            stroke: Color::new(35, 43, 57, 255),
            ..red
        },
    );
    doc.add_annotation(
        AnnotationKind::Text {
            origin: ImagePoint::new(70.0, 112.0),
            text: "Sniplet · screenshot, annotation, and pixel tools".into(),
            font_size: 20.0,
        },
        AnnotationStyle {
            stroke: Color::new(84, 98, 120, 255),
            ..red
        },
    );
    doc.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(60.0, 210.0, 375.0, 210.0),
        },
        red,
    );
    doc.add_annotation(
        AnnotationKind::Arrow {
            start: ImagePoint::new(700.0, 500.0),
            end: ImagePoint::new(420.0, 330.0),
            bend: None,
            variant: Default::default(),
        },
        AnnotationStyle {
            stroke_width: 10.0,
            ..red
        },
    );
    doc.add_annotation(
        AnnotationKind::Counter {
            center: ImagePoint::new(65.0, 210.0),
            value: 1,
            font_size: 30.0,
        },
        red,
    );
    doc
}

pub fn self_test(output: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&output)?;
    let monitors = sniplet_platform::list_monitors()?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary)
        .or_else(|| monitors.first())
        .ok_or_else(|| anyhow::anyhow!("No display available"))?;
    let frame = sniplet_platform::capture_monitor(monitor.index)?;
    anyhow::ensure!(
        frame.image.width() > 0 && frame.image.height() > 0,
        "Empty capture"
    );
    let dimensions = frame.image.dimensions();
    let mut doc = Document::new(frame.image);
    let id = doc.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(20.0, 20.0, 120.0, 90.0),
        },
        AnnotationStyle::default(),
    );
    anyhow::ensure!(doc.annotations().len() == 1, "Annotation missing");
    anyhow::ensure!(doc.undo() && doc.annotations().is_empty(), "Undo failed");
    anyhow::ensure!(doc.redo() && doc.annotation(id).is_some(), "Redo failed");
    doc.set_crop(ImageRect::new(
        0.0,
        0.0,
        320.0_f32.min(dimensions.0 as f32),
        240.0_f32.min(dimensions.1 as f32),
    ));
    let file = output.join("native-capture.png");
    doc.export(&file, sniplet_core::ExportFormat::Png, &render_options())?;
    let reopened = image::open(&file)?.to_rgba8();
    anyhow::ensure!(
        reopened.width() == 320.min(dimensions.0),
        "Crop/export dimensions differ"
    );
    demo().export(
        output.join("editor-demo.png"),
        sniplet_core::ExportFormat::Png,
        &render_options(),
    )?;
    println!(
        "{}",
        serde_json::json!({"result":"passed", "display":monitor.name, "captured_pixels":[dimensions.0,dimensions.1], "checks":["native capture", "annotation", "undo", "redo", "crop", "PNG export", "image reload", "font rendering"], "output":output})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // The toolkit prelude imports its UI test attribute; use Rust's attribute
    // explicitly for these platform-free shortcut checks.
    use std::prelude::v1::test;

    #[cfg(feature = "ui-tests")]
    struct EmptyWindow;

    #[cfg(feature = "ui-tests")]
    impl Render for EmptyWindow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[test]
    fn shortcuts_reject_alias_collisions_and_allow_disabled_bindings() {
        assert!(validate_hotkeys(&[String::new(), "Ctrl+Shift+1".into()]).is_ok());
        assert!(validate_hotkeys(&["Ctrl+Shift+1".into(), "Control+Shift+1".into()]).is_err());
        assert!(validate_hotkeys(&["ThisIsNotAKey".into()]).is_err());
        let settings = sniplet_platform::HotkeySettings::default();
        let values = capture_bindings(&settings).map(|(text, _)| text.to_owned());
        assert!(validate_hotkeys(&values).is_ok());
    }

    fn escape_registration(id: u64) -> EscapeRegistration {
        let mut session = EscapeSession::detached();
        session.id = id;
        EscapeRegistration {
            session,
            key: None,
            overlay: None,
        }
    }

    #[test]
    fn old_capture_cleanup_cannot_clear_a_new_escape_session() {
        let mut services = Services {
            manager: None,
            _tray: None,
            editor_window: None,
            escape: Some(escape_registration(2)),
            next_escape_id: 3,
        };

        assert!(take_escape_registration(&mut services, Some(1)).is_none());
        assert_eq!(services.escape.as_ref().unwrap().session.id, 2);
        assert_eq!(
            take_escape_registration(&mut services, Some(2))
                .unwrap()
                .session
                .id,
            2
        );
        assert!(services.escape.is_none());
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn active_escape_removes_overlay_without_activating_editor(cx: &mut TestAppContext) {
        let (owner, overlay, background, session) = cx.update(|cx| {
            gpui_kit::init(cx);
            install_test_services(cx);
            let (owner, _) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| EmptyWindow)
            })
            .unwrap();
            let session = begin_escape_session(cx);
            let (overlay, _) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| EmptyWindow)
            })
            .unwrap();
            attach_escape_window(&session, overlay, cx);
            let (background, _) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| EmptyWindow)
            })
            .unwrap();
            (owner, overlay, background, session)
        });

        cx.update_window(background, |_, window, _| window.activate_window())
            .unwrap();
        cx.update(cancel_active_escape);
        cx.run_until_parked();

        assert!(session.is_cancelled());
        cx.update(|cx| {
            assert!(cx.global::<Services>().escape.is_none());
            assert_eq!(cx.active_window(), Some(background));
        });
        assert!(!cx.windows().contains(&overlay));
        assert!(cx.windows().contains(&owner));
    }
}
