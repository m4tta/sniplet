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
