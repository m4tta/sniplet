use serde::{Deserialize, Serialize};

use crate::annotation::ArrowVariant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub(crate) fn distance_to(self, other: Self) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ImageRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_corners(start: Point, end: Point) -> Self {
        Self::new(start.x, start.y, end.x - start.x, end.y - start.y).normalized()
    }

    pub fn normalized(self) -> Self {
        let x = if self.width < 0.0 {
            self.x + self.width
        } else {
            self.x
        };
        let y = if self.height < 0.0 {
            self.y + self.height
        } else {
            self.y
        };
        Self::new(x, y, self.width.abs(), self.height.abs())
    }

    pub fn clipped(self, image_width: u32, image_height: u32) -> Self {
        let rect = self.normalized();
        let left = rect.x.clamp(0.0, image_width as f32);
        let top = rect.y.clamp(0.0, image_height as f32);
        let right = (rect.x + rect.width).clamp(0.0, image_width as f32);
        let bottom = (rect.y + rect.height).clamp(0.0, image_height as f32);
        Self::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    pub fn contains(self, point: Point) -> bool {
        let rect = self.normalized();
        point.x >= rect.x
            && point.x <= rect.x + rect.width
            && point.y >= rect.y
            && point.y <= rect.y + rect.height
    }

    pub fn expanded(self, amount: f32) -> Self {
        let rect = self.normalized();
        Self::new(
            rect.x - amount,
            rect.y - amount,
            rect.width + amount * 2.0,
            rect.height + amount * 2.0,
        )
    }

    pub fn translated(self, delta: Point) -> Self {
        Self::new(self.x + delta.x, self.y + delta.y, self.width, self.height)
    }

    pub(crate) fn pixel_bounds(self, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let rect = self.clipped(width, height);
        let left = rect.x.floor() as u32;
        let top = rect.y.floor() as u32;
        let right = (rect.x + rect.width).ceil().min(width as f32) as u32;
        let bottom = (rect.y + rect.height).ceil().min(height as f32) as u32;
        (right > left && bottom > top).then_some((left, top, right, bottom))
    }
}

pub(crate) fn distance_to_segment(point: Point, start: Point, end: Point) -> f32 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return point.distance_to(start);
    }
    let t =
        (((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared).clamp(0.0, 1.0);
    point.distance_to(Point::new(start.x + t * dx, start.y + t * dy))
}

pub(crate) fn quadratic_control(start: Point, middle: Point, end: Point) -> Point {
    Point::new(
        middle.x * 2.0 - (start.x + end.x) * 0.5,
        middle.y * 2.0 - (start.y + end.y) * 0.5,
    )
}

pub(crate) fn quadratic_point(start: Point, control: Point, end: Point, t: f32) -> Point {
    let one_minus_t = 1.0 - t;
    Point::new(
        one_minus_t * one_minus_t * start.x + 2.0 * one_minus_t * t * control.x + t * t * end.x,
        one_minus_t * one_minus_t * start.y + 2.0 * one_minus_t * t * control.y + t * t * end.y,
    )
}

pub(crate) fn quadratic_bounds(start: Point, control: Point, end: Point) -> ImageRect {
    let mut left = start.x.min(end.x);
    let mut right = start.x.max(end.x);
    let mut top = start.y.min(end.y);
    let mut bottom = start.y.max(end.y);

    for (start_axis, control_axis, end_axis, minimum, maximum) in [
        (start.x, control.x, end.x, &mut left, &mut right),
        (start.y, control.y, end.y, &mut top, &mut bottom),
    ] {
        let denominator = start_axis - 2.0 * control_axis + end_axis;
        if denominator.abs() > f32::EPSILON {
            let t = (start_axis - control_axis) / denominator;
            if (0.0..1.0).contains(&t) {
                let value = (1.0 - t).powi(2) * start_axis
                    + 2.0 * (1.0 - t) * t * control_axis
                    + t * t * end_axis;
                *minimum = minimum.min(value);
                *maximum = maximum.max(value);
            }
        }
    }

    ImageRect::new(left, top, right - left, bottom - top)
}

pub(crate) fn distance_to_quadratic(point: Point, start: Point, control: Point, end: Point) -> f32 {
    let estimated_length = start.distance_to(control) + control.distance_to(end);
    let steps = (estimated_length / 4.0).ceil().clamp(8.0, 128.0) as usize;
    let mut minimum = f32::INFINITY;
    let mut previous = start;
    for index in 1..=steps {
        let current = quadratic_point(start, control, end, index as f32 / steps as f32);
        minimum = minimum.min(distance_to_segment(point, previous, current));
        previous = current;
    }
    minimum
}

pub(crate) fn arrow_head_points(
    start: Point,
    control: Point,
    end: Point,
    stroke_width: f32,
) -> [Point; 3] {
    let mut dx = end.x - control.x;
    let mut dy = end.y - control.y;
    let mut length = dx.hypot(dy);
    if length <= f32::EPSILON {
        dx = end.x - start.x;
        dy = end.y - start.y;
        length = dx.hypot(dy);
    }
    if length <= f32::EPSILON {
        return [end; 3];
    }

    let width = stroke_width.max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let base = Point::new(end.x - ux * width * 3.0, end.y - uy * width * 3.0);
    let half_width = width * 1.5;
    [
        end,
        Point::new(base.x - uy * half_width, base.y + ux * half_width),
        Point::new(base.x + uy * half_width, base.y - ux * half_width),
    ]
}

pub(crate) const OPEN_ARROW_STROKE_SCALE: f32 = 0.35;
const THIN_HEAD_DEPTH_SCALE: f32 = 2.2;
const THIN_HEAD_HALF_SPAN_SCALE: f32 = 1.1;
const HAND_DRAWN_HEAD_DEPTH_SCALE: f32 = 2.6;
const HAND_DRAWN_HEAD_HALF_SPAN_SCALE: f32 = 1.35;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct QuadraticCurve {
    pub start: Point,
    pub control: Point,
    pub end: Point,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ArrowHeadGeometry {
    Filled([Point; 3]),
    Open([QuadraticCurve; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ArrowGeometry {
    pub control: Point,
    pub stroke_width: f32,
    pub start_head: Option<ArrowHeadGeometry>,
    pub end_head: Option<ArrowHeadGeometry>,
}

impl ArrowGeometry {
    pub(crate) fn heads(self) -> impl Iterator<Item = ArrowHeadGeometry> {
        self.start_head.into_iter().chain(self.end_head)
    }
}

pub(crate) fn arrow_geometry(
    start: Point,
    middle: Point,
    end: Point,
    size: f32,
    variant: ArrowVariant,
) -> ArrowGeometry {
    let size = size.max(1.0);
    let control = quadratic_control(start, middle, end);
    if variant == ArrowVariant::Solid {
        return ArrowGeometry {
            control,
            stroke_width: size,
            start_head: None,
            end_head: Some(ArrowHeadGeometry::Filled(arrow_head_points(
                start, control, end, size,
            ))),
        };
    }

    let stroke_width = (size * OPEN_ARROW_STROKE_SCALE).max(1.0);
    let (depth, half_span, bow) = match variant {
        ArrowVariant::HandDrawn => (
            size * HAND_DRAWN_HEAD_DEPTH_SCALE,
            size * HAND_DRAWN_HEAD_HALF_SPAN_SCALE,
            size * 0.12,
        ),
        ArrowVariant::Thin | ArrowVariant::DoubleEnded => (
            size * THIN_HEAD_DEPTH_SCALE,
            size * THIN_HEAD_HALF_SPAN_SCALE,
            0.0,
        ),
        ArrowVariant::Solid => unreachable!(),
    };
    let end_head = endpoint_direction(start, control, end, false)
        .map(|inward| ArrowHeadGeometry::Open(open_arrow_head(end, inward, depth, half_span, bow)));
    let start_head = (variant == ArrowVariant::DoubleEnded)
        .then(|| endpoint_direction(start, control, end, true))
        .flatten()
        .map(|inward| {
            ArrowHeadGeometry::Open(open_arrow_head(start, inward, depth, half_span, bow))
        });

    ArrowGeometry {
        control,
        stroke_width,
        start_head,
        end_head,
    }
}

fn endpoint_direction(start: Point, control: Point, end: Point, at_start: bool) -> Option<Point> {
    let (mut dx, mut dy) = if at_start {
        (control.x - start.x, control.y - start.y)
    } else {
        (control.x - end.x, control.y - end.y)
    };
    let mut length = dx.hypot(dy);
    if length <= f32::EPSILON {
        (dx, dy) = if at_start {
            (end.x - start.x, end.y - start.y)
        } else {
            (start.x - end.x, start.y - end.y)
        };
        length = dx.hypot(dy);
    }
    (length > f32::EPSILON).then(|| Point::new(dx / length, dy / length))
}

fn open_arrow_head(
    tip: Point,
    inward: Point,
    depth: f32,
    half_span: f32,
    bow: f32,
) -> [QuadraticCurve; 2] {
    let normal = Point::new(-inward.y, inward.x);
    let base = Point::new(tip.x + inward.x * depth, tip.y + inward.y * depth);
    let left = Point::new(base.x + normal.x * half_span, base.y + normal.y * half_span);
    let right = Point::new(base.x - normal.x * half_span, base.y - normal.y * half_span);
    [
        QuadraticCurve {
            start: tip,
            control: Point::new(
                (tip.x + left.x) * 0.5 + normal.x * bow,
                (tip.y + left.y) * 0.5 + normal.y * bow,
            ),
            end: left,
        },
        QuadraticCurve {
            start: tip,
            control: Point::new(
                (tip.x + right.x) * 0.5 - normal.x * bow,
                (tip.y + right.y) * 0.5 - normal.y * bow,
            ),
            end: right,
        },
    ]
}

pub(crate) fn point_in_triangle(point: Point, triangle: [Point; 3]) -> bool {
    let cross =
        |a: Point, b: Point, p: Point| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    if cross(triangle[0], triangle[1], triangle[2]).abs() <= f32::EPSILON {
        return false;
    }
    let first = cross(triangle[0], triangle[1], point);
    let second = cross(triangle[1], triangle[2], point);
    let third = cross(triangle[2], triangle[0], point);
    (first >= 0.0 && second >= 0.0 && third >= 0.0)
        || (first <= 0.0 && second <= 0.0 && third <= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_variants_share_the_documented_width_mapping() {
        let start = Point::new(10.0, 20.0);
        let middle = Point::new(60.0, 20.0);
        let end = Point::new(110.0, 20.0);
        for variant in [
            ArrowVariant::HandDrawn,
            ArrowVariant::Thin,
            ArrowVariant::DoubleEnded,
        ] {
            assert_eq!(
                arrow_geometry(start, middle, end, 10.0, variant).stroke_width,
                3.5
            );
            assert_eq!(
                arrow_geometry(start, middle, end, 1.0, variant).stroke_width,
                1.0
            );
        }
    }

    #[test]
    fn double_ended_reuses_thin_end_geometry_and_adds_the_start_head() {
        let start = Point::new(10.0, 80.0);
        let middle = Point::new(60.0, 10.0);
        let end = Point::new(110.0, 80.0);
        let thin = arrow_geometry(start, middle, end, 12.0, ArrowVariant::Thin);
        let double = arrow_geometry(start, middle, end, 12.0, ArrowVariant::DoubleEnded);

        assert_eq!(double.stroke_width, thin.stroke_width);
        assert_eq!(double.end_head, thin.end_head);
        assert!(thin.start_head.is_none());
        assert!(double.start_head.is_some());
    }
}
