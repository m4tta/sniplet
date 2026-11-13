#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod area_capture;
mod arrow_palette;
mod capture;
mod capture_overlay;
mod clipboard_jobs;
mod drag_out;
mod editor;
#[cfg(target_os = "macos")]
mod macos;
mod menu_bar;
mod quick_menu;
mod self_test;
mod settings_window;
mod shell;
mod single_instance;
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
    sniplet_platform::updates::startup();
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
    let smoke = args.iter().any(|arg| arg == "--smoke");
    // Test runs start their own copies next to a running Sniplet.
    let instance = if smoke || rune::is_headless() {
        None
    } else {
        match single_instance::claim(&args) {
            Ok(single_instance::Claim::First(instance)) => Some(instance),
            Ok(single_instance::Claim::Forwarded) => return Ok(()),
            Err(error) => {
                eprintln!("Sniplet could not check for a running copy: {error}");
                None
            }
        }
    };
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
                Document::new(sniplet_core::open_image(path)?)
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
    SMOKE.store(smoke, Ordering::Relaxed);
    // macOS reports a login launch only once its event loop runs; `setup` checks it there.
    let minimized = !smoke && startup::start_minimized(&args, false);
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
        scenes: updates::SCENES,
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
            if let Some(instance) = instance {
                let inbox = editor.inbox.clone();
                single_instance::serve(instance, move |args| {
                    inbox.send(move |editor, _| relaunched(editor, &args));
                });
            }
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

/// Another launch of Sniplet handed over its arguments: act on them as a fresh start would.
fn relaunched(editor: &mut Editor, args: &[String]) {
    // A login launch while Sniplet already runs changes nothing.
    if startup::start_minimized(args, false) {
        return;
    }
    shell::show_editor();
    if args.iter().any(|arg| arg == updates::RESTORE_ARG) {
        editor.restore_forwarded_update();
    } else if args.iter().any(|arg| arg == "--demo") {
        editor.load(self_test::demo(), "Ready");
    } else if let Some(path) = args.iter().find(|arg| !arg.starts_with('-')) {
        editor.load_path(path.into());
    }
    if args.iter().any(|arg| arg == "--settings") {
        editor.open_settings(settings_window::Page::General);
    }
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
    // Ask at startup, so that the first capture does not meet the system prompt. The request
    // waits on the macOS permission service, so it runs on a worker.
    #[cfg(target_os = "macos")]
    editor.inbox.spawn(
        sniplet_platform::request_screen_recording,
        |editor, allowed, _| {
            if !allowed {
                // Opening Sniplet again only shows this copy, so the user must quit first.
                editor.status = "To capture, allow Sniplet in System Settings > Privacy & Security > Screen & System Audio Recording, then quit and reopen Sniplet.".into();
            }
        },
    );
    #[cfg(target_os = "macos")]
    if startup::start_minimized(rune::extra_args(), macos::launched_at_login()) {
        MINIMIZED.store(true, Ordering::Relaxed);
        if shell::has_tray() {
            shell::hide_editor();
        }
    }
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
