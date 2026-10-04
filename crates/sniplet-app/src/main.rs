#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod arrow_palette;
mod capture;
mod capture_overlay;
mod editor;
mod runtime;
mod theme;
mod tools;
mod window_picker;

use gpui_kit::{component::TitleBar, *};
use sniplet_core::Document;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "Sniplet — cross-platform screenshot editor\n\nUsage: sniplet [IMAGE | PROJECT.sniplet] [--demo] [--smoke]\n       sniplet --self-test [OUTPUT_DIRECTORY]\n\n--demo       Open an original generated sample image\n--smoke      Open a native GPUI window and quit after 3 seconds\n--self-test  Exercise capture, annotation, PNG export, and image reload\n"
        );
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "--self-test") {
        return runtime::self_test(
            args.get(1)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "artifacts/self-test".into()),
        );
    }
    let mut startup_messages = Vec::new();
    let document_result = if args.iter().any(|arg| arg == "--demo") {
        Ok(Some(runtime::demo()))
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
    let settings = match sniplet_platform::SettingsStore::for_app().and_then(|store| store.load()) {
        Ok(settings) => settings,
        Err(error) => {
            startup_messages.push(format!(
                "Could not load preferences: {error}. Using defaults."
            ));
            sniplet_platform::Settings::default()
        }
    };
    let smoke = args.iter().any(|arg| arg == "--smoke");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            if let Err(error) = cx
                .text_system()
                .add_fonts(vec![std::borrow::Cow::Borrowed(editor::FONT)])
            {
                eprintln!("Could not load the bundled font: {error}");
            }
            let mut options = TitleBar::window_options();
            options.window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.0), px(850.0)),
                cx,
            )));
            options.window_min_size = Some(size(px(900.0), px(560.0)));
            options.app_id = Some(sniplet_platform::APP_ID.into());
            if settings.always_on_top && !args.iter().any(|a| a == "--normal-window") {
                options.kind = WindowKind::PopUp;
            }
            let hotkeys = settings.hotkeys.clone();
            let (handle, editor) = gpui_kit::open_window(options, cx, |window, cx| {
                window.set_window_title("Sniplet");
                if !smoke {
                    window.on_window_should_close(cx, |window, cx| {
                        if runtime::has_tray(cx) {
                            window.minimize_window();
                        } else {
                            cx.quit();
                        }
                        false
                    });
                }
                cx.new(|cx| {
                    let mut editor = editor::Editor::new(document, settings, window, cx);
                    if !startup_messages.is_empty() {
                        editor.status = startup_messages.join(" · ");
                    }
                    editor
                })
            })
            .expect("Could not open Sniplet");
            if smoke {
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(3))
                        .await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
            } else {
                runtime::install(handle, editor, &hotkeys, cx);
            }
            cx.activate(true);
        });
    Ok(())
}

#[cfg(all(test, feature = "ui-tests"))]
mod ui_tests;
