use serde::{Deserialize, Serialize};

use crate::{
    Measurement,
    color::Color,
    document::ImageSize,
    geometry::{
        ArrowHeadGeometry, ImageRect, Point, arrow_geometry, distance_to_quadratic,
        distance_to_segment, point_in_triangle, quadratic_bounds, quadratic_control,
    },
};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct AnnotationId(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: AnnotationId,
    pub kind: AnnotationKind,
    pub style: AnnotationStyle,
}

/// Visual treatment for an arrow while preserving the same three editing handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowVariant {
    #[default]
    Solid,
    HandDrawn,
    Thin,
    DoubleEnded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagnifierPart {
    Source,
    Lens,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnnotationKind {
    /// An imprinted ruler is one raster edit, with its label and end caps intact.
    Measurement {
        measurement: Measurement,
    },
    Line {
        start: Point,
        end: Point,
    },
    Arrow {
        start: Point,
        end: Point,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bend: Option<Point>,
        #[serde(default)]
        variant: ArrowVariant,
    },
    Rectangle {
        rect: ImageRect,
    },
    Ellipse {
        rect: ImageRect,
    },
    Text {
        origin: Point,
        text: String,
        font_size: f32,
    },
    Counter {
        center: Point,
        value: u32,
        font_size: f32,
    },
    Pixelate {
        rect: ImageRect,
        block_size: u32,
    },
    Blur {
        rect: ImageRect,
        radius: f32,
    },
    Highlight {
        rect: ImageRect,
    },
    /// Dims the screenshot outside this region. Multiple spotlights expose the
    /// union of their regions.
    Spotlight {
        rect: ImageRect,
    },
    Freehand {
        points: Vec<Point>,
    },
    Redaction {
        rect: ImageRect,
    },
    /// Covers an area with a source color sampled at `sample`. When omitted,
    /// the renderer averages the pixels immediately around the rectangle.
    RemoveFill {
        rect: ImageRect,
        sample: Option<Point>,
    },
    /// A pasted PNG stored with the project and composited at `origin`.
    Image {
        origin: Point,
        png_bytes: Vec<u8>,
        #[serde(default)]
        size: Option<ImageSize>,
    },
    /// Enlarges pixels at `source` in a linked circular callout. Old projects
    /// without `source` retain their lens at `rect`.
    Magnifier {
        rect: ImageRect,
        zoom: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<Point>,
    },
}

fn rect_center(rect: ImageRect) -> Point {
    Point::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5)
}

fn circle_rect(center: Point, radius: f32) -> ImageRect {
    ImageRect::new(
        center.x - radius,
        center.y - radius,
        radius * 2.0,
        radius * 2.0,
    )
}

fn valid_magnifier_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(1.0, 16.0)
    } else {
        1.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CompositeLayer {
    Raster,
    Image,
    Spotlight,
    Drawing,
}

impl AnnotationKind {
    /// Raster edits affect the base. Image objects remain below drawings and shade.
    pub(crate) fn composite_layer(&self) -> CompositeLayer {
        match self {
            Self::Pixelate { .. }
            | Self::Blur { .. }
            | Self::Redaction { .. }
            | Self::Measurement { .. }
            | Self::RemoveFill { .. } => CompositeLayer::Raster,
            Self::Image { .. } => CompositeLayer::Image,
            Self::Spotlight { .. } => CompositeLayer::Spotlight,
            _ => CompositeLayer::Drawing,
        }
    }

    pub fn bounds(&self) -> ImageRect {
        match self {
            Self::Measurement { measurement } => {
                ImageRect::from_corners(measurement.start, measurement.end)
            }
            Self::Line { start, end } => ImageRect::from_corners(*start, *end),
            Self::Arrow {
                start, end, bend, ..
            } => {
                let middle = bend.unwrap_or_else(|| midpoint(*start, *end));
                quadratic_bounds(*start, quadratic_control(*start, middle, *end), *end)
            }
            Self::Rectangle { rect }
            | Self::Ellipse { rect }
            | Self::Pixelate { rect, .. }
            | Self::Blur { rect, .. }
            | Self::Highlight { rect }
            | Self::Spotlight { rect }
            | Self::Redaction { rect }
            | Self::RemoveFill { rect, .. } => rect.normalized(),
            Self::Magnifier { rect, .. } => {
                self.magnifier_circles()
                    .map_or(rect.normalized(), |[source, lens]| {
                        let left = source.x.min(lens.x);
                        let top = source.y.min(lens.y);
                        ImageRect::new(
                            left,
                            top,
                            (source.x + source.width).max(lens.x + lens.width) - left,
                            (source.y + source.height).max(lens.y + lens.height) - top,
                        )
                    })
            }
            Self::Text {
                origin,
                text,
                font_size,
            } => ImageRect::new(
                origin.x,
                origin.y,
                text.chars().count() as f32 * font_size * 0.62,
                font_size * 1.25,
            ),
            Self::Counter {
                center, font_size, ..
            } => ImageRect::new(
                center.x - font_size * 0.65,
                center.y - font_size * 0.65,
                font_size * 1.3,
                font_size * 1.3,
            ),
            Self::Freehand { points } => points_bounds(points),
            Self::Image {
                origin,
                png_bytes,
                size,
            } => size
                .map(|size| {
                    ImageRect::new(origin.x, origin.y, size.width as f32, size.height as f32)
                })
                .or_else(|| {
                    image::load_from_memory(png_bytes).ok().map(|image| {
                        ImageRect::new(
                            origin.x,
                            origin.y,
                            image.width() as f32,
                            image.height() as f32,
                        )
                    })
                })
                .unwrap_or_else(|| ImageRect::new(origin.x, origin.y, 0.0, 0.0)),
        }
    }

    pub(crate) fn normalize_and_clip(&mut self, width: u32, height: u32) {
        match self {
            Self::Measurement { measurement } => {
                for point in [&mut measurement.start, &mut measurement.end] {
                    point.x = point.x.clamp(0.0, width as f32);
                    point.y = point.y.clamp(0.0, height as f32);
                }
                measurement.scale_factor = measurement.scale();
            }
            Self::Rectangle { rect }
            | Self::Ellipse { rect }
            | Self::Pixelate { rect, .. }
            | Self::Blur { rect, .. }
            | Self::Highlight { rect }
            | Self::Spotlight { rect }
            | Self::Redaction { rect }
            | Self::RemoveFill { rect, .. } => *rect = rect.clipped(width, height),
            Self::Magnifier { rect, source, zoom } => {
                if let Some(source) = source {
                    let normalized = rect.normalized();
                    *rect = circle_rect(
                        rect_center(normalized),
                        normalized.width.min(normalized.height) * 0.5,
                    );
                    source.x = source.x.clamp(0.0, width as f32);
                    source.y = source.y.clamp(0.0, height as f32);
                    *zoom = valid_magnifier_zoom(*zoom);
                } else {
                    *rect = rect.clipped(width, height);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn translate(&mut self, delta: Point) {
        match self {
            Self::Measurement { .. } => {}
            Self::Line { start, end } => {
                start.x += delta.x;
                start.y += delta.y;
                end.x += delta.x;
                end.y += delta.y;
            }
            Self::Arrow {
                start, end, bend, ..
            } => {
                start.x += delta.x;
                start.y += delta.y;
                end.x += delta.x;
                end.y += delta.y;
                if let Some(bend) = bend {
                    bend.x += delta.x;
                    bend.y += delta.y;
                }
            }
            Self::Rectangle { rect }
            | Self::Ellipse { rect }
            | Self::Pixelate { rect, .. }
            | Self::Blur { rect, .. }
            | Self::Highlight { rect }
            | Self::Spotlight { rect }
            | Self::Redaction { rect }
            | Self::RemoveFill { rect, .. } => *rect = rect.translated(delta),
            Self::Magnifier { rect, source, .. } => {
                *rect = rect.translated(delta);
                if let Some(source) = source {
                    source.x += delta.x;
                    source.y += delta.y;
                }
            }
            Self::Text { origin, .. } => {
                origin.x += delta.x;
                origin.y += delta.y;
            }
            Self::Counter { center, .. } => {
                center.x += delta.x;
                center.y += delta.y;
            }
            Self::Freehand { points } => {
                for point in points {
                    point.x += delta.x;
                    point.y += delta.y;
                }
            }
            Self::Image { origin, .. } => {
                origin.x += delta.x;
                origin.y += delta.y;
            }
        }
    }

    pub(crate) fn resize_to(&mut self, target: ImageRect) {
        let old = self.bounds();
        match self {
            Self::Measurement { .. } => {}
            Self::Line { start, end } => {
                *start = remap_point(*start, old, target);
                *end = remap_point(*end, old, target);
            }
            Self::Arrow {
                start, end, bend, ..
            } => {
                *start = remap_point(*start, old, target);
                *end = remap_point(*end, old, target);
                if let Some(bend) = bend {
                    *bend = remap_point(*bend, old, target);
                }
            }
            Self::Rectangle { rect }
            | Self::Ellipse { rect }
            | Self::Pixelate { rect, .. }
            | Self::Blur { rect, .. }
            | Self::Highlight { rect }
            | Self::Spotlight { rect }
            | Self::Redaction { rect }
            | Self::RemoveFill { rect, .. } => *rect = target,
            Self::Magnifier { rect, source, .. } => {
                if let Some(source) = source {
                    *source = remap_point(*source, old, target);
                    let scale = (target.width / old.width.max(1.0))
                        .min(target.height / old.height.max(1.0));
                    *rect = circle_rect(
                        remap_point(rect_center(*rect), old, target),
                        rect.width * 0.5 * scale,
                    );
                } else {
                    *rect = target;
                }
            }
            Self::Text {
                origin,
                text,
                font_size,
            } => {
                *origin = Point::new(target.x, target.y);
                let by_height = target.height / 1.25;
                let character_width = text.chars().count() as f32 * 0.62;
                let by_width = if character_width > f32::EPSILON {
                    target.width / character_width
                } else {
                    f32::INFINITY
                };
                *font_size = by_height.min(by_width).max(f32::EPSILON);
            }
            Self::Counter {
                center, font_size, ..
            } => {
                *center = Point::new(
                    target.x + target.width * 0.5,
                    target.y + target.height * 0.5,
                );
                *font_size = (target.width.min(target.height) / 1.3).max(f32::EPSILON);
            }
            Self::Freehand { points } => {
                for point in points {
                    *point = remap_point(*point, old, target);
                }
            }
            Self::Image { origin, size, .. } => {
                *origin = Point::new(target.x, target.y);
                *size = Some(ImageSize {
                    width: target.width.round().max(1.0) as u32,
                    height: target.height.round().max(1.0) as u32,
                });
            }
        }
    }

    pub(crate) fn hit_test(&self, point: Point, tolerance: f32, stroke_width: f32) -> bool {
        match self {
            Self::Measurement { .. } => false,
            Self::Magnifier {
                source: Some(_), ..
            } => {
                self.magnifier_part_at(point, tolerance.max(stroke_width * 0.5))
                    .is_some()
                    || self.magnifier_connector().is_some_and(|[start, end]| {
                        distance_to_segment(point, start, end) <= tolerance.max(stroke_width * 0.5)
                    })
            }
            Self::Line { start, end } => {
                distance_to_segment(point, *start, *end) <= tolerance.max(stroke_width * 0.5)
            }
            Self::Arrow {
                start,
                end,
                bend,
                variant,
            } => {
                let middle = bend.unwrap_or_else(|| midpoint(*start, *end));
                let geometry = arrow_geometry(*start, middle, *end, stroke_width, *variant);
                let tolerance = tolerance.max(geometry.stroke_width * 0.5);
                distance_to_quadratic(point, *start, geometry.control, *end) <= tolerance
                    || geometry.heads().any(|head| match head {
                        ArrowHeadGeometry::Filled(points) => {
                            point_in_triangle(point, points)
                                || [(0, 1), (1, 2), (2, 0)].into_iter().any(|(a, b)| {
                                    distance_to_segment(point, points[a], points[b]) <= tolerance
                                })
                        }
                        ArrowHeadGeometry::Open(arms) => arms.into_iter().any(|arm| {
                            distance_to_quadratic(point, arm.start, arm.control, arm.end)
                                <= tolerance
                        }),
                    })
            }
            Self::Ellipse { rect } => {
                let tolerance = tolerance.max(stroke_width * 0.5);
                let rect = rect.normalized();
                let rx = rect.width * 0.5;
                let ry = rect.height * 0.5;
                if rx <= f32::EPSILON || ry <= f32::EPSILON {
                    return rect.expanded(tolerance).contains(point);
                }
                let cx = rect.x + rx;
                let cy = rect.y + ry;
                let normalized = ((point.x - cx) / rx).powi(2) + ((point.y - cy) / ry).powi(2);
                normalized <= (1.0 + tolerance / rx.min(ry)).powi(2)
            }
            Self::Freehand { points } => {
                let tolerance = tolerance.max(stroke_width * 0.5);
                points
                    .windows(2)
                    .any(|line| distance_to_segment(point, line[0], line[1]) <= tolerance)
                    || points
                        .first()
                        .is_some_and(|first| point.distance_to(*first) <= tolerance)
            }
            _ => self
                .bounds()
                .expanded(tolerance.max(stroke_width * 0.5))
                .contains(point),
        }
    }

    /// Source and displayed lens bounds for a linked magnifier.
    pub fn magnifier_circles(&self) -> Option<[ImageRect; 2]> {
        let Self::Magnifier {
            rect,
            zoom,
            source: Some(source),
        } = self
        else {
            return None;
        };
        let rect = rect.normalized();
        if ![rect.x, rect.y, rect.width, rect.height, source.x, source.y]
            .into_iter()
            .all(f32::is_finite)
        {
            return None;
        }
        let radius = rect.width.min(rect.height) * 0.5;
        (radius > 0.0).then(|| {
            [
                circle_rect(*source, radius / valid_magnifier_zoom(*zoom)),
                circle_rect(rect_center(rect), radius),
            ]
        })
    }

    pub fn magnifier_connector(&self) -> Option<[Point; 2]> {
        let [source, lens] = self.magnifier_circles()?;
        let start = rect_center(source);
        let end = rect_center(lens);
        let distance = start.distance_to(end);
        let (r1, r2) = (source.width * 0.5, lens.width * 0.5);
        if distance <= r1 + r2 {
            return None;
        }
        let (dx, dy) = ((end.x - start.x) / distance, (end.y - start.y) / distance);
        Some([
            Point::new(start.x + dx * r1, start.y + dy * r1),
            Point::new(end.x - dx * r2, end.y - dy * r2),
        ])
    }

    pub fn magnifier_part_at(&self, point: Point, tolerance: f32) -> Option<MagnifierPart> {
        self.magnifier_circles()?
            .into_iter()
            .enumerate()
            .rev()
            .find_map(|(index, circle)| {
                (point.distance_to(rect_center(circle)) <= circle.width * 0.5 + tolerance.max(0.0))
                    .then_some(if index == 0 {
                        MagnifierPart::Source
                    } else {
                        MagnifierPart::Lens
                    })
            })
    }

    pub fn move_magnifier_circle(&mut self, part: MagnifierPart, delta: Point) {
        if let Self::Magnifier {
            rect,
            source: Some(source),
            ..
        } = self
        {
            match part {
                MagnifierPart::Source => {
                    source.x += delta.x;
                    source.y += delta.y;
                }
                MagnifierPart::Lens => *rect = rect.translated(delta),
            }
        }
    }

    pub fn resize_magnifier_circle(&mut self, part: MagnifierPart, radius: f32) {
        if !radius.is_finite() || radius < 1.0 {
            return;
        }
        if let Self::Magnifier {
            rect,
            zoom,
            source: Some(_),
            ..
        } = self
        {
            match part {
                MagnifierPart::Source => *zoom = (rect.width * 0.5 / radius).clamp(1.0, 16.0),
                MagnifierPart::Lens => *rect = circle_rect(rect_center(*rect), radius),
            }
        }
    }

    /// Returns the arrow's start, visible on-curve midpoint, and end handles.
    pub fn arrow_points(&self) -> Option<[Point; 3]> {
        match self {
            Self::Arrow {
                start, end, bend, ..
            } => Some([*start, bend.unwrap_or_else(|| midpoint(*start, *end)), *end]),
            _ => None,
        }
    }

    /// Repositions one of the three visible arrow handles.
    ///
    /// Moving an endpoint carries an explicit bend by half the endpoint delta,
    /// preserving its displacement from the chord midpoint.
    pub fn set_arrow_point(&mut self, index: usize, point: Point) -> bool {
        let Self::Arrow {
            start, end, bend, ..
        } = self
        else {
            return false;
        };
        match index {
            0 => {
                let delta = Point::new(point.x - start.x, point.y - start.y);
                *start = point;
                if let Some(bend) = bend {
                    bend.x += delta.x * 0.5;
                    bend.y += delta.y * 0.5;
                }
            }
            1 => {
                let chord_middle = midpoint(*start, *end);
                *bend = (point.distance_to(chord_middle) > 1.0e-4).then_some(point);
            }
            2 => {
                let delta = Point::new(point.x - end.x, point.y - end.y);
                *end = point;
                if let Some(bend) = bend {
                    bend.x += delta.x * 0.5;
                    bend.y += delta.y * 0.5;
                }
            }
            _ => return false,
        }
        true
    }
}

fn midpoint(start: Point, end: Point) -> Point {
    Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5)
}

fn remap_point(point: Point, source: ImageRect, target: ImageRect) -> Point {
    let x = if source.width > f32::EPSILON {
        target.x + (point.x - source.x) / source.width * target.width
    } else {
        target.x + target.width * 0.5
    };
    let y = if source.height > f32::EPSILON {
        target.y + (point.y - source.y) / source.height * target.height
    } else {
        target.y + target.height * 0.5
    };
    Point::new(x, y)
}

fn points_bounds(points: &[Point]) -> ImageRect {
    let Some(first) = points.first() else {
        return ImageRect::default();
    };
    let (mut left, mut right, mut top, mut bottom) = (first.x, first.x, first.y, first.y);
    for point in &points[1..] {
        left = left.min(point.x);
        right = right.max(point.x);
        top = top.min(point.y);
        bottom = bottom.max(point.y);
    }
    ImageRect::new(left, top, right - left, bottom - top)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnotationStyle {
    pub stroke: Color,
    pub fill: Option<Color>,
    pub stroke_width: f32,
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        Self {
            stroke: Color::RED,
            fill: None,
            stroke_width: 3.0,
        }
    }
}
