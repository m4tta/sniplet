#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod area_capture;
mod arrow_palette;
mod capture;
mod clipboard_jobs;
mod drag_out;
mod editor;
mod menu_bar;
mod quick_menu;
mod self_test;
mod settings_window;
mod shell;
mod startup;
mod tools;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod tray_panel;
mod ui;
mod updates;
mod window_picker;

use std::sync::atomic::{AtomicBool, Ordering};

use editor::{BAR, Editor};
use rune::{AppSpec, Titlebar, Tx, Waker, WindowOptions};
use sniplet_core::Document;

/// `--smoke`: open the window and quit after three seconds.
static SMOKE: AtomicBool = AtomicBool::new(false);
/// Start in the tray without showing the editor, when a tray exists.
static MINIMIZED: AtomicBool = AtomicBool::new(false);

fn main() -> anyhow::Result<()> {
    // Rune reads its own flags (`--snap`, `--tree`, `--script` and the rest) and leaves the rest
    // to Sniplet, so the app runs headless with the same arguments.
    let args: Vec<String> = rune::extra_args().to_vec();
    if args.iter().any(|arg| arg == "--build-info") {
        println!(
            "{}",
            serde_json::to_string(&sniplet_platform::updates::current_build())?
        );
        return Ok(());
    }
    if let Some(parent) = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--update-parent="))
    {
        sniplet_platform::updates::wait_for_parent(parent.parse()?)?;
    }
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "Sniplet — cross-platform screenshot editor\n\nUsage: sniplet [IMAGE | PROJECT.sniplet] [--demo] [--settings] [--minimized] [--smoke]\n       sniplet --self-test [OUTPUT_DIRECTORY]\n\n--demo       Open an original generated sample image\n--settings   Open the settings window\n--minimized  Start in the tray without opening the editor\n--smoke      Open a native window and quit after 3 seconds\n--self-test  Exercise capture, annotation, PNG export, and image reload\n"
        );
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "--self-test") {
        return self_test::run(
            args.get(1)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "artifacts/self-test".into()),
        );
    }
    let mut startup_messages = Vec::new();
    let restoring = args.iter().any(|arg| arg == updates::RESTORE_ARG);
    let document_result = if restoring {
        let restored = sniplet_platform::SettingsStore::for_app()
            .map_err(anyhow::Error::from)
            .and_then(|store| updates::restore(&store));
        startup_messages.push(match &restored {
            Ok(_) => "Sniplet is updated. Your screenshot is back.".to_owned(),
            Err(error) => {
                format!("Sniplet is updated, but your screenshot could not come back: {error}")
            }
        });
        Ok(restored.ok())
    } else if args.iter().any(|arg| arg == "--demo") {
        Ok(Some(self_test::demo()))
    } else if let Some(path) = args.iter().find(|arg| !arg.starts_with('-')) {
        (|| -> anyhow::Result<Option<Document>> {
            Ok(Some(if sniplet_core::Project::is_project_path(path) {
                sniplet_core::Project::load(path)?.open_document()?
            } else {
                Document::new(image::open(path)?.to_rgba8())
            }))
        })()
    } else {
        Ok(None)
    };
    let document = match document_result {
        Ok(document) => document,
        Err(error) => {
            startup_messages.push(format!("Could not open image: {error}"));
            None
        }
    };
    let recovered = restoring && document.is_some();
    let settings = match sniplet_platform::SettingsStore::for_app().and_then(|store| store.load()) {
        Ok(settings) => settings,
        Err(error) => {
            startup_messages.push(format!(
                "Could not load preferences: {error}. Using defaults."
            ));
            sniplet_platform::Settings::default()
        }
    };
    if recovered && let Ok(store) = sniplet_platform::SettingsStore::for_app() {
        updates::clear_recovery(&store);
    }
    let smoke = args.iter().any(|arg| arg == "--smoke");
    SMOKE.store(smoke, Ordering::Relaxed);
    let login_launch = false;
    let minimized = !smoke && startup::start_minimized(&args, login_launch);
    #[cfg(target_os = "linux")]
    let minimized = minimized && startup::tray_host_available();
    MINIMIZED.store(minimized, Ordering::Relaxed);
    let open_settings = args.iter().any(|arg| arg == "--settings");
    let always_on_top = settings.always_on_top && !args.iter().any(|a| a == "--normal-window");
    let scale = sniplet_platform::list_monitors()
        .ok()
        .and_then(|monitors| monitors.into_iter().find(|m| m.is_primary))
        .map_or(1.0, |monitor| monitor.scale_factor);

    rune::prewarm_gpu();
    let spec = AppSpec {
        min_size: Some((900.0, 560.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(editor::on_key),
        ..AppSpec::new(
            "Sniplet",
            (1280.0, 850.0),
            || unreachable!("init builds the editor"),
            editor::view,
        )
    };
    let app = rune::App::new(spec)
        .init(move || {
            let mut editor = Editor::new(document, settings, scale);
            if !startup_messages.is_empty() {
                editor.status = startup_messages.join(" · ");
            }
            if open_settings {
                editor.open_settings(settings_window::Page::General);
            }
            editor
        })
        .titlebar(Titlebar::new(BAR))
        .modifier_keys()
        .keep_running()
        .on_close(on_close)
        .window_options(WindowOptions {
            // The editor shows once the tray exists, unless it starts in the tray.
            visible: !minimized,
            always_on_top,
            min_size: Some((900.0, 560.0)),
            ..Default::default()
        });
    shell::set_editor(app.handle());
    app.run();
    Ok(())
}

/// Runs once when the editor's runtime starts, with the waker that wakes its loop.
fn setup(editor: &mut Editor, waker: Waker) {
    editor.inbox.set_waker(waker);
    if rune::is_headless() {
        return;
    }
    if SMOKE.load(Ordering::Relaxed) {
        let inbox = editor.inbox.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            inbox.send(|_, tx| tx.quit());
        });
        return;
    }
    shell::install(editor);
    if MINIMIZED.load(Ordering::Relaxed) && !shell::has_tray() {
        shell::show_editor();
    }
    editor.schedule_updates();
}

/// Runs at the start of every frame: results from workers and other windows.
fn poll(editor: &mut Editor, _: f64, tx: &mut Tx) -> bool {
    let changed = ui::Inbox::drain(editor, |editor| &editor.inbox, tx);
    if changed && !rune::is_headless() {
        shell::sync_hotkeys(editor);
    }
    settings_window::sync(editor);
    #[cfg(target_os = "windows")]
    if shell::has_tray() {
        shell::refresh_tray_theme();
        tx.wake_after(std::time::Duration::from_secs(1));
    }
    changed
}

/// The editor's close button: hide to the tray when there is one, else quit.
fn on_close(editor: &mut Editor, tx: &mut Tx) {
    if shell::has_tray() {
        shell::hide_editor();
    } else {
        editor.quit(tx);
    }
}
