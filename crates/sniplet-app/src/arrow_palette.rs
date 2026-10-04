use gpui_kit::base::{Slider, SliderIndicator, SliderThumb, SliderTrack, slider::SliderState};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, Bounds, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Path,
    PathBuilder, Pixels, Role, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    canvas, div, point, px, relative, rgb,
};
use sniplet_core::ArrowVariant;

const TRACK_WIDTH: f32 = 140.0;
const THUMB_SIZE: f32 = 18.0;

/// The compact tapered size control used by the arrow palette.
///
/// `Slider`, `SliderTrack`, `SliderIndicator`, and `SliderThumb` own all pointer
/// interaction. This function only supplies their layout and presentation.
pub(crate) fn size_slider(state: &Entity<SliderState>, cx: &App) -> impl IntoElement {
    let percentage = state.read(cx).percentage().end;
    let percentage = if percentage.is_finite() {
        percentage.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let track_color = cx.theme().muted_foreground;
    let thumb_border = cx.theme().border;

    div()
        .id("arrow-size-slider")
        .test_support()
        .accessibility_id("arrow-size-slider")
        .role(Role::Slider)
        .aria_label("Arrow size")
        .w(px(TRACK_WIDTH + THUMB_SIZE))
        .h(px(32.0))
        .px(px(THUMB_SIZE / 2.0))
        .child(
            Slider::new(state)
                .horizontal()
                .flex()
                .size_full()
                .items_center()
                .child(
                    SliderTrack::new(state)
                        .cursor_pointer()
                        .relative()
                        .flex()
                        .size_full()
                        .items_center()
                        .child(
                            SliderIndicator::new(state)
                                .relative()
                                .w_full()
                                .h(px(THUMB_SIZE))
                                .child(
                                    div()
                                        .id("arrow-size-track")
                                        .test_support()
                                        .absolute()
                                        .size_full()
                                        .child(tapered_track(track_color)),
                                )
                                .child(
                                    SliderThumb::new(state)
                                        .cursor_pointer()
                                        .absolute()
                                        .top_0()
                                        .left(relative(percentage))
                                        .ml(-px(THUMB_SIZE / 2.0))
                                        .size(px(THUMB_SIZE))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(thumb_border)
                                        .bg(rgb(0xffffff)),
                                ),
                        ),
                ),
        )
}

fn tapered_track(color: Hsla) -> impl IntoElement {
    canvas(
        move |bounds, _, _| tapered_outline(bounds),
        move |_, path, window, _| {
            if let Some(path) = path {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .size_full()
}

fn tapered_outline(bounds: Bounds<Pixels>) -> Option<Path<Pixels>> {
    let inset = px(1.0);
    let center_y = bounds.origin.y + bounds.size.height / 2.0;
    let mut path = PathBuilder::stroke(px(1.25));
    path.move_to(point(bounds.origin.x + inset, center_y - px(1.0)));
    path.line_to(point(bounds.right() - inset, center_y - px(8.0)));
    path.line_to(point(bounds.right() - inset, center_y + px(8.0)));
    path.line_to(point(bounds.origin.x + inset, center_y + px(1.0)));
    path.close();
    path.build().ok()
}

/// Paints the 24px arrow-style glyph used inside the palette's segmented buttons.
pub(crate) fn variant_icon(variant: ArrowVariant, color: Hsla) -> impl IntoElement {
    canvas(
        move |bounds, _, _| variant_paths(variant, bounds),
        move |_, paths, window, _| {
            for path in paths {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(24.0))
}

fn variant_paths(variant: ArrowVariant, bounds: Bounds<Pixels>) -> Vec<Path<Pixels>> {
    let scale = (bounds.size.width.min(bounds.size.height).as_f32() / 24.0).max(0.0);
    if scale == 0.0 {
        return Vec::new();
    }
    let origin = point(
        bounds.origin.x + (bounds.size.width - px(24.0 * scale)) / 2.0,
        bounds.origin.y + (bounds.size.height - px(24.0 * scale)) / 2.0,
    );
    let at = |x: f32, y: f32| point(origin.x + px(x * scale), origin.y + px(y * scale));
    let mut path = PathBuilder::stroke(px(if variant == ArrowVariant::Solid {
        1.6
    } else {
        1.8
    } * scale));

    match variant {
        ArrowVariant::Solid => {
            path.move_to(at(2.0, 9.0));
            path.line_to(at(13.5, 9.0));
            path.line_to(at(13.5, 5.0));
            path.line_to(at(22.0, 12.0));
            path.line_to(at(13.5, 19.0));
            path.line_to(at(13.5, 15.0));
            path.line_to(at(2.0, 15.0));
            path.close();
        }
        ArrowVariant::HandDrawn => {
            path.move_to(at(2.0, 13.0));
            path.cubic_bezier_to(at(21.0, 11.0), at(7.0, 9.8), at(14.0, 14.2));
            path.move_to(at(15.0, 5.5));
            path.cubic_bezier_to(at(21.0, 11.0), at(17.5, 6.5), at(19.5, 8.5));
            path.cubic_bezier_to(at(15.0, 18.5), at(19.0, 14.0), at(17.2, 17.0));
        }
        ArrowVariant::Thin => {
            path.move_to(at(2.5, 12.0));
            path.line_to(at(21.5, 12.0));
            path.move_to(at(15.5, 6.0));
            path.line_to(at(21.5, 12.0));
            path.line_to(at(15.5, 18.0));
        }
        ArrowVariant::DoubleEnded => {
            path.move_to(at(3.0, 12.0));
            path.line_to(at(21.0, 12.0));
            path.move_to(at(15.0, 6.0));
            path.line_to(at(21.0, 12.0));
            path.line_to(at(15.0, 18.0));
            path.move_to(at(9.0, 6.0));
            path.line_to(at(3.0, 12.0));
            path.line_to(at(9.0, 18.0));
        }
    }

    path.build().into_iter().collect()
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Context, Render, TestAppContext, Window, WindowOptions, point,
    };

    struct Harness {
        state: Entity<SliderState>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            size_slider(&self.state, cx)
        }
    }

    #[gpui_kit::test]
    fn pointer_changes_size_through_base_slider(cx: &mut TestAppContext) {
        let (window, result) = cx.update(|cx| {
            gpui_kit::init(cx);
            let state = cx.new(|_| {
                SliderState::new()
                    .min(1.0)
                    .max(30.0)
                    .step(0.25)
                    .default_value(10.0)
            });
            let result = state.clone();
            let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Harness { state })
            })
            .unwrap();
            (window, result)
        });

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let track = window.find("arrow-size-track").bounds();
            window.click_at(
                "arrow-size-track",
                point(track.size.width / 2.0, track.size.height / 2.0),
                cx,
            );
            assert!((result.read(cx).value().end() - 15.5).abs() <= 0.25);
        })
        .unwrap();
    }
}
