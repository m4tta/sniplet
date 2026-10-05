use crate::{editor::Editor, tools::Tool};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Entity, InputEvent, TestAppContext, WindowBounds, WindowOptions, point, px,
    size,
};
use sniplet_core::Document;

fn editor(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Editor>) {
    editor_at_size(cx, 1280.0, 850.0)
}

fn editor_at_size(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (gpui_kit::AnyWindowHandle, Entity<Editor>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.0), px(0.0)),
                    size(px(width), px(height)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Some(Document::new(sniplet_core::demo_image(400, 300))),
                        Default::default(),
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap()
    })
}

#[gpui_kit::test]
fn held_measure_key_previews_without_editing_and_click_places_one_undo_step(
    cx: &mut TestAppContext,
) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |editor, cx| {
            let mut image =
                image::RgbaImage::from_pixel(400, 300, image::Rgba([255, 255, 255, 255]));
            for y in 80..140 {
                for x in 80..240 {
                    image.put_pixel(x, y, image::Rgba([80, 150, 220, 255]));
                }
            }
            editor.load(Document::new(image), "measure fixture", cx);
            editor.set_measure_scale(1.0);
        });
        window.render_frame(cx);
        let before = editor.read(cx).export_pixels().unwrap();
        pointer_move(window, point(px(580.0), px(390.0)), cx);
        window.dispatch_event(
            gpui_kit::KeyDownEvent {
                keystroke: gpui_kit::Keystroke::parse("right").unwrap(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(
            window.find("measure-preview").visible(),
            "holding an arrow must show a measurement"
        );
        assert_eq!(editor.read(cx).export_pixels().unwrap(), before);
        assert!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .is_empty()
        );
        pointer_down(window, point(px(580.0), px(390.0)), cx);
        pointer_up(window, point(px(580.0), px(390.0)), cx);
        assert_eq!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        assert_ne!(editor.read(cx).export_pixels().unwrap(), before);
        let sniplet_core::AnnotationKind::Measurement { measurement } =
            editor.read(cx).document.as_ref().unwrap().annotations()[0].kind
        else {
            panic!("a placed ruler must remain one measurement");
        };
        assert_eq!(measurement.label(), "160px");
        window.dispatch_event(
            gpui_kit::KeyUpEvent {
                keystroke: gpui_kit::Keystroke::parse("right").unwrap(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(window.try_find("measure-preview").is_none());
        window.press("ctrl-z", cx);
        assert_eq!(editor.read(cx).export_pixels().unwrap(), before);
        window.press("ctrl-shift-z", cx);
        assert_eq!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn measurement_keys_shift_wheel_and_units_apply_to_the_placed_ruler(cx: &mut TestAppContext) {
    let (handle, entity) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        entity.update(cx, |editor, cx| {
            let image = image::RgbaImage::from_fn(400, 300, |x, y| {
                if (78..242).contains(&x) && (78..142).contains(&y) {
                    if (80..240).contains(&x) && (80..140).contains(&y) {
                        image::Rgba([80, 150, 220, 255])
                    } else {
                        image::Rgba([150, 150, 150, 255])
                    }
                } else if (30..50).contains(&x) {
                    image::Rgba([240, 240, 240, 255])
                } else {
                    image::Rgba([255, 255, 255, 255])
                }
            });
            editor.load(Document::new(image), "Retina ruler fixture", cx);
            editor.set_measure_scale(2.0);
        });
        window.render_frame(cx);
        window.click(("tool", Tool::Ruler as usize), cx);
        assert!(entity.read(cx).panel == Some(crate::editor::Panel::Ruler));
        assert_eq!(entity.read(cx).tool, Tool::Select);
        window.click("close-panel", cx);
        let inside = point(px(580.0), px(390.0));
        pointer_move(window, inside, cx);
        measure_key_down(window, "2", cx);
        place_ruler(window, inside, false, cx);
        assert_eq!(placed_ruler(&entity, cx).label(), "30px");
        measure_key_up(window, "2", cx);
        window.press("ctrl-z", cx);

        measure_key_down(window, "1", cx);
        window.dispatch_event(
            gpui_kit::ModifiersChangedEvent {
                modifiers: gpui_kit::Modifiers {
                    shift: true,
                    ..Default::default()
                },
                capslock: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        place_ruler(window, inside, true, cx);
        assert_eq!(placed_ruler(&entity, cx).label(), "82px");
        window.click("measure-units", cx);
        place_ruler(window, inside, true, cx);
        assert_eq!(placed_ruler(&entity, cx).label(), "164px");
        measure_key_up(window, "1", cx);
        window.press("ctrl-z", cx);
        window.press("ctrl-z", cx);
        window.click("measure-units", cx);

        let gap = point(px(500.0), px(390.0));
        pointer_move(window, gap, cx);
        measure_key_down(window, "right", cx);
        for _ in 0..2 {
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position: gap,
                    delta: gpui_kit::ScrollDelta::Lines(point(0.0, -1.0)),
                    modifiers: Default::default(),
                    touch_phase: gpui_kit::TouchPhase::Moved,
                }
                .to_platform_input(),
                cx,
            );
        }
        place_ruler(window, gap, false, cx);
        assert_eq!(placed_ruler(&entity, cx).label(), "14px");
        measure_key_up(window, "right", cx);
        measure_key_down(window, "left", cx);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("measure-preview").is_none());
        assert_eq!(
            entity
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
    })
    .unwrap();
}

fn measure_key_down(window: &mut gpui_kit::Window, key: &str, cx: &mut gpui_kit::App) {
    window.dispatch_event(
        gpui_kit::KeyDownEvent {
            keystroke: gpui_kit::Keystroke::parse(key).unwrap(),
            is_held: false,
            prefer_character_input: false,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

#[gpui_kit::test]
fn measurement_respects_crop_and_keeps_selection_arrow_keys(cx: &mut TestAppContext) {
    let (handle, entity) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        entity.update(cx, |editor, cx| {
            let mut doc = Document::new(image::RgbaImage::from_pixel(
                400,
                300,
                image::Rgba([80, 150, 220, 255]),
            ));
            doc.set_crop(sniplet_core::ImageRect::new(100.0, 80.0, 100.0, 60.0));
            editor.load(doc, "cropped ruler fixture", cx);
            editor.set_measure_scale(1.0);
        });
        window.render_frame(cx);
        let inside = point(px(640.0), px(450.0));
        pointer_move(window, inside, cx);
        measure_key_down(window, "right", cx);
        place_ruler(window, inside, false, cx);
        let span = placed_ruler(&entity, cx);
        assert_eq!(
            (span.start.x, span.end.x, span.label()),
            (100.0, 200.0, "100px".into())
        );
        pointer_move(window, point(px(580.0), px(450.0)), cx);
        assert!(window.try_find("measure-preview").is_none());
        measure_key_up(window, "right", cx);
        entity.update(cx, |editor, cx| {
            let mut doc = Document::new(sniplet_core::demo_image(400, 300));
            let id = doc.add_annotation(
                sniplet_core::AnnotationKind::Rectangle {
                    rect: sniplet_core::ImageRect::new(20.0, 20.0, 40.0, 40.0),
                },
                Default::default(),
            );
            doc.select(Some(id)).unwrap();
            editor.load(doc, "selected object", cx);
        });
        window.render_frame(cx);
        window.press("shift-right", cx);
        assert_eq!(
            entity.read(cx).document.as_ref().unwrap().annotations()[0]
                .kind
                .bounds()
                .x,
            30.0
        );
        assert!(window.try_find("measure-preview").is_none());
    })
    .unwrap();
}

fn measure_key_up(window: &mut gpui_kit::Window, key: &str, cx: &mut gpui_kit::App) {
    window.dispatch_event(
        gpui_kit::KeyUpEvent {
            keystroke: gpui_kit::Keystroke::parse(key).unwrap(),
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn place_ruler(
    window: &mut gpui_kit::Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    shift: bool,
    cx: &mut gpui_kit::App,
) {
    window.dispatch_event(
        gpui_kit::MouseDownEvent {
            position,
            button: gpui_kit::MouseButton::Left,
            modifiers: gpui_kit::Modifiers {
                shift,
                ..Default::default()
            },
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    pointer_up(window, position, cx);
}

fn placed_ruler(editor: &Entity<Editor>, cx: &gpui_kit::App) -> sniplet_core::Measurement {
    let sniplet_core::AnnotationKind::Measurement { measurement } = editor
        .read(cx)
        .document
        .as_ref()
        .unwrap()
        .annotations()
        .last()
        .unwrap()
        .kind
    else {
        panic!("expected a measurement");
    };
    measurement
}

#[gpui_kit::test]
fn appearance_choices_apply_immediately_and_preserve_capture(cx: &mut TestAppContext) {
    use gpui_kit::component::{ActiveTheme, ThemeMode};
    use sniplet_platform::{SettingsStore, ThemePreference};

    let (handle, entity) = editor_at_size(cx, 900.0, 560.0);
    let path = std::env::temp_dir().join(format!(
        "sniplet-theme-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    let store = SettingsStore::at(&path);
    cx.update_window(handle, |_, window, cx| {
        entity.update(cx, |editor, cx| {
            editor.settings_path = Some(path.clone());
            editor.panel = Some(crate::editor::Panel::Settings);
            cx.notify();
        });
        let before = entity.read(cx).export_pixels().unwrap();
        window.render_frame(cx);
        assert_eq!(entity.read(cx).settings.theme, ThemePreference::System);
        for (id, preference, mode) in [
            ("theme-dark", ThemePreference::Dark, ThemeMode::Dark),
            ("theme-light", ThemePreference::Light, ThemeMode::Light),
            (
                "theme-system",
                ThemePreference::System,
                window.appearance().into(),
            ),
        ] {
            assert!(window.find(id).visible());
            window.click(id, cx);
            assert_eq!(cx.theme().mode, mode);
            assert_eq!(entity.read(cx).settings.theme, preference);
            assert_eq!(store.load().unwrap().theme, preference);
            assert_eq!(entity.read(cx).export_pixels().unwrap(), before);
        }
    })
    .unwrap();
    std::fs::remove_file(path).unwrap();
}

#[gpui_kit::test]
fn narrow_toolbar_keeps_export_and_overflow_controls_visible(cx: &mut TestAppContext) {
    let (handle, _) = editor_at_size(cx, 900.0, 560.0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for id in ["copy", "save", "pin", "more-tools", "zoom-fit"] {
            let button = window.find(id);
            assert!(button.visible(), "{id} should remain visible");
            assert!(
                button.bounds().right() <= px(900.0),
                "{id} should fit inside the window"
            );
        }
        window.click("more-tools", cx);
        assert!(
            window
                .find(("more-tool", Tool::Rectangle as usize))
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn draw_crop_and_undo_via_real_pointer_and_keys(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool", Tool::Rectangle as usize), cx);
        window.drag(point(px(470.0), px(332.0)), point(px(580.0), px(422.0)), cx);
        assert_eq!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        window.press("ctrl-z", cx);
        assert!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .is_empty()
        );
        window.press("ctrl-shift-z", cx);
        assert_eq!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        window.press("v", cx);
        window.drag(point(px(620.0), px(450.0)), point(px(730.0), px(550.0)), cx);
        assert!(editor.read(cx).selection.is_some());
        window.press("enter", cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        let crop = doc.crop().expect("Enter should commit crop");
        assert!((crop.width - 110.0).abs() < 0.01);
        assert!((crop.height - 100.0).abs() < 0.01);
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_entry_survives_editor_shortcuts_and_exports(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool", Tool::Text as usize), cx);
        window.drag(point(px(520.0), px(390.0)), point(px(520.0), px(390.0)), cx);
        window.click("annotation-text", cx);
        window.input("A smooth arrow 12", cx);
        window.press("right", cx);
        assert!(window.try_find("measure-preview").is_none());
        window.click("add-text", cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        assert_eq!(doc.annotations().len(), 1);
        assert!(matches!(&doc.annotations()[0].kind,
            sniplet_core::AnnotationKind::Text { text, .. } if text == "A smooth arrow 12"));
        let image = editor.read(cx).export_pixels().unwrap();
        assert_eq!(image.dimensions(), (400, 300));
        assert_ne!(image, *doc.original());
    })
    .unwrap();
}

fn arrow_snapshot(editor: &Entity<Editor>, cx: &gpui_kit::App) -> sniplet_core::Annotation {
    let doc = editor.read(cx).document.as_ref().unwrap();
    assert_eq!(doc.annotations().len(), 1);
    let arrow = doc.annotations()[0].clone();
    assert!(arrow.kind.arrow_points().is_some());
    arrow
}

#[gpui_kit::test]
fn arrow_key_draws_bold_arrow_with_three_handles_and_remembers_its_width(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
        assert!(editor.read(cx).selection.is_some());
        window.press("a", cx);
        assert_eq!(editor.read(cx).tool, Tool::Arrow);
        assert!(editor.read(cx).selection.is_none());
        window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
        assert_eq!(arrow_snapshot(&editor, cx).style.stroke_width, 10.0);
        for index in 0usize..3 {
            let handle = window.find(("arrow-handle", index));
            assert!(handle.visible());
            assert_eq!(handle.bounds().size, size(px(8.0), px(8.0)));
        }
        assert!(window.try_find(("arrow-handle", 3usize)).is_none());
        let track = window.find("arrow-size-track").bounds();
        window.click_at(
            "arrow-size-track",
            point(track.size.width * (13.0 / 29.0), track.size.height / 2.0),
            cx,
        );
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(arrow_snapshot(&editor, cx).style.stroke_width, 14.0);
        window.click(("tool", Tool::Rectangle as usize), cx);
        window.drag(point(px(750.0), px(520.0)), point(px(800.0), px(570.0)), cx);
        assert_eq!(
            editor.read(cx).document.as_ref().unwrap().annotations()[1]
                .style
                .stroke_width,
            3.0
        );
        window.press("a", cx);
        window.drag(point(px(500.0), px(530.0)), point(px(700.0), px(560.0)), cx);
        assert_eq!(
            editor.read(cx).document.as_ref().unwrap().annotations()[2]
                .style
                .stroke_width,
            14.0
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn anchor_input_is_coalesced_into_one_preview_and_releases_old_gpu_image(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("a", cx);
        window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
        let original = arrow_snapshot(&editor, cx);
        let midpoint = window.find(("arrow-handle", 1usize)).bounds().center();
        let frames = editor.read(cx).preview_frames;
        let previous = editor.read(cx).preview_image().unwrap();
        assert!(window.has_image_atlas_entry(&previous));
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position: midpoint,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        for step in 1..=128 {
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    pressed_button: Some(gpui_kit::MouseButton::Left),
                    position: midpoint + point(px(0.0), px(step as f32 / 4.0)),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        }
        let bent = arrow_snapshot(&editor, cx);
        assert!(
            (bent.kind.arrow_points().unwrap()[1].y
                - original.kind.arrow_points().unwrap()[1].y
                - 32.0)
                .abs()
                < 0.01
        );
        assert_eq!(
            editor.read(cx).preview_frames,
            frames,
            "input must not rasterize"
        );
        assert_eq!(editor.read(cx).preview_image().unwrap().id, previous.id);
        window.render_frame(cx);
        assert_eq!(editor.read(cx).preview_frames, frames + 1);
        let latest = editor.read(cx).preview_image().unwrap();
        assert_ne!(latest.id, previous.id);
        assert!(!window.has_image_atlas_entry(&previous));
        assert!(window.has_image_atlas_entry(&latest));
        let expected = crate::editor::display_image(editor.read(cx).export_pixels().unwrap());
        assert_eq!(latest.as_bytes(0), expected.as_bytes(0));
        pointer_up(window, midpoint + point(px(0.0), px(32.0)), cx);
        window.press("ctrl-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), original);
        window.press("ctrl-shift-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), bent);
        let frames = editor.read(cx).preview_frames;
        pointer_move(window, midpoint, cx);
        window.render_frame(cx);
        assert_eq!(
            editor.read(cx).preview_frames,
            frames,
            "hover must reuse the preview"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn draft_preview_updates_live_and_escape_discards_draft(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("a", cx);
        let original = editor.read(cx).preview_image().unwrap();
        let frames = editor.read(cx).preview_frames;
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position: point(px(470.0), px(350.0)),
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        for step in 1..=128 {
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    pressed_button: Some(gpui_kit::MouseButton::Left),
                    position: point(px(470.0 + step as f32), px(350.0 + step as f32 / 2.0)),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        }
        assert_eq!(editor.read(cx).preview_frames, frames);
        assert!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .is_empty()
        );
        window.render_frame(cx);
        assert_eq!(editor.read(cx).preview_frames, frames + 1);
        assert_ne!(
            editor.read(cx).preview_image().unwrap().as_bytes(0),
            original.as_bytes(0)
        );
        window.press("escape", cx);
        assert_eq!(
            editor.read(cx).preview_image().unwrap().as_bytes(0),
            original.as_bytes(0)
        );
        assert!(!editor.read(cx).document.as_ref().unwrap().can_undo());
        pointer_move(window, point(px(730.0), px(490.0)), cx);
        assert_eq!(
            editor.read(cx).preview_image().unwrap().as_bytes(0),
            original.as_bytes(0)
        );
        let frames = editor.read(cx).preview_frames;
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position: point(px(470.0), px(350.0)),
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                pressed_button: Some(gpui_kit::MouseButton::Left),
                position: point(px(600.0), px(430.0)),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        // GPUI flushes pending frames before delivering a keyboard event.
        // The next frame must contain the cancellation, with no stale draft.
        window.dispatch_keystroke(gpui_kit::Keystroke::parse("escape").unwrap(), cx);
        assert_eq!(editor.read(cx).preview_frames, frames + 1);
        window.render_frame(cx);
        assert_eq!(editor.read(cx).preview_frames, frames + 2);
        assert_eq!(
            editor.read(cx).preview_image().unwrap().as_bytes(0),
            original.as_bytes(0)
        );
        assert!(!editor.read(cx).document.as_ref().unwrap().can_undo());
    })
    .unwrap();
}

#[gpui_kit::test]
fn new_preview_resamples_the_stationary_cursor(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        let source = image::RgbaImage::from_pixel(400, 300, image::Rgba([64, 123, 200, 255]));
        editor.update(cx, |editor, cx| {
            editor.load(Document::new(source.clone()), "Fixture", cx)
        });
        window.render_frame(cx);
        pointer_move(window, point(px(640.0), px(440.0)), cx);
        assert_eq!(
            editor.read(cx).sampled_color(),
            sniplet_core::Color::new(64, 123, 200, 255)
        );
        let mut redacted = Document::new(source);
        redacted.add_annotation(
            sniplet_core::AnnotationKind::Redaction {
                rect: sniplet_core::ImageRect::new(0.0, 0.0, 400.0, 300.0),
            },
            Default::default(),
        );
        let frames = editor.read(cx).preview_frames;
        editor.update(cx, |editor, cx| {
            editor.load(redacted, "Redacted fixture", cx)
        });
        assert_eq!(editor.read(cx).preview_frames, frames);
        window.render_frame(cx);
        assert_eq!(editor.read(cx).preview_frames, frames + 1);
        assert_eq!(editor.read(cx).sampled_color(), sniplet_core::Color::BLACK);
    })
    .unwrap();
}

#[gpui_kit::test]
fn arrow_middle_and_endpoints_edit_independently_and_undo_as_one_drag(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("a", cx);
        window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
        let straight = arrow_snapshot(&editor, cx);
        let straight_pixels = editor.read(cx).export_pixels().unwrap();
        let midpoint = window.find(("arrow-handle", 1usize)).bounds().center();
        window.drag(midpoint, midpoint + point(px(0.0), px(40.0)), cx);
        let bent = arrow_snapshot(&editor, cx);
        let bent_points = bent.kind.arrow_points().unwrap();
        let straight_points = straight.kind.arrow_points().unwrap();
        assert_eq!(bent_points[0], straight_points[0]);
        assert_eq!(bent_points[2], straight_points[2]);
        assert!((bent_points[1].y - straight_points[1].y - 40.0).abs() < 0.01);
        let bent_pixels = editor.read(cx).export_pixels().unwrap();
        assert_ne!(bent_pixels, straight_pixels);
        window.press("ctrl-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), straight);
        assert_eq!(editor.read(cx).export_pixels().unwrap(), straight_pixels);
        window.press("ctrl-shift-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), bent);
        assert_eq!(editor.read(cx).export_pixels().unwrap(), bent_pixels);

        let end = window.find(("arrow-handle", 2usize)).bounds().center();
        window.drag(end, end + point(px(20.0), px(-20.0)), cx);
        let moved_end = arrow_snapshot(&editor, cx).kind.arrow_points().unwrap();
        assert_eq!(moved_end[0], bent_points[0]);
        assert!((moved_end[2].x - bent_points[2].x - 20.0).abs() < 0.01);
        assert!((moved_end[2].y - bent_points[2].y + 20.0).abs() < 0.01);
        window.press("ctrl-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), bent);

        window.press("v", cx);
        window.drag(point(px(600.0), px(460.0)), point(px(600.0), px(460.0)), cx);
        assert!(window.find(("arrow-handle", 0usize)).visible());
        let start = window.find(("arrow-handle", 0usize)).bounds().center();
        window.drag(start, start + point(px(20.0), px(20.0)), cx);
        let moved_start = arrow_snapshot(&editor, cx).kind.arrow_points().unwrap();
        assert_eq!(moved_start[2], bent_points[2]);
        assert!((moved_start[0].x - bent_points[0].x - 20.0).abs() < 0.01);
        assert!((moved_start[0].y - bent_points[0].y - 20.0).abs() < 0.01);
        window.press("ctrl-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), bent);
    })
    .unwrap();
}

#[gpui_kit::test]
fn curved_arrow_shaft_moves_all_handles_and_escape_restores_in_progress_bend(
    cx: &mut TestAppContext,
) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("a", cx);
        window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
        let midpoint = window.find(("arrow-handle", 1usize)).bounds().center();
        window.drag(midpoint, midpoint + point(px(0.0), px(40.0)), cx);
        let bent = arrow_snapshot(&editor, cx);
        window.drag(point(px(535.0), px(415.0)), point(px(555.0), px(435.0)), cx);
        let moved = arrow_snapshot(&editor, cx);
        for (before, after) in bent
            .kind
            .arrow_points()
            .unwrap()
            .into_iter()
            .zip(moved.kind.arrow_points().unwrap())
        {
            assert!((after.x - before.x - 20.0).abs() < 0.01);
            assert!((after.y - before.y - 20.0).abs() < 0.01);
        }
        window.press("ctrl-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), bent);

        let midpoint = window.find(("arrow-handle", 1usize)).bounds().center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position: midpoint,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                pressed_button: Some(gpui_kit::MouseButton::Left),
                position: midpoint + point(px(0.0), px(-70.0)),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert_ne!(arrow_snapshot(&editor, cx), bent);
        window.press("escape", cx);
        assert_eq!(arrow_snapshot(&editor, cx), bent);
        window.press("ctrl-z", cx);
        assert_eq!(
            arrow_snapshot(&editor, cx).kind.arrow_points().unwrap()[1],
            sniplet_core::Point::new(160.0, 118.0)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn arrow_variants_preserve_geometry_export_and_remain_editable(cx: &mut TestAppContext) {
    use sniplet_core::ArrowVariant;
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("a", cx);
        window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
        let midpoint = window.find(("arrow-handle", 1usize)).bounds().center();
        window.drag(midpoint, midpoint + point(px(0.0), px(35.0)), cx);
        window.click(("color", 4usize), cx);
        let initial = arrow_snapshot(&editor, cx);
        let mut exports = vec![editor.read(cx).export_pixels().unwrap()];
        for variant in [ArrowVariant::HandDrawn, ArrowVariant::Thin, ArrowVariant::DoubleEnded] {
            let before = arrow_snapshot(&editor, cx);
            window.click(("arrow-variant", variant as usize), cx);
            let changed = arrow_snapshot(&editor, cx);
            assert_eq!(changed.kind.arrow_points(), initial.kind.arrow_points());
            assert_eq!(changed.style, initial.style);
            assert!(matches!(changed.kind, sniplet_core::AnnotationKind::Arrow { variant: v, .. } if v == variant));
            let pixels = editor.read(cx).export_pixels().unwrap();
            assert!(exports.iter().all(|other| other != &pixels));
            window.press("ctrl-z", cx);
            assert_eq!(arrow_snapshot(&editor, cx), before);
            window.press("ctrl-shift-z", cx);
            assert_eq!(arrow_snapshot(&editor, cx), changed);
            for index in 0usize..3 {
                let control = window.find(("arrow-handle", index)).bounds().center();
                window.drag(control, control + point(px(5.0), px(10.0)), cx);
                assert_ne!(arrow_snapshot(&editor, cx).kind.arrow_points(), changed.kind.arrow_points());
                window.press("ctrl-z", cx);
                assert_eq!(arrow_snapshot(&editor, cx), changed);
            }
            exports.push(pixels);
        }
        window.press("v", cx);
        window.press("a", cx);
        window.drag(point(px(500.0), px(530.0)), point(px(700.0), px(560.0)), cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        assert_eq!(doc.annotations().len(), 2);
        assert!(matches!(doc.annotations()[1].kind, sniplet_core::AnnotationKind::Arrow { variant: ArrowVariant::DoubleEnded, .. }));
    }).unwrap();
}

fn slider_position(window: &gpui_kit::Window, value: f32) -> gpui_kit::Point<gpui_kit::Pixels> {
    let track = window.find("arrow-size-track").bounds();
    point(
        track.left() + track.size.width * ((value - 1.0) / 29.0),
        track.center().y,
    )
}

fn pointer_down(
    window: &mut gpui_kit::Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    cx: &mut gpui_kit::App,
) {
    window.dispatch_event(
        gpui_kit::MouseDownEvent {
            button: gpui_kit::MouseButton::Left,
            position,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn pointer_move(
    window: &mut gpui_kit::Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    cx: &mut gpui_kit::App,
) {
    window.dispatch_event(
        gpui_kit::MouseMoveEvent {
            pressed_button: Some(gpui_kit::MouseButton::Left),
            position,
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn pointer_up(
    window: &mut gpui_kit::Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    cx: &mut gpui_kit::App,
) {
    window.dispatch_event(
        gpui_kit::MouseUpEvent {
            button: gpui_kit::MouseButton::Left,
            position,
            modifiers: Default::default(),
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

#[gpui_kit::test]
fn arrow_size_drag_updates_live_and_is_one_undo_even_when_released_outside(
    cx: &mut TestAppContext,
) {
    let (handle, editor) = editor(cx);
    let original = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("a", cx);
            window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
            let original = arrow_snapshot(&editor, cx);
            let position = slider_position(window, 20.0);
            pointer_down(window, position, cx);
            original
        })
        .unwrap();
    // Each update ends at a real GPUI event boundary, flushing emitted slider events.
    for value in [22.5, 25.25, 28.0] {
        cx.update_window(handle, |_, window, cx| {
            assert!(arrow_snapshot(&editor, cx).style.stroke_width >= 20.0);
            let position = slider_position(window, value);
            pointer_move(window, position, cx);
        })
        .unwrap();
    }
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(arrow_snapshot(&editor, cx).style.stroke_width, 28.0);
        assert_eq!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        pointer_up(window, point(px(10.0), px(150.0)), cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        let resized = arrow_snapshot(&editor, cx);
        window.press("ctrl-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), original);
        window.press("ctrl-shift-z", cx);
        assert_eq!(arrow_snapshot(&editor, cx), resized);
        window.press("ctrl-z", cx);
        window.press("ctrl-z", cx);
        assert!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .is_empty()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_cancels_size_drag_and_later_pointer_moves_cannot_restart_it(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    let original = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("a", cx);
            window.drag(point(px(470.0), px(350.0)), point(px(730.0), px(490.0)), cx);
            let original = arrow_snapshot(&editor, cx);
            let position = window.find(("slider-thumb", 0u32)).bounds().center();
            pointer_down(window, position, cx);
            original
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        let position = slider_position(window, 18.0);
        pointer_move(window, position, cx);
        let position = slider_position(window, 24.0);
        pointer_move(window, position, cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(arrow_snapshot(&editor, cx).style.stroke_width, 24.0);
        assert!(cx.has_active_drag());
        window.press("escape", cx);
        assert_eq!(arrow_snapshot(&editor, cx), original);
        assert!(!cx.has_active_drag());
        let position = slider_position(window, 30.0);
        pointer_move(window, position, cx);
        pointer_up(window, position, cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(arrow_snapshot(&editor, cx), original);
        window.press("ctrl-z", cx);
        assert!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .is_empty()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn arrow_palette_fits_minimum_window_in_both_themes_and_reaches_full_size_range(
    cx: &mut TestAppContext,
) {
    let (handle, editor) = editor_at_size(cx, 900.0, 560.0);
    for mode in [
        gpui_kit::component::ThemeMode::Dark,
        gpui_kit::component::ThemeMode::Light,
    ] {
        cx.update_window(handle, |_, window, cx| {
            gpui_kit::component::Theme::change(mode, Some(window), cx);
            window.press("a", cx);
            for id in 0usize..4 {
                let button = window.find(("arrow-variant", id));
                assert!(button.visible());
                assert!(button.bounds().right() <= px(900.0));
                assert!(button.bounds().left() >= px(0.0));
            }
            assert!(window.find("arrow-size-slider").visible());
            window.drag(point(px(290.0), px(190.0)), point(px(590.0), px(400.0)), cx);
        })
        .unwrap();
        for value in [1.0, 30.0] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let track = window.find("arrow-size-track").bounds();
                let offset = point(
                    px(if value == 1.0 {
                        0.5
                    } else {
                        track.size.width.as_f32() - 0.5
                    }),
                    track.size.height / 2.0,
                );
                window.click_at("arrow-size-track", offset, cx);
            })
            .unwrap();
            cx.update_window(handle, |_, _, cx| {
                let doc = editor.read(cx).document.as_ref().unwrap();
                assert_eq!(
                    doc.annotation(doc.selected().unwrap())
                        .unwrap()
                        .style
                        .stroke_width,
                    value
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn toolbar_menu_and_canvas_have_layout(cx: &mut TestAppContext) {
    let (handle, _) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("save").visible());
        assert!(window.find("copy").bounds().size.width > px(0.0));
        window.click("app-menu", cx);
        assert!(window.find("capture-area").visible());
        window.press("escape", cx);
        assert!(window.try_find("capture-area").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn image_drag_button_starts_a_drag(cx: &mut TestAppContext) {
    let (handle, _) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let button = window.find("drag-file").bounds();
        assert_eq!(button.size, size(px(44.0), px(26.0)));
        pointer_down(window, button.center(), cx);
        pointer_move(window, button.center() + point(px(10.0), px(0.0)), cx);
        assert!(cx.has_active_drag());
        cx.stop_active_drag(window);
    })
    .unwrap();
}

#[gpui_kit::test]
fn image_drag_without_a_document_is_disabled(cx: &mut TestAppContext) {
    let (handle, entity) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        entity.update(cx, |editor, cx| {
            editor.document = None;
            cx.notify();
        });
        window.render_frame(cx);
        let button = window.find("drag-file").bounds();
        pointer_down(window, button.center(), cx);
        pointer_move(window, button.center() + point(px(10.0), px(0.0)), cx);
        assert!(!cx.has_active_drag());
    })
    .unwrap();
}

#[gpui_kit::test]
fn image_drag_file_contains_the_current_export(cx: &mut TestAppContext) {
    let (handle, entity) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        entity.update(cx, |editor, cx| {
            let mut doc = Document::new(sniplet_core::demo_image(400, 300));
            doc.add_annotation(
                sniplet_core::AnnotationKind::Rectangle {
                    rect: sniplet_core::ImageRect::new(100.0, 80.0, 70.0, 50.0),
                },
                Default::default(),
            );
            doc.set_crop(sniplet_core::ImageRect::new(80.0, 60.0, 200.0, 150.0));
            doc.set_backdrop(sniplet_core::Backdrop {
                padding: 12,
                ..Default::default()
            });
            editor.load(doc, "drag fixture", cx);
        });
        window.render_frame(cx);
        for selection in [
            None,
            Some(sniplet_core::ImageRect::new(100.0, 80.0, 70.0, 50.0)),
        ] {
            entity.update(cx, |editor, _| editor.selection = selection);
            let editor = entity.read(cx);
            let expected = editor.export_pixels().unwrap();
            let gpui_kit::ExternalDragPayload::Files(files) = editor.export_drag_payload().unwrap();
            assert_eq!(files.entries().len(), 1);
            let (path, is_directory) = &files.entries()[0];
            assert!(!is_directory);
            assert_eq!(path.extension().unwrap(), "png");
            assert_eq!(image::open(path).unwrap().to_rgba8(), expected);
            std::fs::remove_file(path).unwrap();
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn annotation_resize_is_one_undo_step(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool", Tool::Rectangle as usize), cx);
        window.drag(point(px(470.0), px(332.0)), point(px(580.0), px(422.0)), cx);
        window.press("v", cx);
        window.drag(point(px(470.0), px(360.0)), point(px(470.0), px(360.0)), cx);
        window.drag(point(px(580.0), px(422.0)), point(px(650.0), px(472.0)), cx);
        let rect = editor.read(cx).document.as_ref().unwrap().annotations()[0]
            .kind
            .bounds();
        assert!((rect.width - 180.0).abs() < 0.01);
        assert!((rect.height - 140.0).abs() < 0.01);
        window.press("ctrl-z", cx);
        let rect = editor.read(cx).document.as_ref().unwrap().annotations()[0]
            .kind
            .bounds();
        assert!((rect.width - 110.0).abs() < 0.01);
        assert!((rect.height - 90.0).abs() < 0.01);
    })
    .unwrap();
}

#[gpui_kit::test]
fn floating_palette_updates_the_selected_annotation(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool", Tool::Rectangle as usize), cx);
        window.drag(point(px(470.0), px(332.0)), point(px(580.0), px(422.0)), cx);
        window.click(("color", 4usize), cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        assert_eq!(doc.annotations().len(), 1);
        assert_eq!(
            doc.annotations()[0].style.stroke,
            sniplet_core::Color::new(0, 122, 255, 255)
        );
        window.click("fill", cx);
        assert!(
            editor.read(cx).document.as_ref().unwrap().annotations()[0]
                .style
                .fill
                .is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn added_image_keeps_existing_document_and_undo(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |editor, cx| {
            editor.add_image(
                image::RgbaImage::from_pixel(20, 20, image::Rgba([20, 40, 60, 255])),
                "Pasted image",
                false,
                cx,
            )
        });
        assert_eq!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .dimensions()
                .width,
            400
        );
        assert_eq!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        window.render_frame(cx);
        window.press("ctrl-z", cx);
        assert!(
            editor
                .read(cx)
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .is_empty()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn repeat_before_area_capture_keeps_the_document_and_reports_the_error(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        let original = editor.read(cx).export_pixels().unwrap();
        editor.update(cx, |editor, cx| {
            editor.command(crate::editor::Command::Repeat, window, cx)
        });
        assert!(editor.read(cx).status.contains("Capture an area before"));
        assert_eq!(editor.read(cx).export_pixels().unwrap(), original);
        assert!(editor.read(cx).last_capture.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn added_capture_expands_canvas_and_undoes_in_one_step(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        let original = editor.read(cx).export_pixels().unwrap();
        let capture = image::RgbaImage::from_pixel(80, 340, image::Rgba([20, 40, 60, 255]));
        editor.update(cx, |editor, cx| {
            editor.add_image(capture.clone(), "Added capture", true, cx)
        });
        let rendered = editor.read(cx).export_pixels().unwrap();
        assert_eq!(rendered.dimensions(), (480, 340));
        assert_eq!(rendered.get_pixel(0, 0), original.get_pixel(0, 0));
        assert_eq!(rendered.get_pixel(479, 339), capture.get_pixel(79, 339));
        assert_eq!(rendered.get_pixel(0, 339).0, [0, 0, 0, 0]);
        window.render_frame(cx);
        window.press("ctrl-z", cx);
        assert_eq!(editor.read(cx).export_pixels().unwrap(), original);
        window.press("ctrl-shift-z", cx);
        assert_eq!(editor.read(cx).export_pixels().unwrap(), rendered);
    })
    .unwrap();
}

#[gpui_kit::test]
fn blur_family_changes_preserve_bounds_and_undo(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("b", cx);
        window.drag(point(px(470.0), px(332.0)), point(px(580.0), px(422.0)), cx);
        let before = editor.read(cx).document.as_ref().unwrap().annotations()[0].clone();
        assert!(matches!(
            before.kind,
            sniplet_core::AnnotationKind::Blur { .. }
        ));
        window.click("family-pixelate", cx);
        let after = &editor.read(cx).document.as_ref().unwrap().annotations()[0];
        assert!(matches!(
            after.kind,
            sniplet_core::AnnotationKind::Pixelate { .. }
        ));
        assert_eq!(after.kind.bounds(), before.kind.bounds());
        assert_eq!(after.style, before.style);
        window.press("ctrl-z", cx);
        assert!(matches!(
            editor.read(cx).document.as_ref().unwrap().annotations()[0].kind,
            sniplet_core::AnnotationKind::Blur { .. }
        ));
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_size_buttons_render_and_undo_each_change(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool", Tool::Text as usize), cx);
        window.drag(point(px(520.0), px(390.0)), point(px(520.0), px(390.0)), cx);
        window.click("annotation-text", cx);
        window.input("Resizable", cx);
        window.click("add-text", cx);

        let initial = editor.read(cx).export_pixels().unwrap();
        window.click("text-size-up", cx);
        let enlarged = editor.read(cx).export_pixels().unwrap();
        assert_ne!(enlarged, initial);
        let doc = editor.read(cx).document.as_ref().unwrap();
        let sniplet_core::AnnotationKind::Text { font_size, .. } = &doc.annotations()[0].kind
        else {
            panic!("expected text annotation")
        };
        assert!((*font_size - 26.0).abs() < f32::EPSILON);

        window.click("text-size-down", cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        let sniplet_core::AnnotationKind::Text { font_size, .. } = &doc.annotations()[0].kind
        else {
            panic!("expected text annotation")
        };
        assert!((*font_size - 24.0).abs() < f32::EPSILON);
        window.press("ctrl-z", cx);
        assert_eq!(editor.read(cx).export_pixels().unwrap(), enlarged);
        window.press("ctrl-z", cx);
        assert_eq!(editor.read(cx).export_pixels().unwrap(), initial);
    })
    .unwrap();
}

#[gpui_kit::test]
fn counter_buttons_clamp_and_set_the_next_value(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool", Tool::Counter as usize), cx);
        window.drag(point(px(520.0), px(390.0)), point(px(520.0), px(390.0)), cx);
        let before = editor.read(cx).export_pixels().unwrap();

        window.click("counter-value-down", cx);
        window.click("counter-value-down", cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        assert!(matches!(
            doc.annotations()[0].kind,
            sniplet_core::AnnotationKind::Counter { value: 0, .. }
        ));
        assert_ne!(editor.read(cx).export_pixels().unwrap(), before);

        window.drag(point(px(600.0), px(430.0)), point(px(600.0), px(430.0)), cx);
        window.click("counter-value-up", cx);
        window.drag(point(px(680.0), px(470.0)), point(px(680.0), px(470.0)), cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        let values = doc
            .annotations()
            .iter()
            .map(|annotation| match &annotation.kind {
                sniplet_core::AnnotationKind::Counter { value, .. } => *value,
                _ => panic!("expected counter annotation"),
            })
            .collect::<Vec<_>>();
        assert_eq!(values, [0, 2, 3]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn magnifier_circles_move_and_resize_independently_with_grouped_undo(cx: &mut TestAppContext) {
    let (handle, entity) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        entity.update(cx, |editor, cx| {
            let doc = editor.document.as_mut().unwrap();
            let id = doc.add_annotation(
                sniplet_core::AnnotationKind::Magnifier {
                    rect: sniplet_core::ImageRect::new(200.0, 100.0, 90.0, 90.0),
                    zoom: 3.0,
                    source: Some(sniplet_core::Point::new(60.0, 60.0)),
                },
                Default::default(),
            );
            doc.select(Some(id)).unwrap();
            editor.tool = Tool::Zoom;
            cx.notify();
        });
        window.render_frame(cx);
        let snapshot = |cx: &gpui_kit::App| {
            entity.read(cx).document.as_ref().unwrap().annotations()[0].clone()
        };
        let original = snapshot(cx);
        let lens = window.find(("magnifier-handle", 2usize)).bounds().center();
        window.drag(lens, lens + point(px(35.0), px(20.0)), cx);
        let moved = snapshot(cx);
        let before = original.kind.magnifier_circles().unwrap();
        let after = moved.kind.magnifier_circles().unwrap();
        assert_eq!(before[0], after[0]);
        assert_eq!(after[1].x, before[1].x + 35.0);
        assert_eq!(after[1].y, before[1].y + 20.0);
        window.press("ctrl-z", cx);
        assert_eq!(snapshot(cx), original);
        window.press("ctrl-shift-z", cx);
        assert_eq!(snapshot(cx), moved);

        let source = window.find(("magnifier-handle", 0usize)).bounds().center();
        window.drag(source, source + point(px(20.0), px(15.0)), cx);
        let resampled = snapshot(cx);
        assert_eq!(resampled.kind.magnifier_circles().unwrap()[1], after[1]);
        assert_ne!(resampled.kind.magnifier_circles().unwrap()[0], after[0]);
        window.press("ctrl-z", cx);
        assert_eq!(snapshot(cx), moved);

        for index in [1usize, 3] {
            let edge = window.find(("magnifier-handle", index)).bounds().center();
            window.drag(edge, edge + point(px(8.0), px(0.0)), cx);
            let resized = snapshot(cx);
            assert_ne!(resized, moved);
            let circles = resized.kind.magnifier_circles().unwrap();
            for circle in circles {
                assert_eq!(circle.width, circle.height);
            }
            window.press("ctrl-z", cx);
            assert_eq!(snapshot(cx), moved);
        }
        let lens = window.find(("magnifier-handle", 2usize)).bounds().center();
        pointer_down(window, lens, cx);
        pointer_move(window, lens + point(px(18.0), px(0.0)), cx);
        window.press("escape", cx);
        assert_eq!(snapshot(cx), moved);
    })
    .unwrap();
}

#[gpui_kit::test]
fn magnifier_and_spotlight_property_buttons_change_rendering(cx: &mut TestAppContext) {
    let (handle, editor) = editor(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool", Tool::Zoom as usize), cx);
        window.drag(point(px(500.0), px(350.0)), point(px(620.0), px(450.0)), cx);
        let magnified_thrice = editor.read(cx).export_pixels().unwrap();
        window.click("magnifier-zoom-up", cx);
        let magnified_four_times = editor.read(cx).export_pixels().unwrap();
        assert_ne!(magnified_four_times, magnified_thrice);
        let doc = editor.read(cx).document.as_ref().unwrap();
        let sniplet_core::AnnotationKind::Magnifier { zoom, .. } = &doc.annotations()[0].kind
        else {
            panic!("expected magnifier annotation")
        };
        assert!((*zoom - 4.0).abs() < f32::EPSILON);
        window.click("magnifier-zoom-down", cx);
        let doc = editor.read(cx).document.as_ref().unwrap();
        let sniplet_core::AnnotationKind::Magnifier { zoom, .. } = &doc.annotations()[0].kind
        else {
            panic!("expected magnifier annotation")
        };
        assert!((*zoom - 3.0).abs() < f32::EPSILON);

        window.press("s", cx);
        window.drag(point(px(540.0), px(380.0)), point(px(650.0), px(470.0)), cx);
        for _ in 0..6 {
            window.click("spotlight-dim-down", cx);
        }
        let low_dim = editor.read(cx).export_pixels().unwrap();
        let doc = editor.read(cx).document.as_ref().unwrap();
        assert_eq!(
            doc.annotations()[1].style.fill,
            Some(sniplet_core::Color::new(0, 0, 0, 25))
        );

        for _ in 0..9 {
            window.click("spotlight-dim-up", cx);
        }
        let high_dim = editor.read(cx).export_pixels().unwrap();
        let doc = editor.read(cx).document.as_ref().unwrap();
        assert_eq!(
            doc.annotations()[1].style.fill,
            Some(sniplet_core::Color::new(0, 0, 0, 225))
        );
        let low_corner = low_dim.get_pixel(0, 0).0[..3]
            .iter()
            .map(|channel| u32::from(*channel))
            .sum::<u32>();
        let high_corner = high_dim.get_pixel(0, 0).0[..3]
            .iter()
            .map(|channel| u32::from(*channel))
            .sum::<u32>();
        assert!(high_corner < low_corner);
    })
    .unwrap();
}
