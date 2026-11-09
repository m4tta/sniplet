use std::{
    io::{BufRead, Cursor, Seek},
    path::Path,
};

use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageReader, Limits, RgbaImage,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};

use crate::{Document, RenderOptions, Result, SnipletError};

/// Largest image Sniplet builds from file data: 2^27 pixels, 512 MiB as RGBA8.
///
/// This is the `image` crate's default decode budget for an RGBA8 PNG, so every
/// screenshot project that opened before still opens. It holds 4 full 8K screens.
pub(crate) const MAX_IMAGE_PIXELS: u64 = 1 << 27;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpeg { quality: u8 },
}

/// Rejects a `width` x `height` image over [`MAX_IMAGE_PIXELS`].
pub(crate) fn check_image_size(width: u32, height: u32) -> Result<()> {
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(SnipletError::ImageTooLarge { width, height });
    }
    Ok(())
}

/// Reads only the header of an encoded image to get its size.
pub(crate) fn encoded_dimensions(encoded: &[u8]) -> Result<(u32, u32)> {
    Ok(ImageReader::new(Cursor::new(encoded))
        .with_guessed_format()?
        .into_dimensions()?)
}

/// Decodes PNG, JPEG, or WebP bytes. The header is checked against
/// [`MAX_IMAGE_PIXELS`] first, so the RGBA8 result also stays in that budget.
pub fn decode_image(encoded: &[u8]) -> Result<RgbaImage> {
    decode(ImageReader::new(Cursor::new(encoded)))
}

/// Opens an image file like [`decode_image`]. The file is streamed, never read whole
/// before the header check, so a huge or endless file fails early.
pub fn open_image(path: impl AsRef<Path>) -> Result<RgbaImage> {
    decode(ImageReader::open(path)?)
}

/// Checks the header, then decodes with the same decoder.
fn decode<R: BufRead + Seek>(reader: ImageReader<R>) -> Result<RgbaImage> {
    let decoder = reader.with_guessed_format()?.into_decoder()?;
    let (width, height) = decoder.dimensions();
    check_image_size(width, height)?;
    // The allocation check that `ImageReader::decode` makes.
    Limits::default().reserve(decoder.total_bytes())?;
    Ok(DynamicImage::from_decoder(decoder)?.to_rgba8())
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
