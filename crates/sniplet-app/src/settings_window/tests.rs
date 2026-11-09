use std::time::Duration;

use rune::{
    Env, FileDialogKind, Key, KeyEvent, Mods, PhysicalInput, PhysicalKeyCode, Runtime,
    ShortcutError,
};
use sniplet_platform::{ExportFormat, SettingsStore, ThemePreference, WindowBackground};

use super::*;

/// The main modifier, as a window reports it: Cmd on Mac, Ctrl elsewhere.
fn primary() -> Mods {
    let mac = cfg!(target_os = "macos");
    Mods {
        cmd: true,
        ctrl: !mac,
        logo: mac,
        ..Default::default()
    }
}

/// The Settings window and the editor it talks to, run headless side by side.
struct Harness {
    ui: Runtime<SettingsUi>,
    editor: Runtime<Editor>,
    link: Link,
    directory: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl Harness {
    fn new(page: Page, size: (f32, f32), dark: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let mut editor = Editor::new(None, Settings::default(), 1.0);
        editor.settings_path = Some(path.clone());
        let ui = SettingsUi::new(&editor, page);
        let link = Link {
            inbox: ui.inbox.clone(),
            sent: ui.snapshot.clone(),
        };
        let env = Env {
            system_dark: dark,
            scale: 2.0,
        };
        let mut ui = Runtime::new(ui, view::view, size, env);
        ui.poll = Some(poll);
        ui.frame(0.0);
        let editor = Runtime::new(editor, |_, _| rune::el(), (100.0, 100.0), env);
        Self {
            ui,
            editor,
            link,
            directory,
            path,
        }
    }

    /// One turn of both windows: the window's frame and poll, the editor's inbox, and `sync`.
    /// The last frame steps half a second, so controls that fade in can take presses.
    fn turn(&mut self) {
        let drain = |editor: &mut Runtime<Editor>| {
            editor.update(|editor, tx| {
                Inbox::drain(editor, |editor| &editor.inbox, tx);
            })
        };
        self.ui.frame(1.0 / 60.0);
        drain(&mut self.editor);
        self.link.sync(&self.editor.state);
        self.ui.frame(0.5);
        drain(&mut self.editor);
    }

    /// Turn until `done`, for work on other threads: saves, startup checks, image loads.
    /// It checks only between saves: Windows refuses to replace a file that is open for reading.
    fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        for _ in 0..500 {
            self.turn();
            if !self.editor.state.settings_save_running && done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        panic!("timed out waiting for {what}");
    }

    fn click(&mut self, key: &str) {
        let (x, y) = self
            .ui
            .find(key)
            .unwrap_or_else(|| panic!("nothing keyed {key}"));
        self.ui.pointer_down(x, y);
        self.ui.pointer_up(x, y);
        self.turn();
    }

    fn press(&mut self, key: PhysicalKeyCode, mods: Mods) {
        for pressed in [true, false] {
            self.ui.physical_input(PhysicalInput::Key {
                key,
                mods,
                pressed,
                repeat: false,
            });
            self.turn();
        }
    }

    fn enter(&mut self) {
        self.ui.key(KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        });
        self.turn();
    }

    fn right(&self, key: &str) -> f32 {
        let (rect, _) = self
            .ui
            .presented(key)
            .unwrap_or_else(|| panic!("nothing keyed {key}"));
        rect[0] + rect[2]
    }

    fn saved(&self) -> Settings {
        SettingsStore::at(&self.path).load().unwrap()
    }

    fn settings(&self) -> &Settings {
        &self.ui.state.settings
    }
}

#[test]
fn saved_shortcuts_round_trip_through_recorded_chords() {
    let defaults = HotkeySettings::default();
    for text in defaults.slots() {
        assert_eq!(hotkey_text(&shortcut(text).unwrap()), text);
    }
    assert!(shortcut("").is_none());
    let record = |key, mods| hotkey_text(&Shortcut::from_physical_key(key, mods).unwrap());
    assert_eq!(record(PhysicalKeyCode::F5, Mods::default()), "F5");
    let alt = Mods {
        alt: true,
        ..primary()
    };
    for (key, text) in [
        (PhysicalKeyCode::KeyK, "CommandOrControl+Alt+K"),
        (
            PhysicalKeyCode::BracketLeft,
            "CommandOrControl+Alt+BracketLeft",
        ),
        (PhysicalKeyCode::PageUp, "CommandOrControl+Alt+PageUp"),
        (PhysicalKeyCode::Backslash, "CommandOrControl+Alt+Backslash"),
    ] {
        assert_eq!(record(key, alt), text);
        assert!(crate::shell::parse_hotkey(text).is_ok(), "{text}");
    }
}

#[test]
fn scroll_limit_steps_to_multiples_of_ten_within_bounds() {
    assert_eq!(
        [40, 45, 2, 200, 10].map(|limit| step_limit(limit, true)),
        [50, 50, 10, 200, 20]
    );
    assert_eq!(
        [40, 45, 2, 200, 10].map(|limit| step_limit(limit, false)),
        [30, 40, 2, 190, 2]
    );
}

#[test]
fn changes_save_in_the_background_and_report_the_result() {
    let mut h = Harness::new(Page::General, (680.0, 1400.0), true);
    h.click("theme-light");
    assert_eq!(h.settings().theme, ThemePreference::Light);
    h.click("settings-nav-1");
    h.click("auto-copy");
    h.click("scroll-limit-up");
    assert!(!h.settings().auto_copy);
    assert_eq!(h.settings().scroll_max_frames, 50);
    h.until("the save", |h| {
        let saved = h.saved();
        saved.theme == ThemePreference::Light && !saved.auto_copy && saved.scroll_max_frames == 50
    });
    assert_eq!(h.editor.state.settings, *h.settings());
    h.until("the toast", |h| h.ui.find("settings-toast").is_some());
    assert!(!h.ui.state.notice.as_ref().unwrap().error);

    std::fs::remove_file(&h.path).unwrap();
    std::fs::create_dir(&h.path).unwrap();
    h.click("hide-after-export");
    h.until("the error", |h| {
        h.ui.state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.error)
    });
}

#[test]
fn tabs_and_controls_fit_the_minimum_window_in_both_themes() {
    for dark in [false, true] {
        let mut h = Harness::new(Page::General, (560.0, 480.0), dark);
        for (index, page) in Page::ALL.into_iter().enumerate() {
            h.click(&format!("settings-nav-{index}"));
            assert_eq!(h.ui.state.page, page);
            assert!(h.right(&format!("settings-nav-{index}")) <= 560.0);
            assert!(h.right(&format!("settings-content-{index}")) <= 560.0);
            let controls: &[&str] = match page {
                Page::Capture => &["window-wallpaper", "window-trim", "window-padding"],
                Page::Shortcuts => &["hotkey-0", "hotkey-7", "reset-hotkeys"],
                Page::Share => &["save-format", "upload-enabled"],
                Page::General => &["theme-dark", "always-on-top"],
            };
            for key in controls {
                assert!(h.right(key) <= 560.0, "{key}");
            }
        }
    }
}

#[test]
fn shortcut_recorder_saves_keys_rejects_conflicts_and_pauses_global_shortcuts() {
    let mut h = Harness::new(Page::Shortcuts, (680.0, 1400.0), true);
    let defaults = HotkeySettings::default();
    h.click("hotkey-0");
    assert!(h.ui.state.recorders[0].is_recording());
    assert!(h.editor.state.hotkeys_paused);

    // A plain letter would take the key from typing everywhere, so it is refused.
    h.press(PhysicalKeyCode::KeyA, Mods::default());
    assert!(h.ui.state.recorders[0].error().is_some());
    assert_eq!(h.settings().hotkeys.capture_area, defaults.capture_area);

    let alt = Mods {
        alt: true,
        ..primary()
    };
    h.press(PhysicalKeyCode::Digit9, alt);
    assert_eq!(h.settings().hotkeys.capture_area, "CommandOrControl+Alt+9");
    assert!(!h.ui.state.recorders[0].is_recording());
    assert!(!h.editor.state.hotkeys_paused);
    assert_eq!(
        h.editor.state.settings.hotkeys.capture_area,
        "CommandOrControl+Alt+9"
    );

    // Capture screen has these keys, so nothing changes and the recorder says who has them.
    h.click("hotkey-0");
    let shift = Mods {
        shift: true,
        ..primary()
    };
    h.press(PhysicalKeyCode::Digit1, shift);
    let Some(ShortcutError::Backend(error)) = h.ui.state.recorders[0].error() else {
        panic!("expected a conflict");
    };
    assert!(error.contains("Capture screen"), "{error}");
    assert_eq!(h.settings().hotkeys.capture_area, "CommandOrControl+Alt+9");
    h.press(PhysicalKeyCode::Escape, Mods::default());
    assert!(!h.ui.state.recorders[0].is_recording());
    assert!(!h.editor.state.hotkeys_paused);

    // Backspace turns a shortcut off, and its buttons restore or clear it.
    h.click("hotkey-1");
    h.press(PhysicalKeyCode::Backspace, Mods::default());
    assert!(h.settings().hotkeys.capture_screen.is_empty());
    h.click("hotkey-reset-1");
    assert_eq!(h.settings().hotkeys.capture_screen, defaults.capture_screen);
    h.click("hotkey-clear-7");
    assert!(h.settings().hotkeys.show_editor.is_empty());
    h.until("the save", |h| {
        let saved = h.saved().hotkeys;
        saved.capture_area == "CommandOrControl+Alt+9"
            && saved.capture_screen == defaults.capture_screen
            && saved.show_editor.is_empty()
    });

    h.click("reset-hotkeys");
    assert_eq!(h.settings().hotkeys, defaults);

    // Closing the window while it records turns global shortcuts back on.
    h.click("hotkey-2");
    assert!(h.editor.state.hotkeys_paused);
    let Harness { ui, mut editor, .. } = h;
    drop(ui);
    editor.update(|editor, tx| {
        Inbox::drain(editor, |editor| &editor.inbox, tx);
    });
    assert!(!editor.state.hotkeys_paused);
}

#[test]
fn window_background_choices_swatches_color_and_padding_save() {
    let mut h = Harness::new(Page::Capture, (560.0, 1400.0), false);
    for (key, background) in [
        ("window-wallpaper", WindowBackground::Wallpaper),
        ("window-trim", WindowBackground::TrimShadow),
        ("window-transparent", WindowBackground::Transparent),
        ("window-solid", WindowBackground::Solid),
    ] {
        h.click(key);
        assert_eq!(h.settings().window_capture.background, background);
    }
    for index in 0..view::SWATCHES.len() {
        assert!(h.right(&format!("swatch-{index}")) <= 560.0);
    }
    h.click("swatch-2");
    assert_eq!(h.settings().window_capture.color, [0x1C, 0x1C, 0x1E]);
    assert_eq!(h.ui.state.background_color.text, "#1C1C1E");

    h.click("window-background-color");
    h.ui.update(|s, _| s.background_color.replace_text("invalid"));
    h.enter();
    assert!(h.ui.state.field_error(Field::BackgroundColor).is_some());
    assert_eq!(h.settings().window_capture.color, [0x1C, 0x1C, 0x1E]);
    h.ui.update(|s, _| s.background_color.replace_text("#123456"));
    h.enter();
    assert!(h.ui.state.field_error(Field::BackgroundColor).is_none());
    assert_eq!(h.settings().window_capture.color, [0x12, 0x34, 0x56]);

    // Leaving the field saves it too.
    h.ui.update(|s, _| s.background_color.replace_text("abcdef"));
    h.click("window-solid");
    h.turn();
    assert_eq!(h.settings().window_capture.color, [0xAB, 0xCD, 0xEF]);

    let (x, y) = h.ui.find("window-padding").unwrap();
    h.ui.pointer_down(x + 60.0, y);
    h.ui.pointer_move(x + 65.0, y);
    h.turn();
    h.ui.pointer_up(x + 65.0, y);
    h.turn();
    let padding = h.settings().window_capture.padding;
    assert!(padding > 100 && padding.is_multiple_of(4), "{padding}");
    h.until("the save", |h| {
        let saved = h.saved().window_capture;
        saved.color == [0xAB, 0xCD, 0xEF]
            && saved.padding == padding
            && saved.background == WindowBackground::Solid
    });
}

#[test]
fn file_pickers_save_choices_and_reject_unreadable_wallpapers() {
    let mut h = Harness::new(Page::Share, (680.0, 1400.0), true);
    let folder = h.directory.path().to_owned();
    h.click("choose-screenshot-folder");
    assert_eq!(h.ui.file_dialog().unwrap().0, FileDialogKind::Folder);
    h.ui.resolve_file_dialog(None).unwrap();
    h.turn();
    assert!(h.settings().screenshot_directory.is_none());
    h.click("choose-screenshot-folder");
    h.ui.resolve_file_dialog(Some(folder.clone())).unwrap();
    h.turn();
    assert_eq!(h.settings().screenshot_directory, Some(folder.clone()));
    h.click("save-format/segment1");
    h.click("downscale-on-save");
    assert_eq!(h.settings().format, ExportFormat::Jpeg);
    assert!(h.settings().downscale_on_save);

    h.click("settings-nav-1");
    h.click("window-wallpaper");
    let image = folder.join("wallpaper.png");
    image::RgbaImage::from_pixel(40, 30, image::Rgba([40, 100, 180, 255]))
        .save(&image)
        .unwrap();
    h.click("choose-wallpaper");
    h.ui.resolve_file_dialog(Some(folder.join("missing.png")))
        .unwrap();
    h.until("the refusal", |h| {
        h.ui.state.field_error(Field::Wallpaper).is_some()
    });
    h.click("choose-wallpaper");
    h.ui.resolve_file_dialog(Some(image.clone())).unwrap();
    h.until("the wallpaper", |h| {
        h.settings().window_capture.wallpaper.is_some()
    });
    assert!(h.ui.state.field_error(Field::Wallpaper).is_none());
    h.until("the save", |h| {
        h.saved().window_capture.wallpaper == Some(image.clone())
    });
    h.click("desktop-wallpaper");
    assert!(h.settings().window_capture.wallpaper.is_none());
}

#[test]
fn upload_validates_before_saving_and_turns_off() {
    let mut h = Harness::new(Page::Share, (680.0, 1400.0), true);
    h.click("upload-enabled");
    assert!(h.ui.state.upload_open);
    assert!(h.settings().cloud_upload.is_none());

    h.ui.update(|s, _| s.upload[0].replace_text("not-a-url"));
    h.click("save-upload");
    assert!(h.ui.state.field_error(Field::Upload).is_some());
    assert!(h.settings().cloud_upload.is_none());

    h.click("upload-kind/segment1");
    assert!(h.ui.find("aws-credentials").is_some());
    h.ui.update(|s, _| {
        for (index, value) in [
            (2, "screenshots"),
            (3, "us-east-1"),
            (4, "https://storage.example.com"),
            (5, "../bad"),
        ] {
            s.upload[index].replace_text(value);
        }
    });
    h.click("save-upload");
    assert!(h.ui.state.field_error(Field::Upload).is_some());
    h.ui.update(|s, _| s.upload[5].replace_text("sniplet/"));
    h.click("save-upload");
    assert!(h.ui.state.field_error(Field::Upload).is_none());
    assert!(matches!(
        &h.settings().cloud_upload,
        Some(CloudUploadConfig::S3 { bucket, key_prefix, .. })
            if bucket == "screenshots" && key_prefix == "sniplet/"
    ));
    h.until("the save", |h| h.saved().cloud_upload.is_some());

    h.click("upload-enabled");
    assert!(!h.ui.state.upload_open);
    assert!(h.settings().cloud_upload.is_none());
    h.until("the save", |h| h.saved().cloud_upload.is_none());
}

#[test]
fn an_upload_spends_the_signed_url_in_the_editor_and_this_window() {
    let mut h = Harness::new(Page::Share, (680.0, 1400.0), true);
    h.click("upload-enabled");
    // Port 0 refuses at once, so the upload fails fast. The URL is spent either way.
    h.ui.update(|s, _| s.upload[0].replace_text("http://127.0.0.1:0/shot.png?signature=s"));
    h.click("save-upload");
    h.until("the save", |h| h.saved().cloud_upload.is_some());

    h.editor.update(|editor, tx| {
        editor.document = Some(sniplet_core::Document::new(sniplet_core::demo_image(4, 3)));
        editor.command(crate::editor::Command::Upload, tx);
    });
    h.until("the upload", |h| {
        h.editor.state.status.starts_with("Upload failed")
    });

    assert!(h.editor.state.settings.cloud_upload.is_none());
    assert!(h.saved().cloud_upload.is_none());
    assert!(h.settings().cloud_upload.is_none());
    assert!(h.ui.state.upload[0].text.is_empty());
    // With the URL gone, Save has nothing to send back to the editor.
    h.click("save-upload");
    assert!(h.ui.state.field_error(Field::Upload).is_some());
    assert!(h.editor.state.settings.cloud_upload.is_none());
}

/// Renders every page in both themes to `target/settings-snapshots` for a look:
/// `cargo test -p sniplet-app settings_snapshots -- --ignored`.
#[test]
#[ignore = "writes images for review"]
fn settings_snapshots() {
    let out =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/settings-snapshots");
    std::fs::create_dir_all(&out).unwrap();
    for theme in [ThemePreference::Light, ThemePreference::Dark] {
        for page in Page::ALL {
            let settings = Settings {
                theme,
                window_capture: sniplet_platform::WindowCaptureStyle {
                    background: WindowBackground::Solid,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut ui = SettingsUi::new(&Editor::new(None, settings, 1.0), page);
            ui.upload_open = page == Page::Share;
            let path = out.join(format!("{}-{theme:?}.png", page.title().replace(' ', "")));
            rune::App::new(app_spec())
                .init(move || ui)
                .args(vec![
                    "--snap".into(),
                    path.display().to_string(),
                    "--size".into(),
                    "680x1100".into(),
                ])
                .run();
        }
    }
}
