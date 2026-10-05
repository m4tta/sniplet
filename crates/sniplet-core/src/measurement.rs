use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasurementAxis {
    Horizontal,
    Vertical,
}

/// A pixel span. The end point lies just beyond the last included pixel.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub start: Point,
    pub end: Point,
    pub pixels_per_unit: f32,
    pub scale_factor: f32,
}

impl Measurement {
    pub fn label(self) -> String {
        let units = if self.pixels_per_unit.is_finite() {
            self.pixels_per_unit.clamp(1.0, 8.0)
        } else {
            1.0
        };
        format!("{:.0}px", self.start.distance_to(self.end) / units)
    }

    pub(crate) fn scale(self) -> f32 {
        if self.scale_factor.is_finite() {
            self.scale_factor.clamp(1.0, 8.0)
        } else {
            1.0
        }
    }
}

/// Finds the nearest color edges on the row or column under the pointer.
/// Shift includes the adjoining border bands. A larger tolerance ignores weaker edges.
pub fn measure_at(
    image: &RgbaImage,
    point: Point,
    axis: MeasurementAxis,
    tolerance: u8,
    outer: bool,
) -> Option<Measurement> {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < 0.0
        || point.y < 0.0
        || point.x >= image.width() as f32
        || point.y >= image.height() as f32
    {
        return None;
    }
    let (x, y) = (point.x.floor() as u32, point.y.floor() as u32);
    if image.get_pixel(x, y)[3] == 0 {
        return None;
    }
    let (seed, length) = match axis {
        MeasurementAxis::Horizontal => (x, image.width()),
        MeasurementAxis::Vertical => (y, image.height()),
    };
    let pixel = |index| match axis {
        MeasurementAxis::Horizontal => *image.get_pixel(index, y),
        MeasurementAxis::Vertical => *image.get_pixel(x, index),
    };
    let edge = |a: Rgba<u8>, b: Rgba<u8>| {
        // Compare visible colors, including alpha, to ignore RGB noise in transparent pixels.
        (0..4).any(|channel| {
            let visible = |p: Rgba<u8>| {
                if channel == 3 {
                    p[3]
                } else {
                    (u16::from(p[channel]) * u16::from(p[3]) / 255) as u8
                }
            };
            visible(a).abs_diff(visible(b)) > tolerance
        })
    };
    let left_edge = |mut index: u32| {
        while index > 0 && !edge(pixel(index), pixel(index - 1)) {
            index -= 1;
        }
        index
    };
    let right_edge = |mut index: u32| {
        while index + 1 < length && !edge(pixel(index), pixel(index + 1)) {
            index += 1;
        }
        index + 1
    };
    let mut left = left_edge(seed);
    let mut right = right_edge(seed);
    if outer {
        // Include a thin border, but do not extend through a broad background or gap.
        if left > 0 {
            let border = left_edge(left - 1);
            if left - border <= 16 {
                left = border;
            }
        }
        if right < length {
            let border = right_edge(right);
            if border - right <= 16 {
                right = border;
            }
        }
    }
    let (start, end) = match axis {
        MeasurementAxis::Horizontal => (
            Point::new(left as f32, y as f32 + 0.5),
            Point::new(right as f32, y as f32 + 0.5),
        ),
        MeasurementAxis::Vertical => (
            Point::new(x as f32 + 0.5, left as f32),
            Point::new(x as f32 + 0.5, right as f32),
        ),
    };
    Some(Measurement {
        start,
        end,
        pixels_per_unit: 1.0,
        scale_factor: 1.0,
    })
}
