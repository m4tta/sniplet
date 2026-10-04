use image::RgbaImage;

use crate::{Result, SnipletError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StitchOptions {
    /// Small overlaps are ambiguous on flat UI backgrounds.
    pub min_overlap: u32,
    /// Maximum candidate overlap. `None` considers the full shorter frame.
    pub max_overlap: Option<u32>,
    /// Maximum mean absolute RGB channel difference, in 0..=255.
    pub max_mean_error: f32,
}

impl Default for StitchOptions {
    fn default() -> Self {
        Self {
            min_overlap: 24,
            max_overlap: None,
            max_mean_error: 3.0,
        }
    }
}

/// Vertically stitches ordered screenshot frames by matching adjacent overlap.
///
/// Candidate overlaps are scored by mean absolute RGB-channel error. The lowest
/// error wins, with larger overlaps preferred on exact ties.
pub fn stitch_vertical(frames: &[RgbaImage], options: StitchOptions) -> Result<RgbaImage> {
    let Some(first) = frames.first() else {
        return Err(SnipletError::NoStitchFrames);
    };
    let width = first.width();
    let mut output = first.clone();

    for (index, frame) in frames.iter().enumerate().skip(1) {
        if frame.width() != width {
            return Err(SnipletError::StitchWidthMismatch {
                index,
                expected: width,
                actual: frame.width(),
            });
        }
        let maximum = options
            .max_overlap
            .unwrap_or(u32::MAX)
            .min(output.height())
            .min(frame.height());
        let minimum = options.min_overlap.max(1);
        if maximum < minimum {
            return Err(SnipletError::StitchOverlapNotFound {
                index,
                best_error: f32::INFINITY,
            });
        }
        let mut best = None::<(u32, f32)>;
        for overlap in minimum..=maximum {
            let score = overlap_error(&output, frame, overlap);
            if best.is_none_or(|(best_overlap, best_score)| {
                score < best_score - f32::EPSILON
                    || ((score - best_score).abs() <= f32::EPSILON && overlap > best_overlap)
            }) {
                best = Some((overlap, score));
            }
        }
        let Some((overlap, error)) = best else {
            return Err(SnipletError::StitchOverlapNotFound {
                index,
                best_error: f32::INFINITY,
            });
        };
        if error > options.max_mean_error {
            return Err(SnipletError::StitchOverlapNotFound {
                index,
                best_error: error,
            });
        }
        let appended_height = frame.height() - overlap;
        let mut combined = RgbaImage::new(width, output.height() + appended_height);
        image::imageops::replace(&mut combined, &output, 0, 0);
        let remainder =
            image::imageops::crop_imm(frame, 0, overlap, width, appended_height).to_image();
        image::imageops::replace(&mut combined, &remainder, 0, output.height() as i64);
        output = combined;
    }
    Ok(output)
}

fn overlap_error(existing: &RgbaImage, next: &RgbaImage, overlap: u32) -> f32 {
    let existing_y = existing.height() - overlap;
    let mut difference = 0_u64;
    for y in 0..overlap {
        for x in 0..existing.width() {
            let a = existing.get_pixel(x, existing_y + y).0;
            let b = next.get_pixel(x, y).0;
            difference += a[0].abs_diff(b[0]) as u64;
            difference += a[1].abs_diff(b[1]) as u64;
            difference += a[2].abs_diff(b[2]) as u64;
        }
    }
    difference as f32 / (overlap as f32 * existing.width() as f32 * 3.0)
}
