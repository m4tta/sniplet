//! Times `stitch_vertical` on screen-sized scroll captures and hashes its results over a
//! fixed corpus of generated cases.
//!
//! Run it in release on two revisions: equal hashes mean identical results.
//! `cargo run -p sniplet-core --release --example stitch_bench -- [runs] [--cases]`
//! `--cases` also prints one hash per corpus case, to find the case that changed.
use std::time::{Duration, Instant};

use image::{Rgba, RgbaImage, imageops};
use sniplet_core::{Result, StitchOptions, stitch_vertical};

const CORPUS_CASES: u64 = 3000;

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let print_cases = arguments.iter().any(|argument| argument == "--cases");
    let runs = arguments
        .iter()
        .find_map(|argument| argument.parse().ok())
        .unwrap_or(3);

    let mut hash = Hash::default();
    let (mut ok, mut errors) = (0, 0);
    let start = Instant::now();
    for seed in 0..CORPUS_CASES {
        let (frames, options) = corpus_case(seed);
        let result = stitch_vertical(&frames, options);
        match result {
            Ok(_) => ok += 1,
            Err(_) => errors += 1,
        }
        hash.result(&result);
        if print_cases {
            let mut case = Hash::default();
            case.result(&result);
            println!("case {seed} {:016x}", case.0);
        }
    }
    println!(
        "corpus: {CORPUS_CASES} cases ({ok} ok, {errors} errors) in {} ms, hash {:016x}",
        start.elapsed().as_millis(),
        hash.0
    );

    for (name, frames, options) in scenarios() {
        let mut times = Vec::new();
        let mut hash = Hash::default();
        let mut outcome = String::new();
        for _ in 0..runs {
            let start = Instant::now();
            let result = std::hint::black_box(stitch_vertical(&frames, options));
            times.push(start.elapsed());
            hash = Hash::default();
            hash.result(&result);
            outcome = match result {
                Ok(image) => format!("{}x{}", image.width(), image.height()),
                Err(error) => error.to_string(),
            };
        }
        times.sort();
        let (width, height) = frames[0].dimensions();
        println!(
            "{name}: {} frames of {width}x{height} -> {outcome}, median {:.1} ms over {runs} runs, hash {:016x}",
            frames.len(),
            millis(times[times.len() / 2]),
            hash.0
        );
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// FNV-1a over everything a caller can observe in a stitch result.
#[derive(Default)]
struct Hash(u64);

impl Hash {
    fn bytes(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf2_9ce4_8422_2325;
        }
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
    }

    fn result(&mut self, result: &Result<RgbaImage>) {
        match result {
            Ok(image) => {
                let (width, height) = image.dimensions();
                self.bytes(&width.to_le_bytes());
                self.bytes(&height.to_le_bytes());
                // Only the pixels: a buffer may carry spare bytes past the last row.
                self.bytes(&image.as_raw()[..width as usize * height as usize * 4]);
            }
            Err(error) => self.bytes(format!("{error:?}").as_bytes()),
        }
    }
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A value in `low..=high`.
    fn range(&mut self, low: u32, high: u32) -> u32 {
        low + (self.next() % (u64::from(high - low) + 1)) as u32
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    fn color(&mut self) -> Rgba<u8> {
        Rgba([self.byte(), self.byte(), self.byte(), 255])
    }
}

/// A tall page of random bands: noise, flat color, stripes, sparse "text", gradients.
fn band_page(rng: &mut Rng, width: u32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    let mut top = 0;
    while top < height {
        let length = rng.range(1, 60).min(height - top);
        let kind = rng.range(0, 4);
        let (a, b) = (rng.color(), rng.color());
        let period = rng.range(1, 8);
        let (gx, gy) = (rng.range(0, 9), rng.range(0, 9));
        for y in top..top + length {
            for x in 0..width {
                let pixel = match kind {
                    0 => Rgba([rng.byte(), rng.byte(), rng.byte(), rng.byte()]),
                    1 => a,
                    2 if (y / period).is_multiple_of(2) => a,
                    2 => b,
                    3 if y % 12 < 8 && rng.range(0, 5) == 0 => b,
                    3 => Rgba([250, 250, 250, 255]),
                    _ => Rgba([(x * gx + y * gy) as u8, (y * 3) as u8, (x * 7) as u8, 255]),
                };
                image.put_pixel(x, y, pixel);
            }
        }
        top += length;
    }
    image
}

/// Any valid input: real overlaps, noisy frames, unrelated frames, odd options and sizes.
fn corpus_case(seed: u64) -> (Vec<RgbaImage>, StitchOptions) {
    let mut rng = Rng::new(seed);
    let width = if rng.range(0, 9) == 0 {
        rng.range(65, 200)
    } else {
        rng.range(1, 64)
    };
    let frame_height = rng.range(1, 160);
    let count = rng.range(1, 10);
    let mut starts = vec![0_u32];
    for _ in 1..count {
        let step = rng.range(0, frame_height);
        starts.push(starts.last().unwrap() + step);
    }
    let page = band_page(&mut rng, width, starts.last().unwrap() + frame_height + 8);
    let noise = [0, 0, 1, 2, 3][rng.range(0, 4) as usize];
    let jitter = rng.range(0, 3) == 0;
    let mut frames = Vec::new();
    for &start in &starts {
        let height = if jitter {
            rng.range(0, frame_height + 8)
        } else {
            frame_height
        };
        let mut frame = imageops::crop_imm(&page, 0, start, width, height).to_image();
        if rng.range(0, 19) == 0 {
            frame = RgbaImage::from_fn(width, height, |_, _| rng.color());
        }
        for pixel in frame.pixels_mut() {
            for channel in &mut pixel.0[..3] {
                let offset = rng.range(0, 2 * noise) as i32 - noise as i32;
                *channel = (i32::from(*channel) + offset).clamp(0, 255) as u8;
            }
        }
        if rng.range(0, 9) == 0 {
            let mut raw = frame.into_raw();
            raw.extend_from_slice(&[9; 64]);
            frame = RgbaImage::from_raw(width, height, raw).unwrap();
        }
        frames.push(frame);
    }
    let min_overlap = [0, 1, 2, 4, 8, 24, rng.range(0, frame_height)][rng.range(0, 6) as usize];
    let max_overlap = match rng.range(0, 3) {
        0 | 1 => None,
        2 => Some(frame_height),
        _ => Some(rng.range(0, frame_height + 10)),
    };
    let max_mean_error = [0.0, 0.5, 1.0, 3.0, 10.0, 255.0][rng.range(0, 5) as usize];
    let options = StitchOptions {
        min_overlap,
        max_overlap,
        max_mean_error,
    };
    (frames, options)
}

/// A wide page that looks like a web document: white space, text lines, rules and images.
fn document_page(rng: &mut Rng, width: u32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
    let mut top = 0;
    while top < height {
        let block = rng.range(0, 9);
        let length = match block {
            0 => rng.range(120, 360),
            1 => 2,
            _ => rng.range(40, 200),
        }
        .min(height - top);
        let margin = rng.range(40, 200);
        let tint = rng.color();
        for y in top..top + length {
            for x in margin..width - margin {
                let pixel = match block {
                    0 => Rgba([
                        ((x / 3 + y) as u8).wrapping_add(rng.byte() % 16),
                        (x as u8).wrapping_mul(3) ^ (y as u8),
                        tint.0[2].wrapping_add(rng.byte() % 8),
                        255,
                    ]),
                    1 => Rgba([200, 200, 200, 255]),
                    _ if (y - top) % 24 < 16 && rng.range(0, 3) == 0 => Rgba([30, 30, 30, 255]),
                    _ => continue,
                };
                image.put_pixel(x, y, pixel);
            }
        }
        top += length + rng.range(0, 40);
    }
    image
}

fn screen_frames(seed: u64, steps: &[u32]) -> Vec<RgbaImage> {
    let (width, height) = (1600, 900);
    let mut rng = Rng::new(seed);
    let page = document_page(&mut rng, width, height + steps.iter().sum::<u32>());
    let mut top = 0;
    let mut frames = vec![imageops::crop_imm(&page, 0, 0, width, height).to_image()];
    for &step in steps {
        top += step;
        frames.push(imageops::crop_imm(&page, 0, top, width, height).to_image());
    }
    frames
}

/// Lightens a 300x40 box at the center of every other frame, like a hover effect under the
/// cursor that automatic scrolling parks there.
fn with_hover(mut frames: Vec<RgbaImage>) -> Vec<RgbaImage> {
    for frame in frames.iter_mut().skip(1).step_by(2) {
        for y in 430..470 {
            for x in 650..950 {
                for channel in &mut frame.get_pixel_mut(x, y).0[..3] {
                    *channel = channel.saturating_add(20);
                }
            }
        }
    }
    frames
}

/// Adds -1..=1 to every color channel, like a lossy capture path.
fn with_noise(mut frames: Vec<RgbaImage>) -> Vec<RgbaImage> {
    let mut rng = Rng::new(99);
    for frame in &mut frames {
        for pixel in frame.pixels_mut() {
            for channel in &mut pixel.0[..3] {
                *channel = (i32::from(*channel) + rng.range(0, 2) as i32 - 1).clamp(0, 255) as u8;
            }
        }
    }
    frames
}

fn scenarios() -> Vec<(&'static str, Vec<RgbaImage>, StitchOptions)> {
    let mut rng = Rng::new(77);
    let varied = (0..9).map(|_| rng.range(150, 700)).collect::<Vec<_>>();
    vec![
        (
            "steady 400px scroll",
            screen_frames(1, &[400; 9]),
            StitchOptions::default(),
        ),
        (
            "varied 150-700px scroll",
            screen_frames(2, &varied),
            StitchOptions::default(),
        ),
        (
            "small 100px scroll",
            screen_frames(3, &[100; 9]),
            StitchOptions::default(),
        ),
        (
            "steady 400px, hover box",
            with_hover(screen_frames(5, &[400; 9])),
            StitchOptions::default(),
        ),
        (
            "varied 150-700px, hover box",
            with_hover(screen_frames(6, &varied)),
            StitchOptions::default(),
        ),
        (
            "steady 400px, +-1 noise",
            with_noise(screen_frames(7, &[400; 9])),
            StitchOptions::default(),
        ),
        (
            "copy only, 24px overlap",
            screen_frames(4, &[876; 39]),
            StitchOptions {
                max_overlap: Some(24),
                ..StitchOptions::default()
            },
        ),
    ]
}
