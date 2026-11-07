//! The demo document (`--demo`) and the native self-test (`--self-test`).
use std::path::PathBuf;

use sniplet_core::{
    AnnotationKind, AnnotationStyle, Color, Document, ImageRect, Point as ImagePoint,
};

use crate::editor::render_options;

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

pub fn run(output: PathBuf) -> anyhow::Result<()> {
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
