use std::sync::Arc;

use gpui_kit::*;

use crate::editor::display_image;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DisplayGeometry {
    pub id: DisplayId,
    pub bounds: Bounds<Pixels>,
}

/// A frozen monitor image and the coordinate transform between its GPUI
/// overlay and the native desktop.
pub(crate) struct CaptureSurface {
    pub image: Arc<RenderImage>,
    origin: sniplet_platform::ScreenPoint,
    scale_factor: f32,
}

impl CaptureSurface {
    pub fn new(frame: sniplet_platform::CapturedFrame) -> Self {
        Self {
            image: display_image(frame.image),
            origin: frame.origin,
            scale_factor: frame.scale_factor,
        }
    }

    pub fn desktop_point(&self, position: Point<Pixels>) -> sniplet_platform::ScreenPoint {
        desktop_point(
            self.origin,
            self.scale_factor,
            position,
            cfg!(target_os = "windows"),
        )
    }

    pub fn window_bounds(&self, window: &sniplet_platform::WindowInfo) -> Bounds<Pixels> {
        window_bounds(
            self.origin,
            self.scale_factor,
            window,
            cfg!(target_os = "windows"),
        )
    }
}

/// Subtracts axis-aligned cuts from a rectangle. Each cut partitions an
/// intersecting piece into at most four non-overlapping rectangles.
pub(crate) fn subtract_rectangles(
    source: Bounds<Pixels>,
    cuts: &[Bounds<Pixels>],
) -> Vec<Bounds<Pixels>> {
    cuts.iter().fold(vec![source], |pieces, cut| {
        pieces
            .into_iter()
            .flat_map(|piece| subtract_rectangle(piece, *cut))
            .collect()
    })
}

fn subtract_rectangle(source: Bounds<Pixels>, cut: Bounds<Pixels>) -> Vec<Bounds<Pixels>> {
    let overlap = source.intersect(&cut);
    let overlap_width = f32::from(overlap.size.width);
    let overlap_height = f32::from(overlap.size.height);
    if overlap_width <= 0.0 || overlap_height <= 0.0 {
        return vec![source];
    }

    let left = f32::from(source.origin.x);
    let top = f32::from(source.origin.y);
    let right = left + f32::from(source.size.width);
    let bottom = top + f32::from(source.size.height);
    let cut_left = f32::from(overlap.origin.x);
    let cut_top = f32::from(overlap.origin.y);
    let cut_right = cut_left + overlap_width;
    let cut_bottom = cut_top + overlap_height;

    [
        rectangle(left, top, right, cut_top),
        rectangle(left, cut_bottom, right, bottom),
        rectangle(left, cut_top, cut_left, cut_bottom),
        rectangle(cut_right, cut_top, right, cut_bottom),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn rectangle(left: f32, top: f32, right: f32, bottom: f32) -> Option<Bounds<Pixels>> {
    (right > left && bottom > top).then(|| {
        Bounds::new(
            point(px(left), px(top)),
            size(px(right - left), px(bottom - top)),
        )
    })
}

pub(crate) fn captured_frame_placement(
    frame: &sniplet_platform::CapturedFrame,
    cx: &App,
) -> (Bounds<Pixels>, Option<DisplayId>) {
    let displays = cx
        .displays()
        .into_iter()
        .map(|display| DisplayGeometry {
            id: display.id(),
            bounds: display.bounds(),
        })
        .collect::<Vec<_>>();
    overlay_placement(
        captured_source_id(&frame.source),
        frame.origin,
        frame.image.dimensions(),
        frame.scale_factor,
        &displays,
        cfg!(any(target_os = "windows", target_os = "macos")),
        cfg!(target_os = "windows"),
    )
}

fn captured_source_id(source: &sniplet_platform::CaptureSource) -> u32 {
    match source {
        sniplet_platform::CaptureSource::Monitor { id, .. }
        | sniplet_platform::CaptureSource::MonitorRegion { id, .. }
        | sniplet_platform::CaptureSource::Window { id } => *id,
    }
}

pub(crate) fn overlay_placement(
    source_id: u32,
    origin: sniplet_platform::ScreenPoint,
    image_size: (u32, u32),
    scale_factor: f32,
    displays: &[DisplayGeometry],
    native_display_ids: bool,
    physical_origin: bool,
) -> (Bounds<Pixels>, Option<DisplayId>) {
    let scale = scale_factor.max(1.0);
    let origin_scale = if physical_origin { scale } else { 1.0 };
    let requested = Bounds::new(
        point(
            px(origin.x as f32 / origin_scale),
            px(origin.y as f32 / origin_scale),
        ),
        size(
            px(image_size.0 as f32 / scale),
            px(image_size.1 as f32 / scale),
        ),
    );

    if native_display_ids
        && let Some(display) = displays
            .iter()
            .find(|display| u64::from(display.id) as u32 == source_id)
    {
        return (display.bounds, Some(display.id));
    }

    let center = requested.center();
    let display = displays.iter().min_by(|left, right| {
        display_distance(left.bounds, center).total_cmp(&display_distance(right.bounds, center))
    });
    let Some(display) = display else {
        return (requested, None);
    };

    let same_size = (f32::from(display.bounds.size.width) - f32::from(requested.size.width)).abs()
        <= 2.0
        && (f32::from(display.bounds.size.height) - f32::from(requested.size.height)).abs() <= 2.0;
    (
        if same_size { display.bounds } else { requested },
        Some(display.id),
    )
}

fn display_distance(bounds: Bounds<Pixels>, point: Point<Pixels>) -> f32 {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let right = left + f32::from(bounds.size.width);
    let bottom = top + f32::from(bounds.size.height);
    let x = f32::from(point.x);
    let y = f32::from(point.y);
    let dx = if x < left {
        left - x
    } else if x > right {
        x - right
    } else {
        0.0
    };
    let dy = if y < top {
        top - y
    } else if y > bottom {
        y - bottom
    } else {
        0.0
    };
    dx * dx + dy * dy
}

fn desktop_point(
    origin: sniplet_platform::ScreenPoint,
    scale_factor: f32,
    position: Point<Pixels>,
    physical_coordinates: bool,
) -> sniplet_platform::ScreenPoint {
    let scale = if physical_coordinates {
        scale_factor.max(1.0)
    } else {
        1.0
    };
    sniplet_platform::ScreenPoint {
        x: origin
            .x
            .saturating_add((f32::from(position.x) * scale).floor() as i32),
        y: origin
            .y
            .saturating_add((f32::from(position.y) * scale).floor() as i32),
    }
}

fn window_bounds(
    origin: sniplet_platform::ScreenPoint,
    scale_factor: f32,
    window: &sniplet_platform::WindowInfo,
    physical_coordinates: bool,
) -> Bounds<Pixels> {
    let scale = if physical_coordinates {
        scale_factor.max(1.0)
    } else {
        1.0
    };
    Bounds::new(
        point(
            px((i64::from(window.x) - i64::from(origin.x)) as f32 / scale),
            px((i64::from(window.y) - i64::from(origin.y)) as f32 / scale),
        ),
        size(
            px(window.width as f32 / scale),
            px(window.height as f32 / scale),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(id: u64, x: f32, y: f32, width: f32, height: f32) -> DisplayGeometry {
        DisplayGeometry {
            id: DisplayId::new(id),
            bounds: Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
        }
    }

    fn window(x: i32, y: i32, width: u32, height: u32) -> sniplet_platform::WindowInfo {
        sniplet_platform::WindowInfo {
            id: 1,
            process_id: 2,
            app_name: "Example".into(),
            title: "Document".into(),
            x,
            y,
            width,
            height,
            z_order: 0,
            is_minimized: false,
            is_maximized: false,
            is_focused: false,
        }
    }

    #[::core::prelude::v1::test]
    fn windows_physical_origin_is_converted_to_gpui_coordinates() {
        let (bounds, display) = overlay_placement(
            1,
            sniplet_platform::ScreenPoint { x: 3840, y: 0 },
            (3840, 2160),
            1.5,
            &[],
            false,
            true,
        );

        assert_eq!(f32::from(bounds.origin.x), 2560.0);
        assert_eq!(f32::from(bounds.size.width), 2560.0);
        assert_eq!(display, None);
    }

    #[::core::prelude::v1::test]
    fn native_monitor_id_selects_its_exact_gpui_display() {
        let expected = geometry(42, 0.0, 0.0, 1728.0, 1117.0);
        let other = geometry(7, 0.0, 0.0, 1920.0, 1080.0);
        let (bounds, display) = overlay_placement(
            42,
            sniplet_platform::ScreenPoint { x: 4000, y: 0 },
            (3456, 2234),
            2.0,
            &[other, expected],
            true,
            false,
        );

        assert_eq!(bounds, expected.bounds);
        assert_eq!(display, Some(expected.id));
    }

    #[::core::prelude::v1::test]
    fn native_desktop_and_overlay_coordinates_round_trip() {
        let origin = sniplet_platform::ScreenPoint { x: -2880, y: 120 };
        let position = point(px(100.0), px(40.0));
        assert_eq!(
            desktop_point(origin, 1.5, position, true),
            sniplet_platform::ScreenPoint { x: -2730, y: 180 }
        );
        assert_eq!(
            window_bounds(origin, 1.5, &window(-2730, 180, 600, 300), true),
            Bounds::new(point(px(100.0), px(40.0)), size(px(400.0), px(200.0)))
        );
    }

    #[::core::prelude::v1::test]
    fn logical_desktop_coordinates_do_not_scale_window_geometry() {
        let origin = sniplet_platform::ScreenPoint { x: -1440, y: 180 };
        assert_eq!(
            desktop_point(origin, 2.0, point(px(100.0), px(40.0)), false),
            sniplet_platform::ScreenPoint { x: -1340, y: 220 }
        );
        assert_eq!(
            window_bounds(origin, 2.0, &window(-1340, 220, 400, 200), false),
            Bounds::new(point(px(100.0), px(40.0)), size(px(400.0), px(200.0)))
        );
    }

    fn area(bounds: Bounds<Pixels>) -> f32 {
        f32::from(bounds.size.width) * f32::from(bounds.size.height)
    }

    #[::core::prelude::v1::test]
    fn rectangle_subtraction_splits_around_a_center_cut_without_overlap() {
        let source = Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(80.0)));
        let cut = Bounds::new(point(px(20.0), px(10.0)), size(px(50.0), px(40.0)));
        let pieces = subtract_rectangles(source, &[cut]);

        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces.iter().copied().map(area).sum::<f32>(), 6_000.0);
        for (index, left) in pieces.iter().enumerate() {
            assert_eq!(area(left.intersect(&cut)), 0.0);
            for right in pieces.iter().skip(index + 1) {
                assert_eq!(area(left.intersect(right)), 0.0);
            }
        }
    }

    #[::core::prelude::v1::test]
    fn rectangle_subtraction_handles_overlapping_cuts_and_full_coverage() {
        let source = Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(100.0)));
        let cuts = [
            Bounds::new(point(px(20.0), px(0.0)), size(px(40.0), px(100.0))),
            Bounds::new(point(px(40.0), px(30.0)), size(px(60.0), px(40.0))),
        ];
        let pieces = subtract_rectangles(source, &cuts);

        assert_eq!(pieces.iter().copied().map(area).sum::<f32>(), 4_400.0);
        assert!(subtract_rectangles(source, &[source]).is_empty());
    }
}
