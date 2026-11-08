//! The properties strip: the size slider, arrow variants, colors, fill and counter values.
use sniplet_core::{AnnotationKind, ArrowVariant, Color};

use super::{Harness, arrow_points, editor};
use crate::tools::{Size, Tool};

/// The window point on the size slider that sets `value`.
fn slider_at(h: &Harness, size: Size, value: f32) -> (f32, f32) {
    let r = h.rect("size-slider");
    // The track starts half a thumb (9 pt) in and is 140 pt long.
    (r[0] + 9.0 + 140.0 * size.fraction(value), r[1] + r[3] / 2.0)
}

/// An editor with one selected arrow.
fn arrow() -> Harness {
    let mut h = editor();
    h.press("a");
    h.drag_image((70.0, 50.0), (330.0, 190.0));
    h
}

#[test]
fn arrows_are_bold_and_remember_their_size_for_the_next_one() {
    let mut h = arrow();
    assert_eq!(h.only().style.stroke_width, Size::Arrow.default_value());
    h.click_at(slider_at(&h, Size::Arrow, 14.0));
    assert_eq!(h.only().style.stroke_width, 14.0);

    h.press("r");
    h.drag_image((340.0, 200.0), (380.0, 240.0));
    assert_eq!(h.annotations()[1].style.stroke_width, 3.0);
    h.press("a");
    h.drag_image((60.0, 230.0), (260.0, 260.0));
    assert_eq!(h.annotations()[2].style.stroke_width, 14.0);
}

#[test]
fn a_size_drag_is_live_and_one_undo_step_even_released_off_the_slider() {
    let mut h = arrow();
    let original = h.only();
    h.down(slider_at(&h, Size::Arrow, 20.0));
    for value in [22.5, 25.0, 28.0] {
        h.move_to(slider_at(&h, Size::Arrow, value));
        assert_eq!(h.only().style.stroke_width, value, "the drag updates live");
    }
    // Leave the slider straight down and release there.
    let end = slider_at(&h, Size::Arrow, 28.0);
    h.move_to((end.0, end.1 + 150.0));
    h.up((end.0, end.1 + 150.0));
    let resized = h.only();
    assert_eq!(resized.style.stroke_width, 28.0);
    h.undo();
    assert_eq!(h.only(), original);
    h.redo();
    assert_eq!(h.only(), resized);
    h.undo();
    h.undo();
    assert!(h.annotations().is_empty());
}

#[test]
fn escape_during_a_size_drag_restores_the_size() {
    let mut h = arrow();
    let original = h.only();
    h.down(h.find("size-thumb"));
    h.move_to(slider_at(&h, Size::Arrow, 18.0));
    h.move_to(slider_at(&h, Size::Arrow, 24.0));
    assert_eq!(h.only().style.stroke_width, 24.0);
    h.press("escape");
    assert_eq!(h.only(), original);
    // Later moves and the release belong to no drag.
    let end = slider_at(&h, Size::Arrow, 30.0);
    h.move_to(end);
    h.up(end);
    assert_eq!(h.only(), original);
    h.undo();
    assert!(h.annotations().is_empty());

    // New arrows keep the size from before the drag too.
    h.press("a");
    h.drag_image((60.0, 230.0), (260.0, 260.0));
    assert_eq!(h.only().style.stroke_width, original.style.stroke_width);
}

#[test]
fn arrow_variants_keep_the_geometry_undo_and_carry_to_new_arrows() {
    let mut h = arrow();
    let middle = h.find("arrow-handle-1");
    h.drag(middle, (middle.0, middle.1 + 35.0));
    h.click("color-4");
    let initial = h.only();
    assert_eq!(initial.style.stroke, Color::new(0, 122, 255, 255));
    let mut exports = vec![h.export()];
    for variant in [
        ArrowVariant::HandDrawn,
        ArrowVariant::Thin,
        ArrowVariant::DoubleEnded,
    ] {
        let before = h.only();
        h.click(&format!("arrow-variant-{}", variant as usize));
        let changed = h.only();
        assert_eq!(arrow_points(&changed), arrow_points(&initial));
        assert_eq!(changed.style, initial.style);
        assert!(matches!(changed.kind, AnnotationKind::Arrow { variant: v, .. } if v == variant));
        let pixels = h.export();
        assert!(exports.iter().all(|other| *other != pixels));
        h.undo();
        assert_eq!(h.only(), before);
        h.redo();
        assert_eq!(h.only(), changed);
        exports.push(pixels);
    }
    h.press("v");
    h.press("a");
    h.drag_image((60.0, 230.0), (260.0, 260.0));
    assert!(matches!(
        h.annotations()[1].kind,
        AnnotationKind::Arrow {
            variant: ArrowVariant::DoubleEnded,
            ..
        }
    ));
}

#[test]
fn colors_and_fill_change_the_selected_shape() {
    let mut h = editor();
    h.click(&format!("tool-{}", Tool::Rectangle as usize));
    h.drag_image((70.0, 30.0), (180.0, 120.0));
    h.click("color-4");
    assert_eq!(h.only().style.stroke, Color::new(0, 122, 255, 255));
    h.click("fill");
    assert_eq!(h.only().style.fill, Some(Color::new(0, 122, 255, 255)));
    h.undo();
    assert_eq!(h.only().style.fill, None);
}

#[test]
fn each_text_size_click_is_one_undo_step() {
    let mut h = editor();
    h.press("t");
    h.click_image(120.0, 90.0);
    h.type_text("Resizable");
    h.press("enter");
    let font_size = |h: &Harness| match h.only().kind {
        AnnotationKind::Text { font_size, .. } => font_size,
        kind => panic!("expected text, got {kind:?}"),
    };
    let initial = h.export();
    h.click_at(slider_at(&h, Size::Text, 40.0));
    assert_eq!(font_size(&h), 40.0);
    let enlarged = h.export();
    assert_ne!(enlarged, initial);
    h.click_at(slider_at(&h, Size::Text, 60.0));
    assert_eq!(font_size(&h), 60.0);
    h.undo();
    assert_eq!(h.export(), enlarged);
    h.undo();
    assert_eq!(h.export(), initial);
}

#[test]
fn counter_buttons_clamp_and_set_the_next_number() {
    let mut h = editor();
    h.press("c");
    h.click_image(120.0, 90.0);
    let before = h.export();
    h.click("counter-value-down");
    h.click("counter-value-down");
    assert!(matches!(
        h.only().kind,
        AnnotationKind::Counter { value: 0, .. }
    ));
    assert_ne!(h.export(), before);

    h.click_image(200.0, 130.0);
    h.click("counter-value-up");
    h.click_image(280.0, 170.0);
    let values: Vec<u32> = h
        .annotations()
        .iter()
        .map(|annotation| match annotation.kind {
            AnnotationKind::Counter { value, .. } => value,
            _ => panic!("expected a counter"),
        })
        .collect();
    assert_eq!(values, [0, 2, 3]);
}
