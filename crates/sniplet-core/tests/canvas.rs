use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder};
use sniplet_core::{
    AnnotationKind, AnnotationStyle, Document, ImageRect, ImageSize, Point, Project, RenderOptions,
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

fn source() -> RgbaImage {
    RgbaImage::from_fn(2, 2, |x, y| Rgba([10 + x as u8, 20 + y as u8, 30, 255]))
}

#[test]
fn expanded_canvas_renders_source_and_pasted_capture_side_by_side() {
    let original = source();
    let pasted = RgbaImage::from_fn(3, 2, |x, y| Rgba([100 + x as u8, 110 + y as u8, 120, 255]));
    let mut document = Document::new(original.clone());

    assert!(document.expand_canvas_to(5, 3));
    document.add_annotation(
        AnnotationKind::Image {
            origin: Point::new(2.0, 0.0),
            png_bytes: png(&pasted),
            size: None,
        },
        AnnotationStyle::default(),
    );

    let rendered = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(rendered.dimensions(), (5, 3));
    for y in 0..2 {
        for x in 0..2 {
            assert_eq!(rendered.get_pixel(x, y), original.get_pixel(x, y));
        }
        for x in 0..3 {
            assert_eq!(rendered.get_pixel(x + 2, y), pasted.get_pixel(x, y));
        }
    }
    assert_eq!(rendered.get_pixel(4, 2).0, [0, 0, 0, 0]);
    assert_eq!(document.original(), &original);
    assert_eq!(document.sample_color(Point::new(3.0, 0.0)), None);
}

#[test]
fn canvas_expansion_and_overlay_share_one_undo_group_and_crop_to_expansion() {
    let pasted = RgbaImage::from_fn(2, 3, |x, y| Rgba([150 + x as u8, 160 + y as u8, 170, 255]));
    let mut document = Document::new(source());

    document.begin_group().unwrap();
    document.expand_canvas_to(4, 3);
    document.add_annotation(
        AnnotationKind::Image {
            origin: Point::new(2.0, 0.0),
            png_bytes: png(&pasted),
            size: None,
        },
        AnnotationStyle::default(),
    );
    document.end_group().unwrap();

    assert_eq!(
        document.dimensions(),
        ImageSize {
            width: 4,
            height: 3
        }
    );
    assert_eq!(document.annotations().len(), 1);
    assert!(document.undo());
    assert_eq!(
        document.dimensions(),
        ImageSize {
            width: 2,
            height: 2
        }
    );
    assert!(document.annotations().is_empty());
    assert!(document.redo());
    assert_eq!(
        document.dimensions(),
        ImageSize {
            width: 4,
            height: 3
        }
    );
    assert_eq!(document.annotations().len(), 1);

    document.set_crop(ImageRect::new(2.0, 1.0, 2.0, 2.0));
    let cropped = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(cropped.dimensions(), (2, 2));
    assert_eq!(cropped.get_pixel(0, 0), pasted.get_pixel(0, 1));
    assert_eq!(cropped.get_pixel(1, 1), pasted.get_pixel(1, 2));
    assert_eq!(
        document.render_content_origin(&RenderOptions::default()),
        Point::new(-2.0, -1.0)
    );
}

#[test]
fn project_round_trip_preserves_canvas_extent_and_legacy_defaults_to_source() {
    let original = source();
    let mut document = Document::new(original.clone());
    document.expand_canvas_to(7, 5);
    document.set_crop(ImageRect::new(1.0, 1.0, 6.0, 4.0));

    let json = document.to_project("source.png").to_json_pretty().unwrap();
    let project = Project::from_json(&json).unwrap();
    assert_eq!(
        project.source_size,
        ImageSize {
            width: 2,
            height: 2
        }
    );
    assert_eq!(
        project.canvas_size,
        Some(ImageSize {
            width: 7,
            height: 5
        })
    );
    let restored = Document::from_project(project, original.clone()).unwrap();
    assert_eq!(
        restored.dimensions(),
        ImageSize {
            width: 7,
            height: 5
        }
    );
    assert_eq!(restored.crop(), Some(ImageRect::new(1.0, 1.0, 6.0, 4.0)));
    assert!(!restored.can_undo());

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy.as_object_mut().unwrap().remove("canvas_size");
    let legacy = Project::from_json(&serde_json::to_string(&legacy).unwrap()).unwrap();
    let restored = Document::from_project(legacy, original).unwrap();
    assert_eq!(
        restored.dimensions(),
        ImageSize {
            width: 2,
            height: 2
        }
    );
    assert_eq!(restored.crop(), Some(ImageRect::new(1.0, 1.0, 1.0, 1.0)));
}
