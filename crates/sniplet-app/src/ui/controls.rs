//! Controls rune-kit does not have, built from its tokens and its own looks so they sit with
//! the kit's components.
use rune::{El, Shadow, el, row};
use rune_kit::Kit;

/// `e` as a floating surface, as rune-kit's menus draw theirs: the first layer, rounded, with
/// the floating elevation. Presses on it stay on it rather than reaching what is under it.
pub fn surface<S: 'static>(k: &Kit, e: El) -> El {
    k.elevate(
        e.fill(k.c.layer_01).radius(k.radius.lg),
        &k.elevation.floating,
    )
    .on_press(|_: &mut S| {})
}

/// `content` in a small floating label, such as the editor's status or a selection's size.
pub fn chip(k: &Kit, content: El) -> El {
    k.elevate(
        el().px(8.0).py(4.0).radius(k.radius.md).fill(k.c.layer_01),
        &k.elevation.raised,
    )
    .child(content)
}

/// The track of a segmented choice whose segments hold icons as well as text, in
/// `rune_kit::SegmentedControl`'s look. Fill it with [`segment`]s.
pub fn segments(k: &Kit, label: &str) -> El {
    row()
        .role("radiogroup")
        .label(label)
        .focus_group()
        .center_y()
        .gap(2.0)
        .padding(2.0)
        .radius(k.radius.md + 2.0)
        .fill(k.c.layer_02)
        .box_shadow(ring(k.c.border_base))
}

/// One segment of [`segments`]: raised while it is the choice, lit under the pointer when not.
/// Add its content and press handler.
pub fn segment(k: &Kit, key: String, label: &str, on: bool) -> El {
    let e = row()
        .key(key)
        .role("radio")
        .label(label)
        .checked(on)
        .focusable(true)
        .focus_selected(on)
        .focus_ring(2.0, 1.0, k.c.border_focus)
        .h(28.0)
        .min_w(32.0)
        .px(8.0)
        .gap(6.0)
        .center()
        .radius(k.radius.md)
        .cursor(rune::Cursor::Pointer);
    if on {
        e.fill(k.c.bg_base).box_shadow(ring(k.c.border_strong))
    } else {
        k.interactive(e)
    }
}

/// A kit button that shows it is the current choice, such as the selected tool: held in its
/// pressed fill, and read as checked.
pub fn selected(button: El, k: &Kit, on: bool) -> El {
    let button = button.checked(on);
    if on { button.fill(k.c.pressed) } else { button }
}

fn ring(color: rune::Color) -> Shadow {
    Shadow {
        spread: 0.5,
        color,
        ..Shadow::default()
    }
}
