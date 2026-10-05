use image::{Rgba, RgbaImage};
use sniplet_core::{
    AnnotationKind, AnnotationStyle, Document, ImageRect, MeasurementAxis, Point, Project,
    RenderOptions, measure_at,
};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSans.ttf");

fn fixture() -> RgbaImage {
    RgbaImage::from_fn(400, 300, |x, y| {
        if (78..242).contains(&x) && (78..142).contains(&y) {
            if (80..240).contains(&x) && (80..140).contains(&y) {
                Rgba([80, 150, 220, 255])
            } else {
                Rgba([150, 150, 150, 255])
            }
        } else {
            Rgba([255, 255, 255, 255])
        }
    })
}

#[test]
fn measures_inner_and_outer_sizes_without_an_extra_endpoint_pixel() {
    let image = fixture();
    for (axis, inner, outer) in [
        (MeasurementAxis::Horizontal, 160.0, 164.0),
        (MeasurementAxis::Vertical, 60.0, 64.0),
    ] {
        let inner_span = measure_at(&image, Point::new(120.0, 100.0), axis, 20, false).unwrap();
        let outer_span = measure_at(&image, Point::new(120.0, 100.0), axis, 20, true).unwrap();
        let length = |span: sniplet_core::Measurement| {
            (span.end.x - span.start.x).hypot(span.end.y - span.start.y)
        };
        assert_eq!(length(inner_span), inner);
        assert_eq!(length(outer_span), outer);
        let mut retina = inner_span;
        retina.pixels_per_unit = 2.0;
        retina.scale_factor = 2.0;
        assert_eq!(retina.label(), format!("{:.0}px", inner / 2.0));
        let logical = retina.overlay(FONT).unwrap();
        retina.pixels_per_unit = 1.0;
        assert_eq!(retina.label(), format!("{inner:.0}px"));
        assert_eq!(retina.overlay(FONT).unwrap().1.height(), logical.1.height());
    }
    let borderless = RgbaImage::from_fn(400, 300, |x, y| {
        if (80..240).contains(&x) && (80..140).contains(&y) {
            Rgba([80, 150, 220, 255])
        } else {
            Rgba([255, 255, 255, 255])
        }
    });
    assert_eq!(
        measure_at(
            &borderless,
            Point::new(120.0, 100.0),
            MeasurementAxis::Horizontal,
            20,
            true
        )
        .unwrap()
        .label(),
        "160px"
    );
}

#[test]
fn gap_and_sensitivity_follow_nearest_visible_color_edges() {
    let image = RgbaImage::from_fn(100, 10, |x, _| {
        let gray = if !(20..70).contains(&x) {
            80
        } else if x < 45 {
            240
        } else {
            250
        };
        Rgba([gray, gray, gray, 255])
    });
    let gap = measure_at(
        &image,
        Point::new(40.0, 5.0),
        MeasurementAxis::Horizontal,
        20,
        false,
    )
    .unwrap();
    assert_eq!(
        (gap.start.x, gap.end.x, gap.label()),
        (20.0, 70.0, "50px".into())
    );
    let sensitive = measure_at(
        &image,
        Point::new(40.0, 5.0),
        MeasurementAxis::Horizontal,
        5,
        false,
    )
    .unwrap();
    assert_eq!((sensitive.start.x, sensitive.end.x), (20.0, 45.0));
    let gradient = RgbaImage::from_fn(100, 10, |x, _| Rgba([x as u8, 0, 0, 255]));
    assert_eq!(
        measure_at(
            &gradient,
            Point::new(50.0, 5.0),
            MeasurementAxis::Horizontal,
            5,
            false
        )
        .unwrap()
        .label(),
        "100px"
    );
    for point in [
        Point::new(-1.0, 0.0),
        Point::new(100.0, 5.0),
        Point::new(f32::NAN, 0.0),
    ] {
        assert!(measure_at(&image, point, MeasurementAxis::Horizontal, 20, false).is_none());
    }
    assert!(
        measure_at(
            &RgbaImage::new(100, 10),
            Point::new(50.0, 5.0),
            MeasurementAxis::Horizontal,
            20,
            false
        )
        .is_none()
    );
}

#[test]
fn measurement_source_includes_raster_edits_and_excludes_image_objects_and_drawings() {
    let source = fixture();
    let mut document = Document::new(source.clone());
    let mut png = std::io::Cursor::new(Vec::new());
    RgbaImage::from_pixel(80, 80, Rgba([0, 0, 0, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    document.add_annotation(
        AnnotationKind::Image {
            origin: Point::new(0.0, 0.0),
            png_bytes: png.into_inner(),
            size: None,
        },
        AnnotationStyle::default(),
    );
    document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(0.0, 0.0, 50.0, 50.0),
        },
        AnnotationStyle::default(),
    );
    document.set_crop(ImageRect::new(0.0, 0.0, 20.0, 20.0));
    assert_eq!(
        document.render_raster(&RenderOptions::default()).unwrap(),
        source
    );
    document.add_annotation(
        AnnotationKind::Redaction {
            rect: ImageRect::new(80.0, 80.0, 160.0, 60.0),
        },
        AnnotationStyle::default(),
    );
    assert_ne!(
        document
            .render_raster(&RenderOptions::default())
            .unwrap()
            .get_pixel(100, 100),
        source.get_pixel(100, 100)
    );
}

#[test]
fn imprint_keeps_caps_label_export_project_and_undo_together() {
    let source = fixture();
    let mut document = Document::new(source.clone());
    document.set_source_scale_factor(2.0);
    let measurement = measure_at(
        &source,
        Point::new(120.0, 100.0),
        MeasurementAxis::Horizontal,
        20,
        false,
    )
    .unwrap();
    document.add_annotation(
        AnnotationKind::Measurement { measurement },
        AnnotationStyle::default(),
    );
    let options = RenderOptions {
        font_bytes: Some(FONT),
        ..Default::default()
    };
    let output = document.render(&options).unwrap();
    let mut retina_label = measurement;
    retina_label.scale_factor = 2.0;
    let (bounds, label_overlay) = retina_label.overlay(FONT).unwrap();
    for (_, y, pixel) in label_overlay.enumerate_pixels() {
        if pixel[3] > 0 && pixel[1] > 60 && pixel[2] > 50 {
            assert!(
                bounds.y + (y as f32) < measurement.start.y - 8.0,
                "label text must stay inside the red pill"
            );
        }
    }
    assert_eq!(output.get_pixel(120, 100).0, [255, 59, 48, 255]);
    assert_ne!(output.get_pixel(80, 97), source.get_pixel(80, 97));
    assert!(
        (80..100)
            .any(|y| (140..180).any(|x| output.get_pixel(x, y).0[..3].iter().all(|c| *c > 220))),
        "the red label must contain white text"
    );
    assert!(document.hit_test(Point::new(120.0, 100.0), 2.0).is_none());
    let project =
        Project::from_json(&document.to_project("fixture.png").to_json_pretty().unwrap()).unwrap();
    assert_eq!(project.source_scale_factor, Some(2.0));
    assert_eq!(
        Document::from_project(project, source.clone())
            .unwrap()
            .render(&options)
            .unwrap(),
        output
    );
    assert!(document.undo());
    assert_eq!(document.render(&options).unwrap(), source);
    assert!(document.redo());
    assert_eq!(document.render(&options).unwrap(), output);
}
