use std::borrow::Cow;

use image::RgbaImage;

use crate::{PlatformError, Result};

/// A short-lived handle to the operating system clipboard.
pub struct Clipboard {
    inner: arboard::Clipboard,
}

impl Clipboard {
    pub fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new().map_err(PlatformError::ClipboardUnavailable)?;
        Ok(Self { inner })
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> Result<()> {
        self.inner
            .set_text(text.into())
            .map_err(PlatformError::Clipboard)
    }

    pub fn text(&mut self) -> Result<String> {
        self.inner.get_text().map_err(PlatformError::Clipboard)
    }

    pub fn set_image(&mut self, image: &RgbaImage) -> Result<()> {
        self.inner
            .set_image(arboard::ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: Cow::Borrowed(image.as_raw()),
            })
            .map_err(PlatformError::Clipboard)
    }

    pub fn image(&mut self) -> Result<RgbaImage> {
        let image = self.inner.get_image().map_err(PlatformError::Clipboard)?;
        let width = image.width as u32;
        let height = image.height as u32;
        let bytes = image.into_owned_bytes().into_owned();
        let expected = width as usize * height as usize * 4;
        let actual = bytes.len();
        RgbaImage::from_raw(width, height, bytes)
            .ok_or(PlatformError::InvalidRgbaBuffer { expected, actual })
    }
}
