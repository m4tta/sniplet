//! Drawing, selecting, moving, cropping and undoing annotations with real pointer and keys.
use sniplet_core::{AnnotationKind, ImageRect};

use super::{arrow_points, editor};
use crate::tools::{HideStyle, Tool};

fn tool(tool: Tool) -> String {
    format!("tool-{}", tool as usize)
}

#[test]
fn draw_select_move_resize_and_delete_each_undo_in_one_step() {
    let mut h = editor();
    h.click(&tool(Tool::Rectangle));
    assert_eq!(h.editor().tool, Tool::Rectangle);
    h.drag_image((70.0, 30.0), (180.0, 120.0));
    let drawn = h.only();
    assert_eq!(drawn.kind.bounds(), ImageRect::new(70.0, 30.0, 110.0, 90.0));
    assert_eq!(h.document().selected(), Some(drawn.id));
    h.undo();
    assert!(h.annotations().is_empty());
    h.redo();
    assert_eq!(h.only(), drawn);

    // Select it by its edge and move it.
    h.press("v");
    assert_eq!(h.document().selected(), None);
    h.drag_image((70.0, 60.0), (90.0, 80.0));
    assert_eq!(
        h.only().kind.bounds(),
        ImageRect::new(90.0, 50.0, 110.0, 90.0)
    );
    h.undo();
    assert_eq!(h.only(), drawn);

    // Its bottom-right handle resizes it.
    h.click_image(70.0, 60.0);
    assert_eq!(h.document().selected(), Some(drawn.id));
    h.drag_image((180.0, 120.0), (250.0, 170.0));
    assert_eq!(
        h.only().kind.bounds(),
        ImageRect::new(70.0, 30.0, 180.0, 140.0)
    );
    h.undo();
    assert_eq!(h.only(), drawn);

    h.press("delete");
    assert!(h.annotations().is_empty());
    h.undo();
    assert_eq!(h.only(), drawn);
}

#[test]
fn escape_discards_a_draft_and_puts_back_a_moved_annotation() {
    let mut h = editor();
    let original = h.editor().preview_image().unwrap();
    h.press("a");
    h.down(h.at(70.0, 50.0));
    h.move_to(h.at(150.0, 90.0));
    h.move_to(h.at(200.0, 120.0));
    assert!(h.annotations().is_empty(), "a draft is not in the document");
    assert_ne!(*h.editor().preview_image().unwrap(), *original);
    h.press("escape");
    h.up(h.at(200.0, 120.0));
    assert!(h.annotations().is_empty());
    assert!(!h.document().can_undo());
    assert_eq!(*h.editor().preview_image().unwrap(), *original);

    h.drag_image((70.0, 50.0), (200.0, 120.0));
    let arrow = h.only();
    h.down(h.at(135.0, 85.0));
    h.move_to(h.at(160.0, 120.0));
    assert_ne!(h.only(), arrow);
    h.press("escape");
    h.up(h.at(160.0, 120.0));
    assert_eq!(h.only(), arrow);
    h.undo();
    assert!(
        h.annotations().is_empty(),
        "the cancelled move left no step"
    );
}

#[test]
fn enter_crops_to_the_selection() {
    let mut h = editor();
    h.drag_image((150.0, 100.0), (260.0, 200.0));
    assert_eq!(
        h.editor().selection,
        Some(ImageRect::new(150.0, 100.0, 110.0, 100.0))
    );
    assert!(h.shows("selection"));
    h.press("enter");
    assert_eq!(
        h.document().crop(),
        Some(ImageRect::new(150.0, 100.0, 110.0, 100.0))
    );
    assert_eq!(h.editor().selection, None);
    assert_eq!(h.export().dimensions(), (110, 100));
    h.undo();
    assert_eq!(h.document().crop(), None);
    assert_eq!(h.export().dimensions(), (400, 300));
}

#[test]
fn tool_keys_choose_tools_and_b_cycles_hide_styles() {
    let mut h = editor();
    for tool in Tool::ALL {
        let Some(key) = tool.shortcut() else { continue };
        h.press(&key.to_lowercase());
        assert_eq!(h.editor().tool, tool, "key {key}");
        h.press("escape");
    }

    h.press("b");
    assert_eq!(
        (h.editor().tool, h.editor().hide),
        (Tool::Hide, HideStyle::Blur)
    );
    h.drag_image((70.0, 30.0), (180.0, 120.0));
    let blur = h.only();
    assert!(matches!(blur.kind, AnnotationKind::Blur { .. }));
    // Each B moves the selected annotation to the next style and keeps its bounds.
    for (style, matches) in [
        (
            HideStyle::Pixelate,
            (|k| matches!(k, AnnotationKind::Pixelate { .. })) as fn(&AnnotationKind) -> bool,
        ),
        (HideStyle::Erase, |k| {
            matches!(k, AnnotationKind::RemoveFill { .. })
        }),
        (HideStyle::Redact, |k| {
            matches!(k, AnnotationKind::Redaction { .. })
        }),
        (HideStyle::Blur, |k| {
            matches!(k, AnnotationKind::Blur { .. })
        }),
    ] {
        h.press("b");
        assert_eq!(h.editor().hide, style);
        let changed = h.only();
        assert!(matches(&changed.kind), "{style:?}: {:?}", changed.kind);
        assert_eq!(changed.kind.bounds(), blur.kind.bounds());
    }
    h.undo();
    assert!(matches!(h.only().kind, AnnotationKind::Redaction { .. }));

    // The style buttons do the same.
    h.click(&format!("hide-style-{}", HideStyle::Pixelate as usize));
    assert!(matches!(h.only().kind, AnnotationKind::Pixelate { .. }));
}

#[test]
fn arrow_handles_edit_the_arrow_and_each_drag_is_one_undo_step() {
    let mut h = editor();
    h.press("a");
    h.drag_image((70.0, 50.0), (330.0, 190.0));
    let straight = h.only();
    let points = arrow_points(&straight);
    for index in 0..3 {
        let rect = h.rect(&format!("arrow-handle-{index}"));
        assert_eq!((rect[2], rect[3]), (8.0, 8.0));
    }
    assert!(!h.shows("arrow-handle-3"));

    // Dragging the shaft moves every point.
    h.drag_image((135.0, 85.0), (155.0, 105.0));
    for (before, after) in points.iter().zip(arrow_points(&h.only())) {
        assert!((after.x - before.x - 20.0).abs() < 0.01);
        assert!((after.y - before.y - 20.0).abs() < 0.01);
    }
    h.undo();
    assert_eq!(h.only(), straight);

    // The middle handle bends the shaft and leaves the ends.
    let middle = h.find("arrow-handle-1");
    h.drag(middle, (middle.0, middle.1 + 40.0));
    let bent = h.only();
    let bent_points = arrow_points(&bent);
    assert_eq!((bent_points[0], bent_points[2]), (points[0], points[2]));
    assert!((bent_points[1].y - points[1].y - 40.0).abs() < 0.01);
    h.undo();
    assert_eq!(h.only(), straight);
    h.redo();
    assert_eq!(h.only(), bent);

    // The end handle moves only the head.
    let end = h.find("arrow-handle-2");
    h.drag(end, (end.0 + 20.0, end.1 - 20.0));
    let moved = arrow_points(&h.only());
    assert_eq!(moved[0], bent_points[0]);
    assert!((moved[2].x - bent_points[2].x - 20.0).abs() < 0.01);
    assert!((moved[2].y - bent_points[2].y + 20.0).abs() < 0.01);
    h.undo();
    assert_eq!(h.only(), bent);
}

#[test]
fn text_entry_keeps_editor_shortcuts_out_and_adds_one_annotation() {
    let mut h = editor();
    h.click(&tool(Tool::Text));
    h.click_image(120.0, 90.0);
    assert!(h.shows("annotation-text"));
    // Tool keys and arrows type or move the caret while the field has focus.
    h.type_text("A smooth arrow 12");
    h.press("right");
    assert!(!h.shows("measure-preview"));
    assert_eq!(h.editor().tool, Tool::Text);
    h.click("add-text");
    assert!(!h.shows("annotation-text"));
    let text = h.only();
    assert!(
        matches!(&text.kind, AnnotationKind::Text { text, .. } if text == "A smooth arrow 12"),
        "{:?}",
        text.kind
    );
    assert_ne!(h.export(), *h.document().original());

    // A double click on it edits it, and Enter commits.
    h.press("v");
    let bounds = text.kind.bounds();
    let point = h.at(
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );
    h.down(point);
    h.up(point);
    h.down(point);
    h.up(point);
    assert!(h.shows("annotation-text"));
    assert!(h.shows("add-text"));
    h.press("ctrl-a");
    h.type_text("Edited");
    h.press("enter");
    assert!(matches!(&h.only().kind, AnnotationKind::Text { text, .. } if text == "Edited"));
    h.undo();
    assert_eq!(h.only(), text);
}
