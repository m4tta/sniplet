use image::{Rgba, RgbaImage};

/// Generates an original Sniplet demo screenshot without bundled third-party assets.
pub fn demo_image(width: u32, height: u32) -> RgbaImage {
    let width = width.max(64);
    let height = height.max(64);
    RgbaImage::from_fn(width, height, |x, y| {
        let fx = x as f32 / (width - 1) as f32;
        let fy = y as f32 / (height - 1) as f32;
        let checker = if (x / 32 + y / 32) % 2 == 0 { 4 } else { 0 };
        Rgba([
            (222.0 + 20.0 * fx) as u8 + checker,
            (232.0 + 16.0 * fy) as u8 + checker,
            (239.0 + 10.0 * (1.0 - fx)) as u8 + checker,
            255,
        ])
    })
}
