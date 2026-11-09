//! A bad or hand-edited project file or image must fail with an error, never crash.

use image::{
    ExtendedColorType, ImageEncoder, Rgba, RgbaImage,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};
use serde_json::{Value, json};
use sniplet_core::{
    AnnotationId, AnnotationKind, AnnotationStyle, Backdrop, Color, Document, ImageRect, ImageSize,
    Point, Project, RenderOptions, Shadow, SnipletError, decode_image,
};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSans.ttf");

fn document() -> Document {
    Document::new(RgbaImage::from_pixel(8, 6, Rgba([40, 80, 120, 255])))
}

fn rectangle() -> AnnotationKind {
    AnnotationKind::Rectangle {
        rect: ImageRect::new(1.0, 1.0, 3.0, 3.0),
    }
}

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

/// A small JPEG whose header claims `width` x `height`.
fn jpeg_claiming(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    JpegEncoder::new(&mut bytes)
        .write_image(&[0; 48], 4, 4, ExtendedColorType::Rgb8)
        .unwrap();
    let frame = bytes.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    bytes[frame + 5..frame + 7].copy_from_slice(&height.to_be_bytes());
    bytes[frame + 7..frame + 9].copy_from_slice(&width.to_be_bytes());
    bytes
}

/// Saves `document`, applies `edit` to the JSON, and opens the result.
fn reopen(document: &Document, edit: impl FnOnce(&mut Value)) -> sniplet_core::Result<Document> {
    let mut json = serde_json::to_value(document.to_project("source.png")).unwrap();
    edit(&mut json);
    let project = Project::from_json(&json.to_string())?;
    Document::from_project(project, document.original().clone())
}

#[test]
fn non_finite_edits_are_refused_so_the_project_still_reloads() {
    let mut document = document();
    let id = document.add_annotation(rectangle(), AnnotationStyle::default());

    assert!(matches!(
        document.move_annotation(id, Point::new(f32::NAN, 0.0)),
        Err(SnipletError::NonFiniteValue)
    ));
    let line = AnnotationKind::Line {
        start: Point::new(f32::INFINITY, 0.0),
        end: Point::default(),
    };
    assert!(matches!(
        document.update_annotation(id, line, AnnotationStyle::default()),
        Err(SnipletError::NonFiniteValue)
    ));
    let text = AnnotationKind::Text {
        origin: Point::new(1.0, 1.0),
        text: "x".into(),
        font_size: f32::NAN,
    };
    let ignored = document.add_annotation(text, AnnotationStyle::default());
    assert!(document.annotation(ignored).is_none());
    document.set_backdrop(Backdrop {
        shadow: Some(Shadow {
            offset: Point::default(),
            blur_radius: f32::INFINITY,
            color: Color::BLACK,
        }),
        ..Backdrop::default()
    });
    assert_eq!(document.backdrop(), Backdrop::default());
    // A finite move saturates instead of overflowing to infinity.
    for _ in 0..2 {
        document
            .move_annotation(id, Point::new(f32::MAX, 0.0))
            .unwrap();
    }

    assert_eq!(reopen(&document, |_| {}).unwrap().annotations().len(), 1);
}

#[test]
fn infinite_numbers_in_a_file_are_rejected_on_open() {
    let mut document = document();
    document.add_annotation(
        AnnotationKind::Line {
            start: Point::new(1.0, 1.0),
            end: Point::new(5.0, 5.0),
        },
        AnnotationStyle::default(),
    );
    // serde reads 1e39 into an f32 as infinity.
    let opened = reopen(&document, |json| {
        json["annotations"][0]["kind"]["start"]["x"] = json!(1e39);
    });
    assert!(matches!(opened, Err(SnipletError::NonFiniteValue)));
}

#[test]
fn oversized_canvas_is_refused() {
    let mut document = document();
    let opened = reopen(&document, |json| {
        json["canvas_size"] = json!({"width": 100_000, "height": 100_000});
    });
    assert!(matches!(
        opened,
        Err(SnipletError::ImageTooLarge {
            width: 100_000,
            height: 100_000
        })
    ));
    assert!(!document.expand_canvas_to(100_000, 100_000));
    assert!(document.expand_canvas_to(16, 12));
}

#[test]
fn oversized_image_annotation_is_a_render_error() {
    let mut document = document();
    document.add_annotation(
        AnnotationKind::Image {
            origin: Point::default(),
            png_bytes: png(&RgbaImage::new(2, 2)),
            size: Some(ImageSize {
                width: 100_000,
                height: 100_000,
            }),
        },
        AnnotationStyle::default(),
    );
    assert!(matches!(
        document.render(&RenderOptions::default()),
        Err(SnipletError::ImageTooLarge { .. })
    ));
}

#[cfg(unix)]
#[test]
fn a_project_source_that_never_ends_fails_to_open() {
    // The source is streamed, so the format check reads a few bytes and stops.
    let project = document().to_project("/dev/zero");
    assert!(project.open_document().is_err());
}

#[test]
fn image_size_comes_from_the_header_before_any_pixels_are_decoded() {
    let bomb = jpeg_claiming(12_000, 12_000);
    assert!(matches!(
        decode_image(&bomb),
        Err(SnipletError::ImageTooLarge {
            width: 12_000,
            height: 12_000
        })
    ));
    let pasted = AnnotationKind::Image {
        origin: Point::new(1.0, 2.0),
        png_bytes: bomb,
        size: None,
    };
    assert_eq!(
        pasted.bounds(),
        ImageRect::new(1.0, 2.0, 12_000.0, 12_000.0)
    );
}

#[test]
fn repeated_annotation_ids_get_new_ids_on_open() {
    let mut document = document();
    document.add_annotation(rectangle(), AnnotationStyle::default());
    document.add_annotation(rectangle(), AnnotationStyle::default());

    let mut opened = reopen(&document, |json| json["annotations"][1]["id"] = json!(1)).unwrap();
    let ids: Vec<_> = opened.annotations().iter().map(|a| a.id).collect();
    assert_eq!(ids, [AnnotationId(1), AnnotationId(2)]);
    opened.delete_annotation(AnnotationId(1)).unwrap();
    assert_eq!(opened.annotations().len(), 1);
}

#[test]
fn huge_sizes_and_far_coordinates_render_in_bounded_time_and_memory() {
    let mut document = document();
    let thick = AnnotationStyle {
        stroke_width: 1e30,
        ..AnnotationStyle::default()
    };
    for (kind, style) in [
        (
            AnnotationKind::Text {
                origin: Point::new(1.0, 1.0),
                text: "Hi".into(),
                font_size: 1e6,
            },
            AnnotationStyle::default(),
        ),
        (
            AnnotationKind::Text {
                origin: Point::new(3.4e38, 0.0),
                text: "x".into(),
                font_size: 12.0,
            },
            AnnotationStyle::default(),
        ),
        (
            AnnotationKind::Counter {
                center: Point::new(2.0, 2.0),
                value: 7,
                font_size: 1e6,
            },
            AnnotationStyle::default(),
        ),
        (rectangle(), thick),
        (
            AnnotationKind::Blur {
                rect: ImageRect::new(0.0, 0.0, 8.0, 6.0),
                radius: 1e30,
            },
            AnnotationStyle::default(),
        ),
        (
            AnnotationKind::Pixelate {
                rect: ImageRect::new(1.0, 1.0, 6.0, 4.0),
                block_size: u32::MAX,
            },
            AnnotationStyle::default(),
        ),
    ] {
        document.add_annotation(kind, style);
    }
    let options = RenderOptions {
        font_bytes: Some(FONT),
        ..RenderOptions::default()
    };
    document.render(&options).unwrap();

    document.set_backdrop(Backdrop {
        padding: u32::MAX,
        ..Backdrop::default()
    });
    assert_eq!(
        document.render(&options).unwrap().dimensions(),
        (2056, 2054)
    );

    document.set_backdrop(Backdrop {
        padding: 4,
        shadow: Some(Shadow {
            offset: Point::new(3.4e38, -3.4e38),
            blur_radius: 1e30,
            color: Color::BLACK,
        }),
        ..Backdrop::default()
    });
    document.render(&options).unwrap();
}
