use image::RgbaImage;
use sniplet_core::{
    AnnotationKind, AnnotationStyle, ArrowVariant, Color, Document, ImageRect, Point, Project,
    RenderOptions,
};

fn curved_arrow() -> AnnotationKind {
    AnnotationKind::Arrow {
        start: Point::new(10.0, 50.0),
        end: Point::new(90.0, 50.0),
        bend: Some(Point::new(50.0, 10.0)),
        variant: ArrowVariant::Solid,
    }
}

#[test]
fn arrow_handles_preserve_bend_displacement_when_endpoints_move() {
    let mut arrow = AnnotationKind::Arrow {
        start: Point::new(0.0, 0.0),
        end: Point::new(100.0, 0.0),
        bend: Some(Point::new(50.0, 30.0)),
        variant: ArrowVariant::Solid,
    };

    assert_eq!(
        arrow.arrow_points(),
        Some([
            Point::new(0.0, 0.0),
            Point::new(50.0, 30.0),
            Point::new(100.0, 0.0),
        ])
    );
    assert!(arrow.set_arrow_point(0, Point::new(20.0, 10.0)));
    assert_eq!(
        arrow.arrow_points(),
        Some([
            Point::new(20.0, 10.0),
            Point::new(60.0, 35.0),
            Point::new(100.0, 0.0),
        ])
    );
    assert!(arrow.set_arrow_point(2, Point::new(80.0, 20.0)));
    assert_eq!(
        arrow.arrow_points(),
        Some([
            Point::new(20.0, 10.0),
            Point::new(50.0, 45.0),
            Point::new(80.0, 20.0),
        ])
    );

    assert!(arrow.set_arrow_point(1, Point::new(50.0, 15.0)));
    assert!(matches!(arrow, AnnotationKind::Arrow { bend: None, .. }));
    assert!(!arrow.set_arrow_point(3, Point::new(1.0, 1.0)));
    assert!(
        !AnnotationKind::Rectangle {
            rect: ImageRect::new(0.0, 0.0, 1.0, 1.0)
        }
        .set_arrow_point(0, Point::new(1.0, 1.0))
    );
}

#[test]
fn curve_bounds_and_hit_testing_follow_the_visible_quadratic() {
    let arrow = curved_arrow();
    assert_eq!(arrow.bounds(), ImageRect::new(10.0, 10.0, 80.0, 40.0));

    let mut document = Document::new(RgbaImage::new(100, 70));
    let id = document.add_annotation(
        arrow,
        AnnotationStyle {
            stroke_width: 8.0,
            ..AnnotationStyle::default()
        },
    );
    assert_eq!(document.hit_test(Point::new(50.0, 10.0), 0.0), Some(id));
    assert_eq!(document.hit_test(Point::new(50.0, 50.0), 0.0), None);
    assert_eq!(document.hit_test(Point::new(76.0, 29.0), 0.0), Some(id));
}

#[test]
fn zero_length_arrow_only_hits_near_its_endpoint() {
    let mut document = Document::new(RgbaImage::new(100, 70));
    let id = document.add_annotation(
        AnnotationKind::Arrow {
            start: Point::new(50.0, 35.0),
            end: Point::new(50.0, 35.0),
            bend: None,
            variant: ArrowVariant::Solid,
        },
        AnnotationStyle {
            stroke_width: 10.0,
            ..AnnotationStyle::default()
        },
    );

    assert_eq!(document.hit_test(Point::new(52.0, 35.0), 0.0), Some(id));
    assert_eq!(document.hit_test(Point::new(2.0, 2.0), 0.0), None);
}

#[test]
fn move_resize_and_handle_edits_are_undoable_with_all_three_points() {
    let mut document = Document::new(RgbaImage::new(200, 120));
    let style = AnnotationStyle::default();
    let id = document.add_annotation(curved_arrow(), style);

    document.move_annotation(id, Point::new(5.0, 7.0)).unwrap();
    assert_eq!(
        document.annotation(id).unwrap().kind.arrow_points(),
        Some([
            Point::new(15.0, 57.0),
            Point::new(55.0, 17.0),
            Point::new(95.0, 57.0),
        ])
    );
    assert!(document.undo());
    assert_eq!(document.annotation(id).unwrap().kind, curved_arrow());

    document
        .resize_annotation(id, ImageRect::new(0.0, 0.0, 160.0, 80.0))
        .unwrap();
    assert_eq!(
        document.annotation(id).unwrap().kind.arrow_points(),
        Some([
            Point::new(0.0, 80.0),
            Point::new(80.0, 0.0),
            Point::new(160.0, 80.0),
        ])
    );
    assert_eq!(
        document.annotation(id).unwrap().kind.bounds(),
        ImageRect::new(0.0, 0.0, 160.0, 80.0)
    );
    assert!(document.undo());

    let mut edited = document.annotation(id).unwrap().kind.clone();
    assert!(edited.set_arrow_point(1, Point::new(50.0, 20.0)));
    document.update_annotation(id, edited, style).unwrap();
    assert_eq!(
        document
            .annotation(id)
            .unwrap()
            .kind
            .arrow_points()
            .unwrap()[1],
        Point::new(50.0, 20.0)
    );
    assert!(document.undo());
    assert_eq!(document.annotation(id).unwrap().kind, curved_arrow());
}

#[test]
fn projects_round_trip_bends_and_accept_legacy_straight_arrows() {
    let mut document = Document::new(RgbaImage::new(100, 70));
    document.add_annotation(curved_arrow(), AnnotationStyle::default());
    document.add_annotation(
        AnnotationKind::Arrow {
            start: Point::new(5.0, 60.0),
            end: Point::new(95.0, 60.0),
            bend: None,
            variant: ArrowVariant::Solid,
        },
        AnnotationStyle::default(),
    );

    let json = document.to_project("source.png").to_json_pretty().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value["annotations"][0]["kind"]["bend"],
        serde_json::json!({ "x": 50.0, "y": 10.0 })
    );
    assert!(value["annotations"][1]["kind"].get("bend").is_none());
    assert_eq!(
        Project::from_json(&json).unwrap().annotations,
        document.annotations()
    );

    let legacy: AnnotationKind = serde_json::from_str(
        r#"{"type":"arrow","start":{"x":1.0,"y":2.0},"end":{"x":9.0,"y":6.0}}"#,
    )
    .unwrap();
    assert_eq!(
        legacy.arrow_points(),
        Some([
            Point::new(1.0, 2.0),
            Point::new(5.0, 4.0),
            Point::new(9.0, 6.0),
        ])
    );
    assert!(matches!(
        legacy,
        AnnotationKind::Arrow {
            variant: ArrowVariant::Solid,
            ..
        }
    ));
}

#[test]
fn renderer_produces_a_connected_antialiased_curve_and_bold_head() {
    let mut document = Document::new(RgbaImage::new(180, 120));
    document.add_annotation(
        AnnotationKind::Arrow {
            start: Point::new(20.0, 90.0),
            end: Point::new(160.0, 90.0),
            bend: Some(Point::new(90.0, 20.0)),
            variant: ArrowVariant::Solid,
        },
        AnnotationStyle {
            stroke: Color::new(230, 40, 35, 160),
            fill: None,
            stroke_width: 10.0,
        },
    );
    let rendered = document.render(&RenderOptions::default()).unwrap();

    let partial_pixels = rendered
        .pixels()
        .filter(|pixel| pixel[3] > 0 && pixel[3] < 160)
        .count();
    assert!(partial_pixels > 100, "curved edge should be antialiased");
    assert!(rendered.pixels().all(|pixel| pixel[3] <= 160));
    assert!(rendered.get_pixel(90, 20)[3] > 100);
    assert_eq!(rendered.get_pixel(90, 90)[3], 0);

    for index in 1..20 {
        let t = index as f32 / 20.0;
        let one_minus_t = 1.0 - t;
        let x = one_minus_t * one_minus_t * 20.0 + 2.0 * one_minus_t * t * 90.0 + t * t * 160.0;
        let y = one_minus_t * one_minus_t * 90.0 + 2.0 * one_minus_t * t * -50.0 + t * t * 90.0;
        let x = x.round().clamp(0.0, 179.0) as u32;
        let y = y.round().clamp(0.0, 119.0) as u32;
        assert!(
            rendered.get_pixel(x, y)[3] > 100,
            "shaft or head gap near ({x}, {y})"
        );
    }
    assert!(
        (157..=160)
            .flat_map(|x| (87..=93).map(move |y| (x, y)))
            .any(|(x, y)| rendered.get_pixel(x, y)[3] > 100),
        "arrow tip was missing"
    );
}

#[test]
fn straight_arrow_head_is_about_three_shaft_widths_wide_and_long() {
    let mut document = Document::new(RgbaImage::new(170, 100));
    document.add_annotation(
        AnnotationKind::Arrow {
            start: Point::new(20.0, 50.0),
            end: Point::new(150.0, 50.0),
            bend: None,
            variant: ArrowVariant::Solid,
        },
        AnnotationStyle {
            stroke_width: 10.0,
            ..AnnotationStyle::default()
        },
    );
    let rendered = document.render(&RenderOptions::default()).unwrap();

    assert!(rendered.get_pixel(121, 36)[3] > 0);
    assert!(rendered.get_pixel(121, 64)[3] > 0);
    assert_eq!(rendered.get_pixel(118, 34)[3], 0);
    assert!(rendered.get_pixel(20, 50)[3] > 200);
    assert_eq!(rendered.get_pixel(19, 50)[3], 0);
    assert!(
        (145..=149)
            .flat_map(|x| (48..=52).map(move |y| (x, y)))
            .any(|(x, y)| rendered.get_pixel(x, y)[3] > 200)
    );
    assert_eq!(rendered.get_pixel(151, 50)[3], 0);
}

#[test]
fn variants_use_snake_case_json_and_round_trip_in_projects() {
    let variants = [
        (ArrowVariant::Solid, "solid"),
        (ArrowVariant::HandDrawn, "hand_drawn"),
        (ArrowVariant::Thin, "thin"),
        (ArrowVariant::DoubleEnded, "double_ended"),
    ];
    let mut document = Document::new(RgbaImage::new(200, 120));
    for (index, (variant, name)) in variants.into_iter().enumerate() {
        document.add_annotation(
            AnnotationKind::Arrow {
                start: Point::new(10.0, index as f32 * 20.0 + 10.0),
                end: Point::new(190.0, index as f32 * 20.0 + 10.0),
                bend: None,
                variant,
            },
            AnnotationStyle::default(),
        );
        assert_eq!(serde_json::to_value(variant).unwrap(), name);
    }

    let json = document.to_project("source.png").to_json_pretty().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    for (index, (_, name)) in variants.into_iter().enumerate() {
        assert_eq!(value["annotations"][index]["kind"]["variant"], name);
    }
    assert_eq!(
        Project::from_json(&json).unwrap().annotations,
        document.annotations()
    );
}

#[test]
fn curved_open_heads_follow_both_endpoint_tangents_in_hit_testing() {
    let arrow = |variant| AnnotationKind::Arrow {
        start: Point::new(20.0, 100.0),
        end: Point::new(180.0, 100.0),
        bend: Some(Point::new(100.0, 20.0)),
        variant,
    };
    let style = AnnotationStyle {
        stroke_width: 10.0,
        ..AnnotationStyle::default()
    };

    let mut thin = Document::new(RgbaImage::new(200, 130));
    let thin_id = thin.add_annotation(arrow(ArrowVariant::Thin), style);
    assert_eq!(thin.hit_test(Point::new(180.0, 75.5), 0.0), Some(thin_id));
    assert_eq!(thin.hit_test(Point::new(20.0, 75.5), 0.0), None);

    let mut double = Document::new(RgbaImage::new(200, 130));
    let double_id = double.add_annotation(arrow(ArrowVariant::DoubleEnded), style);
    assert_eq!(
        double.hit_test(Point::new(180.0, 75.5), 0.0),
        Some(double_id)
    );
    assert_eq!(
        double.hit_test(Point::new(20.0, 75.5), 0.0),
        Some(double_id)
    );

    let mut hand_drawn = Document::new(RgbaImage::new(200, 130));
    let hand_id = hand_drawn.add_annotation(arrow(ArrowVariant::HandDrawn), style);
    assert_eq!(
        hand_drawn.hit_test(Point::new(181.0, 87.0), 0.0),
        Some(hand_id)
    );
}

#[test]
fn open_arrow_strokes_scale_with_the_user_size() {
    fn shaft_rows(size: f32) -> usize {
        let mut document = Document::new(RgbaImage::new(200, 100));
        document.add_annotation(
            AnnotationKind::Arrow {
                start: Point::new(20.0, 50.0),
                end: Point::new(180.0, 50.0),
                bend: None,
                variant: ArrowVariant::Thin,
            },
            AnnotationStyle {
                stroke_width: size,
                ..AnnotationStyle::default()
            },
        );
        let rendered = document.render(&RenderOptions::default()).unwrap();
        (0..rendered.height())
            .filter(|y| rendered.get_pixel(80, *y)[3] > 0)
            .count()
    }

    let small = shaft_rows(4.0);
    let large = shaft_rows(12.0);
    assert!(
        (1..=3).contains(&small),
        "small shaft occupied {small} rows"
    );
    assert!(large >= 4, "large shaft occupied only {large} rows");
    assert!(large > small);
}

#[test]
fn every_variant_handles_short_and_degenerate_arrows() {
    for variant in [
        ArrowVariant::Solid,
        ArrowVariant::HandDrawn,
        ArrowVariant::Thin,
        ArrowVariant::DoubleEnded,
    ] {
        let mut document = Document::new(RgbaImage::new(80, 60));
        let point = Point::new(30.0, 30.0);
        let zero_id = document.add_annotation(
            AnnotationKind::Arrow {
                start: point,
                end: point,
                bend: None,
                variant,
            },
            AnnotationStyle {
                stroke_width: 12.0,
                ..AnnotationStyle::default()
            },
        );
        document.add_annotation(
            AnnotationKind::Arrow {
                start: Point::new(45.0, 30.0),
                end: Point::new(48.0, 30.0),
                bend: Some(Point::new(46.5, 29.0)),
                variant,
            },
            AnnotationStyle {
                stroke_width: 12.0,
                ..AnnotationStyle::default()
            },
        );

        assert_eq!(document.hit_test(point, 0.0), Some(zero_id));
        let rendered = document.render(&RenderOptions::default()).unwrap();
        assert!(rendered.pixels().any(|pixel| pixel[3] > 0));
    }
}

#[test]
fn default_solid_rendering_matches_the_pre_variant_gallery_exactly() {
    let background = RgbaImage::from_fn(760, 620, |x, y| {
        if x % 40 == 0 || y % 40 == 0 {
            image::Rgba([224, 228, 234, 255])
        } else {
            image::Rgba([247, 248, 250, 255])
        }
    });
    let mut document = Document::new(background);
    let style = AnnotationStyle {
        stroke: Color::new(232, 48, 42, 255),
        fill: None,
        stroke_width: 10.0,
    };
    for (start, bend, end) in [
        (Point::new(70.0, 70.0), None, Point::new(330.0, 70.0)),
        (
            Point::new(430.0, 70.0),
            Some(Point::new(575.0, 135.0)),
            Point::new(700.0, 70.0),
        ),
        (
            Point::new(70.0, 240.0),
            Some(Point::new(200.0, 140.0)),
            Point::new(340.0, 240.0),
        ),
        (
            Point::new(430.0, 210.0),
            Some(Point::new(560.0, 315.0)),
            Point::new(700.0, 210.0),
        ),
        (
            Point::new(50.0, 420.0),
            Some(Point::new(200.0, 545.0)),
            Point::new(350.0, 560.0),
        ),
        (
            Point::new(440.0, 440.0),
            Some(Point::new(570.0, 390.0)),
            Point::new(700.0, 550.0),
        ),
    ] {
        document.add_annotation(
            AnnotationKind::Arrow {
                start,
                end,
                bend,
                variant: ArrowVariant::default(),
            },
            style,
        );
    }

    let rendered = document.render(&RenderOptions::default()).unwrap();
    let hash = rendered
        .as_raw()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    assert_eq!(hash, 0xef68_f49b_9771_dc88);
}
