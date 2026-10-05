use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder};
use sniplet_core::{
    AnnotationKind, AnnotationStyle, Color, Document, ImageRect, Point, RenderOptions,
};

fn png(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .unwrap();
    bytes
}

#[test]
fn raster_tools_preserve_drawings_and_image_objects() {
    let rect = ImageRect::new(2.0, 2.0, 20.0, 20.0);
    for kind in [
        AnnotationKind::Blur { rect, radius: 3.0 },
        AnnotationKind::Pixelate {
            rect,
            block_size: 7,
        },
        AnnotationKind::RemoveFill {
            rect,
            sample: Some(Point::new(0.0, 0.0)),
        },
        AnnotationKind::Redaction { rect },
    ] {
        let source = RgbaImage::from_fn(32, 24, |x, y| {
            let value = if (x + y) % 2 == 0 { 0 } else { 255 };
            Rgba([value, value, value, 255])
        });
        let mut document = Document::new(source.clone());
        let line = document.add_annotation(
            AnnotationKind::Line {
                start: Point::new(1.0, 6.0),
                end: Point::new(30.0, 6.0),
            },
            AnnotationStyle {
                stroke_width: 3.0,
                ..Default::default()
            },
        );
        document.add_annotation(
            AnnotationKind::Image {
                origin: Point::new(10.0, 10.0),
                png_bytes: png(&RgbaImage::from_pixel(3, 3, Rgba([0, 220, 80, 255]))),
                size: None,
            },
            AnnotationStyle::default(),
        );
        document.add_annotation(
            kind.clone(),
            AnnotationStyle {
                stroke: Color::BLACK,
                ..Default::default()
            },
        );

        let rendered = document.render(&RenderOptions::default()).unwrap();
        let red: Rgba<u8> = Color::RED.into();
        assert_eq!(*rendered.get_pixel(8, 6), red, "{kind:?} changed a drawing");
        assert_eq!(
            rendered.get_pixel(11, 11).0,
            [0, 220, 80, 255],
            "{kind:?} changed an image object"
        );
        assert_ne!(
            rendered.get_pixel(2, 3),
            source.get_pixel(2, 3),
            "{kind:?} did not change the base pixels"
        );
        assert_eq!(document.hit_test(Point::new(8.0, 6.0), 0.0), Some(line));
        assert_eq!(document.original(), &source);
    }
}

#[test]
fn pasted_images_stay_below_drawings_and_spotlight_shading() {
    let mut document = Document::new(RgbaImage::from_pixel(32, 24, Rgba([255, 255, 255, 255])));
    let line = document.add_annotation(
        AnnotationKind::Line {
            start: Point::new(4.0, 7.0),
            end: Point::new(11.0, 7.0),
        },
        AnnotationStyle {
            stroke_width: 3.0,
            ..Default::default()
        },
    );
    document.add_annotation(
        AnnotationKind::Image {
            origin: Point::new(4.0, 4.0),
            png_bytes: png(&RgbaImage::from_pixel(8, 8, Rgba([0, 120, 220, 255]))),
            size: None,
        },
        AnnotationStyle::default(),
    );
    document.add_annotation(
        AnnotationKind::Spotlight {
            rect: ImageRect::new(20.0, 18.0, 4.0, 4.0),
        },
        AnnotationStyle::default(),
    );

    let rendered = document.render(&RenderOptions::default()).unwrap();
    let red: Rgba<u8> = Color::RED.into();
    assert_eq!(*rendered.get_pixel(8, 7), red);
    assert!(
        rendered.get_pixel(6, 4).0[2] < 220,
        "Spotlight must shade pasted images"
    );
    assert_eq!(rendered.get_pixel(21, 19).0, [255, 255, 255, 255]);
    assert_eq!(document.hit_test(Point::new(8.0, 7.0), 0.0), Some(line));
}

#[test]
fn multiple_spotlights_expose_the_union_and_dim_everything_else() {
    let mut document = Document::new(RgbaImage::from_pixel(20, 10, Rgba([255, 255, 255, 255])));
    for rect in [
        ImageRect::new(2.0, 2.0, 4.0, 4.0),
        ImageRect::new(14.0, 2.0, 4.0, 4.0),
    ] {
        document.add_annotation(
            AnnotationKind::Spotlight { rect },
            AnnotationStyle::default(),
        );
    }
    let rendered = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(rendered.get_pixel(3, 3).0, [255, 255, 255, 255]);
    assert_eq!(rendered.get_pixel(15, 3).0, [255, 255, 255, 255]);
    assert!(rendered.get_pixel(10, 3).0[0] < 150);
}

#[test]
fn pasted_png_has_bounds_hit_testing_movement_and_alpha_compositing() {
    let pasted = RgbaImage::from_pixel(3, 2, Rgba([10, 100, 220, 255]));
    let mut document = Document::new(RgbaImage::from_pixel(20, 20, Rgba([255, 255, 255, 255])));
    let id = document.add_annotation(
        AnnotationKind::Image {
            origin: Point::new(4.0, 5.0),
            png_bytes: png(&pasted),
            size: None,
        },
        AnnotationStyle::default(),
    );
    assert_eq!(
        document.annotation(id).unwrap().kind.bounds(),
        ImageRect::new(4.0, 5.0, 3.0, 2.0)
    );
    assert_eq!(document.hit_test(Point::new(5.0, 6.0), 0.0), Some(id));
    document.move_annotation(id, Point::new(2.0, 3.0)).unwrap();
    let rendered = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(rendered.get_pixel(6, 8).0, [10, 100, 220, 255]);
    assert_eq!(rendered.get_pixel(4, 5).0, [255, 255, 255, 255]);

    document
        .resize_annotation(id, ImageRect::new(8.0, 9.0, 6.0, 4.0))
        .unwrap();
    assert_eq!(
        document.annotation(id).unwrap().kind.bounds(),
        ImageRect::new(8.0, 9.0, 6.0, 4.0)
    );
    let scaled = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(scaled.get_pixel(13, 12).0, [10, 100, 220, 255]);
    assert_eq!(scaled.get_pixel(14, 12).0, [255, 255, 255, 255]);
}

#[test]
fn linked_magnifier_keeps_its_source_when_the_lens_moves() {
    let mut source = RgbaImage::from_pixel(120, 80, Rgba([255, 255, 255, 255]));
    for y in 17..24 {
        for x in 17..24 {
            source.put_pixel(x, y, Rgba([0, 120, 240, 255]));
        }
    }
    let mut document = Document::new(source);
    let style = AnnotationStyle {
        stroke: Color::RED,
        stroke_width: 2.0,
        ..Default::default()
    };
    let id = document.add_annotation(
        AnnotationKind::Magnifier {
            rect: ImageRect::new(60.0, 5.0, 30.0, 30.0),
            zoom: 3.0,
            source: Some(Point::new(20.0, 20.0)),
        },
        style,
    );
    let rendered = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(rendered.get_pixel(75, 20).0, [0, 120, 240, 255]);
    assert_eq!(*rendered.get_pixel(25, 20), Rgba::from(Color::RED));
    assert_eq!(*rendered.get_pixel(40, 20), Rgba::from(Color::RED));
    assert_eq!(document.hit_test(Point::new(20.0, 20.0), 0.0), Some(id));
    assert_eq!(document.hit_test(Point::new(40.0, 20.0), 0.0), Some(id));
    assert_eq!(document.hit_test(Point::new(60.0, 5.0), 0.0), None);

    document
        .update_annotation(
            id,
            AnnotationKind::Magnifier {
                rect: ImageRect::new(80.0, 45.0, 30.0, 30.0),
                zoom: 3.0,
                source: Some(Point::new(20.0, 20.0)),
            },
            style,
        )
        .unwrap();
    let moved = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(moved.get_pixel(95, 60).0, [0, 120, 240, 255]);
    document.undo();
    assert_eq!(
        document.render(&RenderOptions::default()).unwrap(),
        rendered
    );

    let json = document.to_project("source.png").to_json_pretty().unwrap();
    let project = sniplet_core::Project::from_json(&json).unwrap();
    assert_eq!(project.annotations, document.annotations());
}

#[test]
fn magnifier_enlarges_source_pixels_inside_an_elliptical_lens() {
    let mut source = RgbaImage::from_pixel(20, 20, Rgba([255, 255, 255, 255]));
    for y in 9..11 {
        for x in 9..11 {
            source.put_pixel(x, y, Rgba([0, 120, 240, 255]));
        }
    }
    let mut document = Document::new(source);
    document.add_annotation(
        AnnotationKind::Magnifier {
            rect: ImageRect::new(5.0, 5.0, 10.0, 10.0),
            zoom: 5.0,
            source: None,
        },
        AnnotationStyle {
            stroke: Color::BLACK,
            fill: None,
            stroke_width: 1.0,
        },
    );
    let rendered = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(rendered.get_pixel(7, 10).0, [0, 120, 240, 255]);
    assert_eq!(rendered.get_pixel(4, 10).0, [255, 255, 255, 255]);
    assert!(document.hit_test(Point::new(10.0, 10.0), 0.0).is_some());
}

#[test]
fn new_annotation_variants_round_trip_through_project_json() {
    let pasted = png(&RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])));
    let mut document = Document::new(RgbaImage::new(10, 10));
    document.add_annotation(
        AnnotationKind::Image {
            origin: Point::new(2.0, 3.0),
            png_bytes: pasted,
            size: None,
        },
        AnnotationStyle::default(),
    );
    document.add_annotation(
        AnnotationKind::Spotlight {
            rect: ImageRect::new(1.0, 1.0, 4.0, 4.0),
        },
        AnnotationStyle::default(),
    );
    document.add_annotation(
        AnnotationKind::Magnifier {
            rect: ImageRect::new(4.0, 4.0, 4.0, 4.0),
            zoom: 2.0,
            source: None,
        },
        AnnotationStyle::default(),
    );
    let json = document.to_project("source.png").to_json_pretty().unwrap();
    let project = sniplet_core::Project::from_json(&json).unwrap();
    assert_eq!(project.annotations, document.annotations());

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy["annotations"][0]["kind"]
        .as_object_mut()
        .unwrap()
        .remove("size");
    let legacy: sniplet_core::Project = serde_json::from_value(legacy).unwrap();
    assert!(matches!(
        legacy.annotations[0].kind,
        AnnotationKind::Image { size: None, .. }
    ));
}
