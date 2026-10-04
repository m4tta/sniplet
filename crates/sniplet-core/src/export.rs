use std::path::Path;

use image::{
    ExtendedColorType, ImageEncoder, RgbaImage,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};

use crate::{Document, RenderOptions, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpeg { quality: u8 },
}

pub fn decode_image(encoded: &[u8]) -> Result<RgbaImage> {
    Ok(image::load_from_memory(encoded)?.to_rgba8())
}

impl Document {
    pub fn encode(&self, format: ExportFormat, options: &RenderOptions<'_>) -> Result<Vec<u8>> {
        let rendered = self.render(options)?;
        let mut bytes = Vec::new();
        match format {
            ExportFormat::Png => PngEncoder::new(&mut bytes).write_image(
                rendered.as_raw(),
                rendered.width(),
                rendered.height(),
                ExtendedColorType::Rgba8,
            )?,
            ExportFormat::Jpeg { quality } => {
                if !(1..=100).contains(&quality) {
                    return Err(crate::SnipletError::InvalidJpegQuality);
                }
                let rgb = image::DynamicImage::ImageRgba8(rendered).to_rgb8();
                JpegEncoder::new_with_quality(&mut bytes, quality).encode(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )?;
            }
        }
        Ok(bytes)
    }

    pub fn export(
        &self,
        path: impl AsRef<Path>,
        format: ExportFormat,
        options: &RenderOptions<'_>,
    ) -> Result<()> {
        std::fs::write(path, self.encode(format, options)?)?;
        Ok(())
    }
}
