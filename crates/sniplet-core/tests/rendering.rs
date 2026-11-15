use image::{Rgba, RgbaImage};
use sniplet_core::{
    Annotation, AnnotationId, AnnotationKind, AnnotationStyle, ArrowVariant, Backdrop, Background,
    Color, Document, ExportFormat, ImageRect, Point, RenderOptions, Shadow, decode_image,
};

fn white_document() -> Document {
    Document::new(RgbaImage::from_pixel(32, 24, Rgba([255, 255, 255, 255])))
}

#[test]
fn rendering_is_non_destructive_and_composites_vector_and_raster_tools() {
    let mut document = Document::new(RgbaImage::from_fn(32, 24, |x, y| {
        Rgba([(x * 7) as u8, (y * 9) as u8, (x + y) as u8, 255])
    }));
    let pristine = document.original().clone();
    let style = AnnotationStyle {
        stroke: Color::RED,
        fill: Some(Color::new(10, 20, 30, 255)),
        stroke_width: 2.0,
    };
    document.add_annotation(
        AnnotationKind::Line {
            start: Point::new(1.0, 1.0),
            end: Point::new(15.0, 1.0),
        },
        style,
    );
    document.add_annotation(
        AnnotationKind::Pixelate {
            rect: ImageRect::new(0.0, 8.0, 8.0, 8.0),
            block_size: 4,
        },
        style,
    );
    document.add_annotation(
        AnnotationKind::Blur {
            rect: ImageRect::new(10.0, 8.0, 8.0, 8.0),
            radius: 2.0,
        },
        style,
    );
    document.add_annotation(
        AnnotationKind::RemoveFill {
            rect: ImageRect::new(20.0, 8.0, 4.0, 4.0),
            sample: Some(Point::new(0.0, 0.0)),
        },
        style,
    );

    let rendered = document.render(&RenderOptions::default()).unwrap();
    assert_eq!(document.original(), &pristine);
    assert_eq!(rendered.get_pixel(7, 1).0, [255, 55, 66, 255]);
    assert_eq!(rendered.get_pixel(0, 8), rendered.get_pixel(3, 11));
    assert_eq!(rendered.get_pixel(21, 9), pristine.get_pixel(0, 0));
}

#[test]
fn every_vector_shape_composites_into_the_raster() {
    let mut document = Document::new(RgbaImage::from_pixel(100, 70, Rgba([255, 255, 255, 255])));
    let outline = AnnotationStyle::default();
    let filled = AnnotationStyle {
        fill: Some(Color::new(20, 80, 160, 255)),
        ..outline
    };
    document.add_annotation(
        AnnotationKind::Arrow {
            start: Point::new(4.0, 8.0),
            end: Point::new(20.0, 8.0),
            bend: None,
            variant: ArrowVariant::Solid,
        },
        outline,
    );
    document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(28.0, 3.0, 16.0, 16.0),
        },
        filled,
    );
    document.add_annotation(
        AnnotationKind::Ellipse {
            rect: ImageRect::new(52.0, 3.0, 18.0, 18.0),
        },
        filled,
    );
    document.add_annotation(
        AnnotationKind::Highlight {
            rect: ImageRect::new(4.0, 30.0, 16.0, 12.0),
        },
        outline,
    );
    document.add_annotation(
        AnnotationKind::Freehand {
            points: vec![
                Point::new(28.0, 35.0),
                Point::new(36.0, 31.0),
                Point::new(44.0, 38.0),
            ],
        },
        outline,
    );
    document.add_annotation(
        AnnotationKind::Redaction {
            rect: ImageRect::new(52.0, 30.0, 18.0, 12.0),
        },
        outline,
    );

    let rendered = document.render(&RenderOptions::default()).unwrap();
    for point in [(10, 8), (35, 10), (61, 12), (10, 35), (36, 31), (60, 35)] {
        assert_ne!(rendered.get_pixel(point.0, point.1).0, [255, 255, 255, 255]);
    }
}

#[test]
fn a_stroke_wider_than_its_rectangle_fills_it_without_stepping_outside() {
    let mut document = Document::new(RgbaImage::from_pixel(20, 20, Rgba([255, 255, 255, 255])));
    let red = Color::new(255, 0, 0, 255);
    // Covers pixels 0..5. A fractional corner used to walk the stroke diagonally out to (9, 9).
    document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(0.5, 0.5, 4.0, 4.0),
        },
        AnnotationStyle {
            stroke: red,
            fill: None,
            stroke_width: 10.0,
        },
    );
    // Its composite bounds reach (6, 6), so it also shows pixels a shape left in the shared layer.
    document.add_annotation(
        AnnotationKind::Line {
            start: Point::new(18.0, 18.0),
            end: Point::new(18.0, 18.0),
        },
        AnnotationStyle {
            stroke: Color::new(0, 0, 255, 255),
            fill: None,
            stroke_width: 20.0,
        },
    );

    let rendered = document.render(&RenderOptions::default()).unwrap();
    for (x, y, pixel) in rendered.enumerate_pixels() {
        let inside = x < 5 && y < 5;
        assert_eq!(pixel.0 == [255, 0, 0, 255], inside, "pixel ({x}, {y})");
    }
}

#[test]
fn nan_and_oversized_ellipses_do_not_draw_stray_pixels_or_panic() {
    let mut document = white_document();
    let filled = AnnotationStyle {
        fill: Some(Color::new(20, 80, 160, 255)),
        ..AnnotationStyle::default()
    };
    // A NaN coordinate used to put the fill at the canvas edge.
    document.add_annotation(
        AnnotationKind::Ellipse {
            rect: ImageRect::new(f32::NAN, 5.0, 6.0, 4.0),
        },
        filled,
    );
    assert_eq!(
        document.render(&RenderOptions::default()).unwrap(),
        *document.original()
    );

    // Live previews are not clipped to the canvas, and imageproc squares radii as i32.
    let preview = [Annotation {
        id: AnnotationId(99),
        kind: AnnotationKind::Ellipse {
            rect: ImageRect::new(0.0, 0.0, 100_000.0, 50.0),
        },
        style: filled,
    }];
    document
        .render(&RenderOptions {
            extra_annotations: &preview,
            ..RenderOptions::default()
        })
        .unwrap();
}

#[test]
fn live_preview_crop_backdrop_rounding_shadow_and_gradient_are_applied() {
    let mut document = white_document();
    document.set_crop(ImageRect::new(4.0, 2.0, 20.0, 16.0));
    document.set_backdrop(Backdrop {
        padding: 6,
        corner_radius: 4,
        background: Background::LinearGradient {
            start: Color::new(0, 10, 20, 255),
            end: Color::new(80, 90, 100, 255),
            angle_degrees: 0.0,
        },
        shadow: Some(Shadow {
            offset: Point::new(1.0, 2.0),
            blur_radius: 2.0,
            color: Color::new(0, 0, 0, 120),
        }),
    });
    let preview = Annotation {
        id: AnnotationId(999),
        kind: AnnotationKind::Redaction {
            rect: ImageRect::new(8.0, 6.0, 4.0, 4.0),
        },
        style: AnnotationStyle::default(),
    };
    let options = RenderOptions {
        extra_annotations: &[preview],
        ..RenderOptions::default()
    };
    let rendered = document.render(&options).unwrap();

    assert_eq!(rendered.dimensions(), (32, 28));
    assert_eq!(
        document.render_content_origin(&options),
        Point::new(2.0, 4.0)
    );
    assert_ne!(rendered.get_pixel(0, 0), rendered.get_pixel(31, 0));
    assert_eq!(rendered.get_pixel(6, 6).0[3], 255); // rounded corner exposes backdrop
    assert_eq!(rendered.get_pixel(10, 10).0, [0, 0, 0, 255]);

    let source = document.render_source(&options).unwrap();
    assert_eq!(source.dimensions(), (32, 24));
}

#[test]
fn fractional_crop_origin_matches_the_pixels_used_by_rendering() {
    let source = RgbaImage::from_fn(20, 20, |x, y| Rgba([x as u8, y as u8, 0, 255]));
    let mut document = Document::new(source);
    document.set_crop(ImageRect::new(4.7, 3.2, 8.0, 7.0));
    document.set_backdrop(Backdrop {
        padding: 2,
        ..Backdrop::default()
    });
    let options = RenderOptions::default();
    let rendered = document.render(&options).unwrap();
    assert_eq!(
        document.render_content_origin(&options),
        Point::new(-2.0, -1.0)
    );
    assert_eq!(rendered.get_pixel(2, 2).0, [4, 3, 0, 255]);
}

#[test]
fn png_and_jpeg_encoders_produce_decodable_images() {
    let document = white_document();
    let png = document
        .encode(ExportFormat::Png, &RenderOptions::default())
        .unwrap();
    let jpeg = document
        .encode(
            ExportFormat::Jpeg { quality: 88 },
            &RenderOptions::default(),
        )
        .unwrap();
    assert_eq!(decode_image(&png).unwrap().dimensions(), (32, 24));
    assert_eq!(decode_image(&jpeg).unwrap().dimensions(), (32, 24));
    assert!(
        document
            .encode(ExportFormat::Jpeg { quality: 0 }, &RenderOptions::default())
            .is_err()
    );
}

#[test]
fn text_requires_a_caller_supplied_font() {
    let mut document = white_document();
    document.add_annotation(
        AnnotationKind::Text {
            origin: Point::new(2.0, 2.0),
            text: "Sniplet".into(),
            font_size: 14.0,
        },
        AnnotationStyle::default(),
    );
    assert!(matches!(
        document.render(&RenderOptions::default()),
        Err(sniplet_core::SnipletError::MissingFont)
    ));
}

#[test]
fn caller_supplied_system_font_renders_text_and_counters_when_available() {
    let candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/SFNS.ttf",
    ];
    let Some(font) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else {
        // Font discovery belongs to the platform crate; minimal containers may have none.
        return;
    };
    let mut document = Document::new(RgbaImage::from_pixel(160, 60, Rgba([255, 255, 255, 255])));
    document.add_annotation(
        AnnotationKind::Text {
            origin: Point::new(5.0, 5.0),
            text: "Sniplet".into(),
            font_size: 24.0,
        },
        AnnotationStyle::default(),
    );
    document.add_annotation(
        AnnotationKind::Counter {
            center: Point::new(130.0, 28.0),
            value: 3,
            font_size: 20.0,
        },
        AnnotationStyle::default(),
    );
    let rendered = document
        .render(&RenderOptions {
            font_bytes: Some(&font),
            ..RenderOptions::default()
        })
        .unwrap();
    let changed = rendered
        .pixels()
        .filter(|pixel| pixel.0 != [255, 255, 255, 255])
        .count();
    assert!(changed > 100);
}

#[test]
fn a_magnifier_never_shows_what_a_raster_edit_hides() {
    let secret = Rgba([12, 200, 90, 255]);
    for source in [None, Some(Point::new(12.0, 12.0))] {
        let mut document = Document::new(RgbaImage::from_pixel(60, 40, secret));
        document.add_annotation(
            AnnotationKind::Redaction {
                rect: ImageRect::new(0.0, 0.0, 60.0, 40.0),
            },
            AnnotationStyle::default(),
        );
        document.add_annotation(
            AnnotationKind::Magnifier {
                rect: ImageRect::new(30.0, 8.0, 24.0, 24.0),
                zoom: 2.0,
                source,
            },
            AnnotationStyle::default(),
        );
        let rendered = document.render(&RenderOptions::default()).unwrap();
        assert!(
            rendered.pixels().all(|pixel| *pixel != secret),
            "the lens ({source:?}) showed redacted pixels"
        );
    }
}

#[test]
fn renders_that_reuse_cached_layers_match_a_fresh_render() {
    let build = |extra: bool| {
        let mut document = Document::new(RgbaImage::from_fn(48, 32, |x, y| {
            Rgba([(x * 5) as u8, (y * 7) as u8, (x ^ y) as u8, 255])
        }));
        document.add_annotation(
            AnnotationKind::Blur {
                rect: ImageRect::new(4.0, 4.0, 20.0, 12.0),
                radius: 3.0,
            },
            AnnotationStyle::default(),
        );
        let mut png = std::io::Cursor::new(Vec::new());
        RgbaImage::from_pixel(3, 2, Rgba([200, 30, 60, 160]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        document.add_annotation(
            AnnotationKind::Image {
                origin: Point::new(20.0, 10.0),
                png_bytes: png.into_inner().into(),
                size: Some(sniplet_core::ImageSize {
                    width: 9,
                    height: 6,
                }),
            },
            AnnotationStyle::default(),
        );
        document.set_backdrop(Backdrop {
            padding: 5,
            corner_radius: 3,
            background: Background::Solid {
                color: Color::new(240, 240, 250, 255),
            },
            shadow: Some(Shadow {
                offset: Point::new(1.0, 2.0),
                blur_radius: 2.5,
                color: Color::new(0, 0, 0, 90),
            }),
        });
        if extra {
            document.add_annotation(
                AnnotationKind::Rectangle {
                    rect: ImageRect::new(2.0, 2.0, 30.0, 20.0),
                },
                AnnotationStyle::default(),
            );
        }
        document
    };
    let mut document = build(false);
    let options = RenderOptions::default();
    let first = document.render(&options).unwrap();
    assert_eq!(document.render(&options).unwrap(), first);
    // A drawing reuses the cached base and backdrop.
    document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(2.0, 2.0, 30.0, 20.0),
        },
        AnnotationStyle::default(),
    );
    assert_eq!(
        document.render(&options).unwrap(),
        build(true).render(&options).unwrap()
    );
    assert!(document.undo());
    assert_eq!(document.snapshot().render(&options).unwrap(), first);
}

#[test]
fn text_bounds_hold_the_drawn_glyphs() {
    let mut document = white_document();
    let id = document.add_annotation(
        AnnotationKind::Text {
            origin: Point::new(3.0, 2.0),
            text: "Wg".into(),
            font_size: 16.0,
        },
        AnnotationStyle::default(),
    );
    let bounds = document.annotation(id).unwrap().kind.bounds();
    let rendered = document
        .render(&RenderOptions {
            font_bytes: Some(sniplet_core::FONT),
            ..RenderOptions::default()
        })
        .unwrap();
    let inked: Vec<_> = rendered
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel.0 != [255, 255, 255, 255])
        .map(|(x, y, _)| (x as f32, y as f32))
        .collect();
    assert!(!inked.is_empty());
    for (x, y) in &inked {
        assert!(
            *x >= bounds.x - 1.0
                && *x < bounds.x + bounds.width + 1.0
                && *y >= bounds.y - 1.0
                && *y < bounds.y + bounds.height + 1.0,
            "ink at ({x}, {y}) is outside {bounds:?}"
        );
    }
    let right = inked.iter().map(|(x, _)| *x).fold(0.0, f32::max);
    assert!(
        right > bounds.x + bounds.width - 4.0,
        "{bounds:?} is wider than the text"
    );
}
