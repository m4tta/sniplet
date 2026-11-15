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
    let Some((first, rest)) = frames.split_first() else {
        return Err(SnipletError::NoStitchFrames);
    };
    let mut stitcher = Stitcher::new(first, options);
    for frame in rest {
        stitcher.push(frame)?;
    }
    Ok(stitcher.into_image())
}

/// Stitches frames one at a time, as a scrolling capture takes them. Each frame continues the
/// page below the frames before it.
pub struct Stitcher {
    options: StitchOptions,
    width: u32,
    /// The stitched rows, without a sticky footer. One buffer grows by the new rows of each
    /// frame, so earlier rows are not copied per frame.
    output: Vec<u8>,
    height: u32,
    /// Frames seen so far, the first included.
    frames: usize,
    sticky: Option<Sticky>,
}

/// What sticky-band stitching remembers between frames.
struct Sticky {
    /// The last frame that stitched, to find the rows that stayed in place.
    previous: Vec<u8>,
    /// The height of the footer that stays in place, known from the second frame on.
    footer: Option<u32>,
}

impl Stitcher {
    /// Start a page with its top frame. Frames match at their full height, as
    /// `stitch_vertical` does.
    pub fn new(first: &RgbaImage, options: StitchOptions) -> Self {
        Self {
            options,
            width: first.width(),
            output: pixels(first).to_vec(),
            height: first.height(),
            frames: 1,
            sticky: None,
        }
    }

    /// `new`, but rows at the top and bottom that stay the same between frames, such as a
    /// fixed header or footer, are left out of matching and of the appended rows. The page
    /// keeps the first frame's header and the last frame's footer.
    pub fn with_sticky_bands(first: &RgbaImage, options: StitchOptions) -> Self {
        Self {
            sticky: Some(Sticky {
                previous: pixels(first).to_vec(),
                footer: None,
            }),
            ..Self::new(first, options)
        }
    }

    /// Add the next frame and return how many rows it added. Zero means the frame showed
    /// nothing new, as at the end of the page. On an error the page stays as it was.
    pub fn push(&mut self, frame: &RgbaImage) -> Result<u32> {
        let index = self.frames;
        self.frames += 1;
        let width = self.width;
        if frame.width() != width {
            return Err(SnipletError::StitchWidthMismatch {
                index,
                expected: width,
                actual: frame.width(),
            });
        }
        let row_bytes = width as usize * 4;
        let frame_pixels = pixels(frame);
        let (top, footer, first_footer) = match &self.sticky {
            None => (0, 0, false),
            Some(sticky) => {
                let (top, bottom) = static_bands(&sticky.previous, frame_pixels, row_bytes);
                if top == frame.height() {
                    return Ok(0);
                }
                match sticky.footer {
                    Some(footer) => (top, footer, false),
                    // Kept only when this frame adds rows: a page that redraws in place also
                    // has bottom rows that stay the same, and they are not a footer.
                    None => (top, bottom.min(self.height), true),
                }
            }
        };
        // The page to match against, without the first frame's footer.
        let page = if first_footer {
            self.height - footer
        } else {
            self.height
        };
        let end = frame.height().saturating_sub(footer);
        if top >= end {
            // Only the bands changed.
            self.remember(frame_pixels);
            return Ok(0);
        }
        let next = &frame_pixels[top as usize * row_bytes..end as usize * row_bytes];
        let rows = end - top;
        let not_found = |best_error| SnipletError::StitchOverlapNotFound { index, best_error };
        let mut maximum = self
            .options
            .max_overlap
            .unwrap_or(u32::MAX)
            .min(page)
            .min(rows);
        let mut minimum = self.options.min_overlap.max(1);
        // Bands can leave fewer rows than the minimum overlap, as a blinking caret on a page
        // that did not move does. Those rows can only be in the same place, so test only that.
        if self.sticky.is_some() && rows < minimum {
            (minimum, maximum) = (rows, rows.min(page));
        }
        // Zero-width frames have no pixels, so no overlap can be measured.
        if maximum < minimum || width == 0 {
            return Err(not_found(f32::INFINITY));
        }
        let search = Search {
            existing: &self.output[..page as usize * row_bytes],
            next,
            row_bytes,
        };
        let (overlap, error) = search.best(minimum..=maximum, self.options.max_mean_error);
        // False for a NaN threshold, so that accepts nothing.
        let accepted = error <= self.options.max_mean_error;
        if !accepted {
            return Err(not_found(error));
        }
        let added = rows - overlap;
        if added > 0 {
            if first_footer && let Some(sticky) = &mut self.sticky {
                // The first frame's footer leaves the page; `into_image` puts the last frame's
                // footer back below it.
                self.output.truncate(page as usize * row_bytes);
                self.height = page;
                sticky.footer = Some(footer);
            }
            self.output
                .extend_from_slice(&next[overlap as usize * row_bytes..]);
            self.height += added;
        }
        self.remember(frame_pixels);
        Ok(added)
    }

    fn remember(&mut self, frame_pixels: &[u8]) {
        if let Some(sticky) = &mut self.sticky {
            sticky.previous.clear();
            sticky.previous.extend_from_slice(frame_pixels);
        }
    }

    /// The stitched page.
    pub fn into_image(self) -> RgbaImage {
        let mut output = self.output;
        let mut height = self.height;
        if let Some(Sticky {
            previous,
            footer: Some(footer),
        }) = &self.sticky
        {
            let row_bytes = (self.width as usize * 4).max(1);
            // A shorter last frame cannot give more rows than it has.
            let bytes = (*footer as usize * row_bytes).min(previous.len() / row_bytes * row_bytes);
            output.extend_from_slice(&previous[previous.len() - bytes..]);
            height += (bytes / row_bytes) as u32;
        }
        output.shrink_to_fit();
        RgbaImage::from_raw(self.width, height, output).expect("stitched buffer matches its size")
    }
}

/// How many rows at the top, and at the bottom, are the same in two frames of one size.
/// Frames of different sizes share none.
fn static_bands(a: &[u8], b: &[u8], row_bytes: usize) -> (u32, u32) {
    if a.len() != b.len() || row_bytes == 0 {
        return (0, 0);
    }
    let rows = || a.chunks_exact(row_bytes).zip(b.chunks_exact(row_bytes));
    let top = rows().take_while(|(a, b)| a == b).count();
    let bottom = rows().rev().take_while(|(a, b)| a == b).count();
    (top as u32, bottom as u32)
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
