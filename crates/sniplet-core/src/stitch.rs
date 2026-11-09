use std::ops::RangeInclusive;

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
    stitch_vertical_refs(&frames.iter().collect::<Vec<_>>(), options)
}

/// Stitch shared frames without making a second copy of each input image.
pub fn stitch_vertical_refs(frames: &[&RgbaImage], options: StitchOptions) -> Result<RgbaImage> {
    let Some(first) = frames.first() else {
        return Err(SnipletError::NoStitchFrames);
    };
    let width = first.width();
    let row_bytes = width as usize * 4;
    let mut height = first.height();
    // One buffer grows by the new rows of each frame, so earlier rows are never copied again.
    let mut output = Vec::with_capacity(frames.iter().map(|frame| pixels(frame).len()).sum());
    output.extend_from_slice(pixels(first));

    for (index, frame) in frames.iter().copied().enumerate().skip(1) {
        if frame.width() != width {
            return Err(SnipletError::StitchWidthMismatch {
                index,
                expected: width,
                actual: frame.width(),
            });
        }
        let not_found = |best_error| SnipletError::StitchOverlapNotFound { index, best_error };
        let maximum = options
            .max_overlap
            .unwrap_or(u32::MAX)
            .min(height)
            .min(frame.height());
        let minimum = options.min_overlap.max(1);
        // Zero-width frames have no pixels, so no overlap can be measured.
        if maximum < minimum || width == 0 {
            return Err(not_found(f32::INFINITY));
        }
        let next = pixels(frame);
        let search = Search {
            existing: &output,
            next,
            row_bytes,
        };
        let (overlap, error) = search.best(minimum..=maximum, options.max_mean_error);
        // False for a NaN threshold, so that accepts nothing.
        let accepted = error <= options.max_mean_error;
        if !accepted {
            return Err(not_found(error));
        }
        output.extend_from_slice(&next[overlap as usize * row_bytes..]);
        height += frame.height() - overlap;
    }
    output.shrink_to_fit();
    Ok(RgbaImage::from_raw(width, height, output).expect("stitched buffer matches its size"))
}

/// The image's pixel bytes, without any spare bytes a caller left past the last row.
fn pixels(image: &RgbaImage) -> &[u8] {
    &image.as_raw()[..image.width() as usize * image.height() as usize * 4]
}

/// A candidate replaces the best only when it scores lower or within `f32::EPSILON`, so one
/// that is provably more than this above the best can stop early. Scores are at most 255,
/// where f32 steps are about 1.5e-5, so rounding never closes this gap.
const MARGIN: f32 = 1.0e-3;

/// Compares the bottom of the stitched output with the top of the next frame.
struct Search<'a> {
    existing: &'a [u8],
    next: &'a [u8],
    row_bytes: usize,
}

impl Search<'_> {
    /// The overlap and error that a full ascending scan of `overlaps` picks: the lowest
    /// error wins, and a later (larger) overlap wins a tie within `f32::EPSILON`.
    ///
    /// Each scan bets on how low the best error is. A lost bet costs one quick scan and never
    /// changes the answer. Screen captures are usually pixel-exact, so bet on 0 first, then on
    /// the largest error the caller accepts, and last scan with no bet.
    fn best(&self, overlaps: RangeInclusive<u32>, max_error: f32) -> (u32, f32) {
        [0.0, max_error, f32::INFINITY]
            .into_iter()
            .find_map(|floor| self.scan(overlaps.clone(), floor))
            .expect("a scan with no floor scores the first candidate")
    }

    /// Scans in ascending order and keeps the winner, but stops scoring any candidate that
    /// provably cannot change the result.
    ///
    /// Until the first candidate that scores at most `floor + MARGIN`, every candidate is
    /// skipped. If that first one scores at most `floor`, it beats each skipped one by more
    /// than `MARGIN`, so a full scan would also hold it as the best at that point, and the
    /// result is the same. Otherwise (no such candidate, or it scores between the two), this
    /// returns `None`. With an infinite `floor`, nothing is skipped and the result is `Some`.
    fn scan(&self, overlaps: RangeInclusive<u32>, floor: f32) -> Option<(u32, f32)> {
        let mut best = None::<(u32, f32)>;
        for overlap in overlaps {
            let limit = best.map_or(floor, |(_, score)| score) + MARGIN;
            let Some(score) = self.error(overlap, limit) else {
                continue;
            };
            let wins = match best {
                None if score > floor => return None,
                None => true,
                Some((_, best_score)) => {
                    score < best_score - f32::EPSILON || (score - best_score).abs() <= f32::EPSILON
                }
            };
            if wins {
                best = Some((overlap, score));
            }
        }
        best
    }

    /// Mean absolute RGB error between the last `overlap` rows of the output and the first
    /// `overlap` rows of the next frame, or `None` once it is sure to be above `limit`.
    fn error(&self, overlap: u32, limit: f32) -> Option<f32> {
        let bytes = overlap as usize * self.row_bytes;
        let existing = &self.existing[self.existing.len() - bytes..];
        let width = (self.row_bytes / 4) as f32;
        let denominator = overlap as f32 * width * 3.0;
        let mut difference = 0_u64;
        for (a, b) in existing
            .chunks_exact(self.row_bytes)
            .zip(self.next[..bytes].chunks_exact(self.row_bytes))
        {
            difference += u64::from(row_difference(a, b));
            // The total only grows, so a partial mean above the limit is final.
            if difference as f32 / denominator > limit {
                return None;
            }
        }
        Some(difference as f32 / denominator)
    }
}

/// Sum of absolute RGB differences between two pixel rows. Alpha is ignored.
fn row_difference(a: &[u8], b: &[u8]) -> u32 {
    let (a, b) = (a.as_chunks::<4>().0, b.as_chunks::<4>().0);
    a.iter()
        .zip(b)
        .map(|([ar, ag, ab, _], [br, bg, bb, _])| {
            u32::from(ar.abs_diff(*br)) + u32::from(ag.abs_diff(*bg)) + u32::from(ab.abs_diff(*bb))
        })
        .sum()
}
