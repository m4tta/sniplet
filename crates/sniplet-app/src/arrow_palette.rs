use rune::{Color, Drag, El, LineCap, Path, Phase, Shape, Tx, canvas, el};
use sniplet_core::ArrowVariant;

use crate::ui::Palette;

const TRACK_WIDTH: f32 = 140.0;
const THUMB_SIZE: f32 = 18.0;

/// The compact tapered size control shared by every tool in the properties popover.
/// `fraction` is the value as a 0–1 share of the range. `change` hears every press and move
/// with the new fraction and the drag's phase, so the host can group one drag into one undo
/// step and close the group on `Phase::Up`.
pub(crate) fn size_slider<S: 'static>(
    p: &Palette,
    label: &'static str,
    fraction: f32,
    change: impl Fn(&mut S, f32, Phase, &mut Tx) + 'static,
) -> El {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let track = canvas(tapered_track(p.muted_foreground)).size(TRACK_WIDTH, THUMB_SIZE);
    let thumb = el()
        .key("size-thumb")
        .size(THUMB_SIZE, THUMB_SIZE)
        .radius(THUMB_SIZE / 2.0)
        .fill(Color::WHITE)
        .border(1.0, p.border)
        .absolute(fraction * TRACK_WIDTH, 7.0);
    el().key("size-slider")
        .role("slider")
        .label(label)
        .a11y_value(format!("{:.0}%", fraction * 100.0))
        .w(TRACK_WIDTH + THUMB_SIZE)
        .h(32.0)
        .cursor(rune::Cursor::Pointer)
        .child(
            el().key("size-track")
                .absolute(THUMB_SIZE / 2.0, 7.0)
                .child(track),
        )
        .child(thumb)
        .on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
            let value = ((drag.x - THUMB_SIZE / 2.0) / TRACK_WIDTH).clamp(0.0, 1.0);
            change(s, value, drag.phase, tx);
        })
}

/// A thin wedge that widens to the right, so the track reads as "bigger this way".
fn tapered_track(color: Color) -> Shape {
    let (w, mid) = (TRACK_WIDTH, THUMB_SIZE / 2.0);
    let wedge = Path::new()
        .move_to([1.0, mid - 1.0])
        .line_to([w - 1.0, mid - 8.0])
        .line_to([w - 1.0, mid + 8.0])
        .line_to([1.0, mid + 1.0])
        .close();
    Shape::new().stroke_path(&wedge, 1.25, LineCap::Round, color)
}

/// The 24 pt arrow-style glyph inside the palette's segmented buttons.
pub(crate) fn variant_icon(variant: ArrowVariant, color: Color) -> El {
    let width = if variant == ArrowVariant::Solid {
        1.6
    } else {
        1.8
    };
    let shape = Shape::new().stroke_path(&variant_path(variant), width, LineCap::Round, color);
    canvas(shape).size(24.0, 24.0)
}

/// A variant's glyph in a 24 × 24 box.
fn variant_path(variant: ArrowVariant) -> Path {
    let path = Path::new();
    match variant {
        ArrowVariant::Solid => path
            .move_to([2.0, 9.0])
            .line_to([13.5, 9.0])
            .line_to([13.5, 5.0])
            .line_to([22.0, 12.0])
            .line_to([13.5, 19.0])
            .line_to([13.5, 15.0])
            .line_to([2.0, 15.0])
            .close(),
        ArrowVariant::HandDrawn => path
            .move_to([2.0, 13.0])
            .cubic_to([7.0, 9.8], [14.0, 14.2], [21.0, 11.0])
            .move_to([15.0, 5.5])
            .cubic_to([17.5, 6.5], [19.5, 8.5], [21.0, 11.0])
            .cubic_to([19.0, 14.0], [17.2, 17.0], [15.0, 18.5]),
        ArrowVariant::Thin => path
            .move_to([2.5, 12.0])
            .line_to([21.5, 12.0])
            .move_to([15.5, 6.0])
            .line_to([21.5, 12.0])
            .line_to([15.5, 18.0]),
        ArrowVariant::DoubleEnded => path
            .move_to([3.0, 12.0])
            .line_to([21.0, 12.0])
            .move_to([15.0, 6.0])
            .line_to([21.0, 12.0])
            .line_to([15.0, 18.0])
            .move_to([9.0, 6.0])
            .line_to([3.0, 12.0])
            .line_to([9.0, 18.0]),
    }
}
