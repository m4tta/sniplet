use image::{Rgba, RgbaImage};
use sniplet_core::{
    AnnotationKind, AnnotationStyle, ArrowVariant, Backdrop, Color, Document, ImageRect, Point,
    Project,
};

fn document() -> Document {
    Document::new(RgbaImage::from_fn(100, 80, |x, y| {
        Rgba([x as u8, y as u8, 25, 255])
    }))
}

#[test]
fn rectangles_are_normalized_clipped_and_selectable() {
    let mut document = document();
    let id = document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::from_corners(Point::new(120.0, 70.0), Point::new(-10.0, 20.0)),
        },
        AnnotationStyle::default(),
    );

    assert_eq!(
        document.annotation(id).unwrap().kind.bounds(),
        ImageRect::new(0.0, 20.0, 100.0, 50.0)
    );
    assert_eq!(document.hit_test(Point::new(50.0, 40.0), 0.0), Some(id));
    assert_eq!(document.select_at(Point::new(50.0, 40.0), 0.0), Some(id));
    assert_eq!(document.selected(), Some(id));

    document
        .move_annotation(id, Point::new(5.0, -10.0))
        .unwrap();
    assert_eq!(
        document.annotation(id).unwrap().kind.bounds(),
        ImageRect::new(5.0, 10.0, 100.0, 50.0)
    );
    document.delete_annotation(id).unwrap();
    assert_eq!(document.selected(), None);
}

#[test]
fn resize_maps_point_geometry_and_is_one_history_step() {
    let mut document = document();
    let original = AnnotationKind::Arrow {
        start: Point::new(10.0, 20.0),
        end: Point::new(30.0, 40.0),
        bend: None,
        variant: ArrowVariant::Solid,
    };
    let id = document.add_annotation(original.clone(), AnnotationStyle::default());
    document
        .resize_annotation(id, ImageRect::new(5.0, 6.0, 50.0, 20.0))
        .unwrap();
    assert_eq!(
        document.annotation(id).unwrap().kind,
        AnnotationKind::Arrow {
            start: Point::new(5.0, 6.0),
            end: Point::new(55.0, 26.0),
            bend: None,
            variant: ArrowVariant::Solid,
        }
    );
    assert!(document.undo());
    assert_eq!(document.annotation(id).unwrap().kind, original);
    assert!(document.redo());
    assert_eq!(
        document.annotation(id).unwrap().kind.bounds(),
        ImageRect::new(5.0, 6.0, 50.0, 20.0)
    );
}

#[test]
fn invalid_resize_does_not_change_geometry_or_history() {
    let mut document = document();
    let id = document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(10.0, 10.0, 20.0, 20.0),
        },
        AnnotationStyle::default(),
    );
    assert!(matches!(
        document.resize_annotation(id, ImageRect::new(0.0, 0.0, 0.0, 20.0)),
        Err(sniplet_core::SnipletError::InvalidAnnotationBounds)
    ));
    assert!(matches!(
        document.resize_annotation(id, ImageRect::new(f32::NAN, 0.0, 20.0, 20.0)),
        Err(sniplet_core::SnipletError::InvalidAnnotationBounds)
    ));
    assert_eq!(
        document.annotation(id).unwrap().kind.bounds(),
        ImageRect::new(10.0, 10.0, 20.0, 20.0)
    );
    assert!(document.undo());
    assert!(document.annotations().is_empty());
}

#[test]
fn rectangular_text_counter_and_freehand_geometry_resize_predictably() {
    let mut document = document();
    let target = ImageRect::new(-5.0, 7.0, 30.0, 18.0);
    let rectangular_kinds = [
        AnnotationKind::Rectangle {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
        },
        AnnotationKind::Ellipse {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
        },
        AnnotationKind::Pixelate {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
            block_size: 4,
        },
        AnnotationKind::Blur {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
            radius: 2.0,
        },
        AnnotationKind::Highlight {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
        },
        AnnotationKind::Spotlight {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
        },
        AnnotationKind::Redaction {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
        },
        AnnotationKind::RemoveFill {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
            sample: None,
        },
        AnnotationKind::Magnifier {
            rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
            zoom: 2.0,
            source: None,
        },
    ];
    for kind in rectangular_kinds {
        let id = document.add_annotation(kind, AnnotationStyle::default());
        document.resize_annotation(id, target).unwrap();
        assert_eq!(document.annotation(id).unwrap().kind.bounds(), target);
    }

    let text = document.add_annotation(
        AnnotationKind::Text {
            origin: Point::new(1.0, 1.0),
            text: "AB".into(),
            font_size: 10.0,
        },
        AnnotationStyle::default(),
    );
    document
        .resize_annotation(text, ImageRect::new(2.0, 3.0, 60.0, 25.0))
        .unwrap();
    let text_bounds = document.annotation(text).unwrap().kind.bounds();
    assert_eq!(
        Point::new(text_bounds.x, text_bounds.y),
        Point::new(2.0, 3.0)
    );
    assert!(text_bounds.width <= 60.0 && text_bounds.height <= 25.0);

    let counter = document.add_annotation(
        AnnotationKind::Counter {
            center: Point::new(10.0, 10.0),
            value: 1,
            font_size: 10.0,
        },
        AnnotationStyle::default(),
    );
    document
        .resize_annotation(counter, ImageRect::new(0.0, 0.0, 26.0, 13.0))
        .unwrap();
    assert_eq!(
        document.annotation(counter).unwrap().kind.bounds(),
        ImageRect::new(6.5, 0.0, 13.0, 13.0)
    );

    let freehand = document.add_annotation(
        AnnotationKind::Freehand {
            points: vec![Point::new(1.0, 1.0), Point::new(3.0, 5.0)],
        },
        AnnotationStyle::default(),
    );
    document
        .resize_annotation(freehand, ImageRect::new(10.0, 20.0, 8.0, 4.0))
        .unwrap();
    assert_eq!(
        document.annotation(freehand).unwrap().kind.bounds(),
        ImageRect::new(10.0, 20.0, 8.0, 4.0)
    );
}

#[test]
fn edit_groups_undo_and_redo_as_one_change() {
    let mut document = document();
    document.begin_group().unwrap();
    let first = document.add_annotation(
        AnnotationKind::Line {
            start: Point::new(1.0, 1.0),
            end: Point::new(10.0, 10.0),
        },
        AnnotationStyle::default(),
    );
    let second = document.add_annotation(
        AnnotationKind::Highlight {
            rect: ImageRect::new(20.0, 20.0, 10.0, 10.0),
        },
        AnnotationStyle::default(),
    );
    document.set_crop(ImageRect::new(5.0, 5.0, 50.0, 40.0));
    document.end_group().unwrap();

    assert_eq!(document.annotations().len(), 2);
    assert!(document.undo());
    assert!(document.annotations().is_empty());
    assert_eq!(document.crop(), None);
    assert!(document.redo());
    assert!(document.annotation(first).is_some());
    assert!(document.annotation(second).is_some());
    assert_eq!(document.crop(), Some(ImageRect::new(5.0, 5.0, 50.0, 40.0)));
}

#[test]
fn cancelling_unchanged_group_does_not_consume_prior_history() {
    let mut document = document();
    let id = document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(4.0, 5.0, 10.0, 12.0),
        },
        AnnotationStyle::default(),
    );
    document.select(Some(id)).unwrap();
    document.begin_group().unwrap();
    document.cancel_group().unwrap();
    assert_eq!(document.selected(), Some(id));
    assert!(document.undo());
    assert!(document.annotations().is_empty());
}

#[test]
fn cancelling_modified_group_restores_geometry_selection_and_id_sequence() {
    let mut document = document();
    let original = document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(4.0, 5.0, 10.0, 12.0),
        },
        AnnotationStyle::default(),
    );
    document.select(Some(original)).unwrap();
    document.begin_group().unwrap();
    document
        .move_annotation(original, Point::new(20.0, 30.0))
        .unwrap();
    let temporary = document.add_annotation(
        AnnotationKind::Redaction {
            rect: ImageRect::new(1.0, 1.0, 3.0, 3.0),
        },
        AnnotationStyle::default(),
    );
    document.select(Some(temporary)).unwrap();
    document.set_crop(ImageRect::new(2.0, 2.0, 20.0, 20.0));
    document.cancel_group().unwrap();

    assert_eq!(document.annotations().len(), 1);
    assert_eq!(document.selected(), Some(original));
    assert_eq!(document.crop(), None);
    assert_eq!(
        document.annotation(original).unwrap().kind.bounds(),
        ImageRect::new(4.0, 5.0, 10.0, 12.0)
    );
    let reused = document.add_annotation(
        AnnotationKind::Line {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 1.0),
        },
        AnnotationStyle::default(),
    );
    assert_eq!(reused, temporary);
}

#[test]
fn project_round_trip_keeps_editable_state_but_not_history() {
    let mut document = document();
    let id = document.add_annotation(
        AnnotationKind::Redaction {
            rect: ImageRect::new(3.0, 4.0, 5.0, 6.0),
        },
        AnnotationStyle {
            fill: Some(Color::BLACK),
            ..AnnotationStyle::default()
        },
    );
    document.set_crop(ImageRect::new(2.0, 3.0, 40.0, 30.0));
    document.set_backdrop(Backdrop {
        padding: 12,
        ..Backdrop::default()
    });

    let json = document.to_project("capture.png").to_json_pretty().unwrap();
    let project = Project::from_json(&json).unwrap();
    let restored = Document::from_project(project, document.original().clone()).unwrap();
    assert_eq!(restored.annotation(id), document.annotation(id));
    assert_eq!(restored.crop(), document.crop());
    assert_eq!(restored.backdrop(), document.backdrop());
    assert!(!restored.can_undo());

    let mut unsupported = document.to_project("capture.png");
    unsupported.version += 1;
    assert!(matches!(
        Document::from_project(unsupported, document.original().clone()),
        Err(sniplet_core::SnipletError::UnsupportedProjectVersion { .. })
    ));
}

#[test]
fn project_paths_accept_current_and_legacy_extensions() {
    assert!(Project::is_project_path("capture.sniplet"));
    assert!(Project::is_project_path("capture.SNIPLET"));
    assert!(Project::is_project_path("capture.clippy"));
    assert!(Project::is_project_path("capture.CLIPPY"));
    assert!(!Project::is_project_path("capture.png"));
    assert!(!Project::is_project_path("sniplet"));
}

#[test]
fn current_and_legacy_projects_resolve_relative_source_after_directory_move() {
    let unique = format!(
        "sniplet-core-project-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    let original_dir = root.join("original");
    let moved_dir = root.join("moved");
    std::fs::create_dir_all(&original_dir).unwrap();
    let document = document();
    document
        .original()
        .save(original_dir.join("source.png"))
        .unwrap();
    for name in ["capture.sniplet", "capture.clippy"] {
        document
            .to_project("source.png")
            .save(original_dir.join(name))
            .unwrap();
    }
    std::fs::rename(&original_dir, &moved_dir).unwrap();

    for name in ["capture.sniplet", "capture.clippy"] {
        let raw_json = std::fs::read_to_string(moved_dir.join(name)).unwrap();
        assert_eq!(
            Project::from_json(&raw_json).unwrap().source_image,
            std::path::PathBuf::from("source.png")
        );
        let loaded = Project::load(moved_dir.join(name)).unwrap();
        assert_eq!(loaded.source_image, moved_dir.join("source.png"));
        assert_eq!(
            loaded.open_document().unwrap().dimensions(),
            document.dimensions()
        );
    }

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_sampling_and_color_formats_are_stable() {
    let document = document();
    let color = document.sample_color(Point::new(7.9, 9.2)).unwrap();
    assert_eq!(color, Color::new(7, 9, 25, 255));
    assert_eq!(color.format(sniplet_core::ColorFormat::HexRgb), "#070919");
    assert_eq!(
        Color::from_hex("#abc"),
        Some(Color::new(170, 187, 204, 255))
    );
    assert_eq!(document.sample_color(Point::new(-1.0, 0.0)), None);
}
