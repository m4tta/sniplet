//! Measuring, color sampling, the camera, the recognized text dialog and the smallest window.
use image::{Rgba, RgbaImage};
use rune::{Mods, WheelEvent};
use sniplet_core::{AnnotationKind, Color, Document, ImageRect};

use super::{Harness, WINDOW, placed_ruler, shift};
use crate::tools::Tool;

fn harness(image: RgbaImage) -> Harness {
    Harness::new(Document::new(image), WINDOW, true)
}

#[test]
fn a_held_measure_key_previews_and_a_click_places_one_ruler() {
    let mut image = RgbaImage::from_pixel(400, 300, Rgba([255, 255, 255, 255]));
    for y in 80..140 {
        for x in 80..240 {
            image.put_pixel(x, y, Rgba([80, 150, 220, 255]));
        }
    }
    let mut h = harness(image);
    h.update(|e| e.set_measure_scale(1.0));
    let before = h.export();
    let inside = h.at(180.0, 110.0);
    h.move_to(inside);
    h.press("right");
    h.repeat("right");
    assert!(h.shows("measure-preview"), "a held arrow key shows a ruler");
    assert!(h.annotations().is_empty());
    assert_eq!(h.export(), before);

    h.down(inside);
    h.up(inside);
    assert_eq!(h.annotations().len(), 1);
    assert_eq!(placed_ruler(&h).label(), "160px");
    assert_ne!(h.export(), before);
    h.release("right");
    assert!(!h.shows("measure-preview"));
    h.undo();
    assert_eq!(h.export(), before);
    h.redo();
    assert_eq!(h.annotations().len(), 1);
}

#[test]
fn measure_keys_shift_units_and_wheel_change_the_placed_ruler() {
    let image = RgbaImage::from_fn(400, 300, |x, y| {
        if (78..242).contains(&x) && (78..142).contains(&y) {
            if (80..240).contains(&x) && (80..140).contains(&y) {
                Rgba([80, 150, 220, 255])
            } else {
                Rgba([150, 150, 150, 255])
            }
        } else if (30..50).contains(&x) {
            Rgba([240, 240, 240, 255])
        } else {
            Rgba([255, 255, 255, 255])
        }
    });
    let mut h = harness(image);
    h.update(|e| e.set_measure_scale(2.0));
    h.click(&format!("tool-{}", Tool::Ruler as usize));
    assert_eq!(h.editor().tool, Tool::Ruler);
    assert!(h.shows("measure-units"));
    let inside = h.at(180.0, 110.0);
    let place = |h: &mut Harness| {
        h.down(inside);
        h.up(inside);
    };

    h.move_to(inside);
    h.press("2");
    place(&mut h);
    assert_eq!(placed_ruler(&h).label(), "30px");
    h.release("2");
    h.undo();

    // Shift includes the outer edges; the units button switches to physical pixels.
    h.press("1");
    h.hold(shift());
    place(&mut h);
    assert_eq!(placed_ruler(&h).label(), "82px");
    h.click("measure-units");
    place(&mut h);
    assert_eq!(placed_ruler(&h).label(), "164px");
    h.hold(Mods::default());
    h.release("1");
    h.undo();
    h.undo();
    h.click("measure-units");
    assert!(h.annotations().is_empty());

    // The wheel lowers the tolerance, so the faint stripe stops the ruler.
    let gap = h.at(60.0, 110.0);
    h.move_to(gap);
    h.press("right");
    for _ in 0..2 {
        h.rt.wheel_event(WheelEvent::new(0.0, -1.0).at(gap.0, gap.1));
        h.settle();
    }
    h.down(gap);
    h.up(gap);
    assert_eq!(placed_ruler(&h).label(), "14px");
    h.release("right");

    h.press("left");
    assert!(h.shows("measure-preview"));
    h.press("escape");
    assert!(!h.shows("measure-preview"));
    assert_eq!(h.annotations().len(), 1);
}

#[test]
fn measuring_stays_inside_the_crop_and_arrows_nudge_a_selected_annotation() {
    let mut doc = Document::new(RgbaImage::from_pixel(400, 300, Rgba([80, 150, 220, 255])));
    doc.set_crop(ImageRect::new(100.0, 80.0, 100.0, 60.0));
    let mut h = Harness::new(doc, WINDOW, true);
    h.update(|e| e.set_measure_scale(1.0));
    h.move_to(h.at(150.0, 110.0));
    h.press("right");
    let inside = h.at(150.0, 110.0);
    h.down(inside);
    h.up(inside);
    let span = placed_ruler(&h);
    assert_eq!(
        (span.start.x, span.end.x, span.label()),
        (100.0, 200.0, "100px".into())
    );
    h.release("right");

    let mut doc = Document::new(sniplet_core::demo_image(400, 300));
    let id = doc.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(20.0, 20.0, 40.0, 40.0),
        },
        Default::default(),
    );
    doc.select(Some(id)).unwrap();
    h.update(|e| e.load(doc, "Selected object"));
    h.press("shift-right");
    assert_eq!(h.only().kind.bounds().x, 30.0);
    assert!(!h.shows("measure-preview"));
}

#[test]
fn the_pointer_samples_colors_and_a_new_preview_resamples_it() {
    let source = RgbaImage::from_pixel(400, 300, Rgba([64, 123, 200, 255]));
    let mut h = harness(source.clone());
    h.move_to(h.at(200.0, 150.0));
    assert_eq!(h.editor().sampled_color(), Color::new(64, 123, 200, 255));
    h.press("tab");
    assert!(h.shows("color-peek"));
    h.release("tab");
    assert!(!h.shows("color-peek"));

    let mut redacted = Document::new(source);
    redacted.add_annotation(
        AnnotationKind::Redaction {
            rect: ImageRect::new(0.0, 0.0, 400.0, 300.0),
        },
        Default::default(),
    );
    h.update(|e| e.load(redacted, "Redacted"));
    assert_eq!(h.editor().sampled_color(), Color::BLACK);

    // C with a selection takes the selection's average color.
    let halves = RgbaImage::from_fn(400, 300, |x, _| {
        if x < 200 {
            Rgba([200, 0, 0, 255])
        } else {
            Rgba([0, 0, 100, 255])
        }
    });
    h.update(|e| e.load(Document::new(halves), "Halves"));
    h.drag_image((100.0, 0.0), (300.0, 300.0));
    h.press("c");
    assert_eq!(h.editor().tool, Tool::Select, "C with a selection samples");
    assert_eq!(h.editor().sampled_color(), Color::new(100, 0, 50, 255));
    assert!(h.shows("color-peek"));
    h.release("c");
}

#[test]
fn zoom_keys_wheel_and_the_zoom_button_move_the_camera() {
    let mut h = harness(RgbaImage::from_pixel(2000, 1200, Rgba([90, 90, 90, 255])));
    let zoom = |h: &Harness| h.editor().transform().zoom();
    let fit = zoom(&h);
    assert!(fit < 1.0 && h.editor().fit);
    let preview = h.rect("preview");
    assert!((preview[2] - 2000.0 * fit).abs() < 0.5);

    h.press("ctrl-0");
    assert_eq!(zoom(&h), 1.0);
    h.press("ctrl-=");
    assert_eq!(zoom(&h), 1.25);
    h.press("ctrl-minus");
    assert_eq!(zoom(&h), 1.0);
    h.click("zoom-fit");
    assert!(h.editor().fit);
    assert_eq!(zoom(&h), fit);

    // Ctrl + wheel zooms in around the pointer.
    let point = h.at(1000.0, 600.0);
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Default::default()
    };
    h.rt.wheel_event(WheelEvent::new(0.0, 100.0).mods(ctrl).at(point.0, point.1));
    h.settle();
    assert!(zoom(&h) > fit);
    let (canvas, _) = h.rt.presented("editor-canvas").unwrap();
    let under = h
        .editor()
        .image_point(point.0 - canvas[0], point.1 - canvas[1]);
    assert!((under.x - 1000.0).abs() < 0.5 && (under.y - 600.0).abs() < 0.5);

    // Holding Z makes a click zoom in two times there.
    let before = zoom(&h);
    h.press("z");
    h.click_image(1000.0, 600.0);
    h.release("z");
    assert!((zoom(&h) - before * 2.0).abs() < 0.001);
    assert!(h.annotations().is_empty() && h.editor().selection.is_none());

    h.press("ctrl-1");
    assert_eq!(zoom(&h), fit);
}

/// A white 600 × 300 image with two QR codes: "Selected text" on the left and "Outside
/// selection" on the right.
fn qr_codes() -> RgbaImage {
    let mut image = RgbaImage::from_pixel(600, 300, Rgba([255, 255, 255, 255]));
    for (x, text) in [(20, "Selected text"), (350, "Outside selection")] {
        let qr = qrcode::QrCode::new(text.as_bytes())
            .unwrap()
            .render::<image::Luma<u8>>()
            .min_dimensions(160, 160)
            .build();
        assert!(qr.width() < 220);
        image::imageops::overlay(
            &mut image,
            &image::DynamicImage::ImageLuma8(qr).to_rgba8(),
            x,
            40,
        );
    }
    image
}

#[test]
fn the_ocr_tool_reads_only_its_region_into_a_dialog_that_opens_and_closes() {
    let mut h = harness(qr_codes());
    let before = h.export();
    h.click(&format!("tool-{}", Tool::Ocr as usize));
    h.drag_image((10.0, 30.0), (240.0, 260.0));
    assert_eq!(
        h.editor().selection,
        Some(ImageRect::new(10.0, 30.0, 230.0, 230.0))
    );
    let text = |h: &Harness| h.editor().recognized_text.as_ref().map(|e| e.text.clone());
    assert_eq!(text(&h).as_deref(), Some("Selected text"));
    assert!(h.shows("recognized-text-dialog"));
    // The dialog sits in the middle of the window, over a scrim that covers it all.
    let dialog = h.rect("recognized-text-dialog");
    assert!((dialog[0] + dialog[2] / 2.0 - WINDOW.0 / 2.0).abs() < 1.0);
    assert!((dialog[1] + dialog[3] / 2.0 - WINDOW.1 / 2.0).abs() < 1.0);
    assert_eq!(
        h.rect("recognized-text-scrim"),
        [0.0, 0.0, WINDOW.0, WINDOW.1]
    );
    // Reading changes nothing in the document.
    assert!(h.annotations().is_empty() && h.document().crop().is_none());
    assert!(!h.document().can_undo());
    assert_eq!(
        h.document()
            .render(&crate::editor::render_options())
            .unwrap(),
        before
    );

    // The text is editable, and editor keys stay out of it.
    h.press("ctrl-a");
    h.type_text("Edited");
    h.press("enter");
    h.type_text("Second line");
    h.press("right");
    assert_eq!(
        text(&h).as_deref(),
        Some(
            "Edited
Second line"
        )
    );
    assert!(!h.shows("measure-preview"));
    h.press("escape");
    assert!(!h.shows("recognized-text-dialog"));

    // Ctrl/Cmd O reads the selection again; the close button closes it.
    h.press("ctrl-o");
    assert_eq!(text(&h).as_deref(), Some("Selected text"));
    h.click("close-recognized-text");
    assert!(!h.shows("recognized-text-dialog"));

    // A click without a region does not start reading the whole image.
    h.click_image(300.0, 150.0);
    assert!(h.editor().selection.is_none() && h.editor().recognized_text.is_none());
}

#[test]
fn the_smallest_window_fits_the_toolbar_dock_and_arrow_palette_in_both_themes() {
    for dark in [true, false] {
        let mut h = Harness::new(
            Document::new(sniplet_core::demo_image(400, 300)),
            (900.0, 560.0),
            dark,
        );
        let inside = |h: &Harness, key: &str| {
            let r = h.rect(key);
            assert!(
                r[0] >= 0.0 && r[1] >= 0.0 && r[0] + r[2] <= 900.0 && r[1] + r[3] <= 560.0,
                "{key} at {r:?} leaves the window (dark {dark})"
            );
            r
        };
        let tools = Tool::ALL.map(|tool| format!("tool-{}", tool as usize));
        for key in [
            "app-menu",
            "zoom-fit",
            "backdrop",
            "drag-file",
            "pin",
            "save",
            "copy",
        ]
        .map(String::from)
        .iter()
        .chain(&tools)
        {
            inside(&h, key);
        }

        h.press("a");
        for variant in 0..4 {
            inside(&h, &format!("arrow-variant-{variant}"));
        }
        let slider = inside(&h, "size-slider");
        let properties = inside(&h, "properties");
        assert!(properties[1] + properties[3] <= h.rect("tool-dock")[1]);

        // The slider's ends reach the whole range.
        h.drag_image((60.0, 40.0), (300.0, 200.0));
        for (x, width) in [(slider[0] + 1.0, 1.0), (slider[0] + slider[2] - 1.0, 30.0)] {
            h.click_at((x, slider[1] + slider[3] / 2.0));
            assert_eq!(h.only().style.stroke_width, width, "dark {dark}");
        }
    }
}

#[test]
fn the_dragged_file_holds_the_export_of_the_image_or_the_selection() {
    let mut doc = Document::new(sniplet_core::demo_image(400, 300));
    doc.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(100.0, 80.0, 70.0, 50.0),
        },
        Default::default(),
    );
    doc.set_crop(ImageRect::new(80.0, 60.0, 200.0, 150.0));
    let mut h = Harness::new(doc, WINDOW, true);
    for selection in [None, Some(ImageRect::new(100.0, 80.0, 70.0, 50.0))] {
        h.update(|e| e.selection = selection);
        let path = h.editor().export_drag_file().unwrap();
        assert_eq!(path.extension().unwrap(), "png");
        assert_eq!(image::open(&path).unwrap().to_rgba8(), h.export());
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn the_backdrop_panel_scrolls_to_its_last_controls_in_the_smallest_window() {
    let mut h = Harness::new(
        Document::new(sniplet_core::demo_image(400, 300)),
        (900.0, 560.0),
        true,
    );
    h.click("backdrop");
    let panel = h.rect("floating-panel");
    assert!(
        panel[1] + panel[3] <= 560.0,
        "the panel fits the window: {panel:?}"
    );
    let below = |h: &Harness| {
        let r = h.rect("toggle-shadow");
        r[1] + r[3] > panel[1] + panel[3]
    };
    assert!(below(&h), "this window is short enough to need scrolling");
    let (x, y) = (panel[0] + panel[2] / 2.0, panel[1] + panel[3] / 2.0);
    for _ in 0..20 {
        if !below(&h) {
            break;
        }
        h.rt.wheel_event(WheelEvent::new(0.0, -60.0).at(x, y));
        h.settle();
    }
    assert!(!below(&h), "scrolling brings the last control into view");
    let shadow = h.document().backdrop().shadow.is_some();
    h.click("toggle-shadow");
    assert_ne!(h.document().backdrop().shadow.is_some(), shadow);
}
