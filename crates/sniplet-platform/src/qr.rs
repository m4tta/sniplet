use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::{PlatformError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrCode {
    pub content: String,
    /// Top-left, top-right, bottom-right, bottom-left.
    pub bounds: [QrPoint; 4],
}

/// Finds and decodes every QR code visible in an RGBA image.
pub fn scan_qr_codes(image: &RgbaImage) -> Result<Vec<QrCode>> {
    let gray = image::DynamicImage::ImageRgba8(image.clone()).into_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    prepared
        .detect_grids()
        .into_iter()
        .map(|grid| {
            let (_, content) = grid.decode().map_err(PlatformError::QrDecode)?;
            let bounds = grid.bounds.map(|point| QrPoint {
                x: point.x,
                y: point.y,
            });
            Ok(QrCode { content, bounds })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_image_contains_no_qr_codes() {
        let image = RgbaImage::from_pixel(128, 128, image::Rgba([255, 255, 255, 255]));
        assert!(scan_qr_codes(&image).unwrap().is_empty());
    }

    #[test]
    fn decodes_a_generated_qr_fixture() {
        let gray = qrcode::QrCode::new(b"https://sniplet.local/fixture")
            .unwrap()
            .render::<image::Luma<u8>>()
            .min_dimensions(256, 256)
            .build();
        let rgba = image::DynamicImage::ImageLuma8(gray).into_rgba8();
        let codes = scan_qr_codes(&rgba).unwrap();
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].content, "https://sniplet.local/fixture");
    }
}
