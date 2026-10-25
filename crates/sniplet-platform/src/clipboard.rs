use std::borrow::Cow;
#[cfg(target_os = "linux")]
use std::sync::Mutex;

use image::RgbaImage;

use crate::{PlatformError, Result};

// Linux serves clipboard data from the owning process. Keep the backend alive
// after a copy worker drops its handle, including when there is no clipboard
// manager. Initialize lazily so a failed connection can be retried.
#[cfg(target_os = "linux")]
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// An operation handle to the operating system clipboard.
///
/// On Linux the shared owner remains alive for the process lifetime; dropping
/// this handle does not discard copied images or text.
pub struct Clipboard {
    #[cfg(target_os = "linux")]
    inner: &'static Mutex<Option<arboard::Clipboard>>,
    #[cfg(not(target_os = "linux"))]
    inner: arboard::Clipboard,
}

impl Clipboard {
    pub fn new() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let mut owner = CLIPBOARD.lock().unwrap_or_else(|error| error.into_inner());
            if owner.is_none() {
                *owner =
                    Some(arboard::Clipboard::new().map_err(PlatformError::ClipboardUnavailable)?);
            }
            Ok(Self { inner: &CLIPBOARD })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let inner = arboard::Clipboard::new().map_err(PlatformError::ClipboardUnavailable)?;
            Ok(Self { inner })
        }
    }

    fn with_inner<T>(&mut self, operation: impl FnOnce(&mut arboard::Clipboard) -> T) -> T {
        #[cfg(target_os = "linux")]
        {
            let mut owner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
            operation(owner.as_mut().expect("clipboard initialized by new"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            operation(&mut self.inner)
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> Result<()> {
        self.with_inner(|clipboard| clipboard.set_text(text.into()))
            .map_err(PlatformError::Clipboard)
    }

    pub fn text(&mut self) -> Result<String> {
        self.with_inner(|clipboard| clipboard.get_text())
            .map_err(PlatformError::Clipboard)
    }

    pub fn set_image(&mut self, image: &RgbaImage) -> Result<()> {
        self.with_inner(|clipboard| {
            clipboard.set_image(arboard::ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: Cow::Borrowed(image.as_raw()),
            })
        })
        .map_err(PlatformError::Clipboard)
    }

    pub fn image(&mut self) -> Result<RgbaImage> {
        let image = self
            .with_inner(|clipboard| clipboard.get_image())
            .map_err(PlatformError::Clipboard)?;
        let width = image.width as u32;
        let height = image.height as u32;
        let bytes = image.into_owned_bytes().into_owned();
        let expected = width as usize * height as usize * 4;
        let actual = bytes.len();
        RgbaImage::from_raw(width, height, bytes)
            .ok_or(PlatformError::InvalidRgbaBuffer { expected, actual })
    }
}
