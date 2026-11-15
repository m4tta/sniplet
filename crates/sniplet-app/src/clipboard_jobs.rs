use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use sniplet_core::Document;
use sniplet_platform::Clipboard;

use crate::editor::Export;

pub(crate) enum Source {
    Original(Document),
    Rendered(Export),
    Text(String),
}

/// Text found in an image. `copied` is `Ok(false)` when a newer copy owns the clipboard.
pub(crate) struct Recognized {
    pub text: String,
    pub copied: anyhow::Result<bool>,
}

#[derive(Default)]
struct CopyOrder {
    next: AtomicU64,
    writer: Mutex<u64>,
}

impl CopyOrder {
    fn request(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn write<T>(
        &self,
        request: u64,
        write: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<Option<T>> {
        let mut last_written = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if request < *last_written {
            return Ok(None);
        }
        let value = write()?;
        *last_written = request;
        Ok(Some(value))
    }
}

static ORDER: CopyOrder = CopyOrder {
    next: AtomicU64::new(0),
    writer: Mutex::new(0),
};

/// The work of one copy, for a worker thread. Its order slot is taken now, so an old render
/// cannot replace a newer copy. `Ok(None)` means a newer copy already owns the clipboard.
pub(crate) fn copy(source: Source) -> impl FnOnce() -> anyhow::Result<Option<()>> + Send + 'static {
    let request = ORDER.request();
    move || {
        // Rendering is outside the write lock. Clipboard writes alone are ordered.
        let rendered = match &source {
            Source::Rendered(export) => Some(export.image()?),
            _ => None,
        };
        ORDER.write(request, || {
            let mut clipboard = Clipboard::new()?;
            match source {
                Source::Original(document) => clipboard.set_image(document.original())?,
                Source::Rendered(_) => clipboard.set_image(rendered.as_ref().unwrap())?,
                Source::Text(text) => clipboard.set_text(text)?,
            }
            Ok(())
        })
    }
}

/// Read QR contents, or OCR text when there is no QR code, then copy the result.
/// The order slot is taken now, so a slow recognition cannot replace a newer copy.
/// The text comes back even when the clipboard write fails, so the editor can show it.
pub(crate) fn recognize(
    export: Export,
    qr_only: bool,
) -> impl FnOnce() -> anyhow::Result<Recognized> + Send + 'static {
    let request = ORDER.request();
    move || {
        let image = export.image()?;
        // OCR runs beside the QR scan. A QR code wins, and its answer does not wait for OCR.
        let ocr = (!qr_only).then(|| {
            let image = Arc::clone(&image);
            std::thread::spawn(move || {
                sniplet_platform::recognize_text(&image, &Default::default())
            })
        });
        let codes = sniplet_platform::scan_qr_codes(&image)?;
        let text = if !codes.is_empty() {
            codes
                .into_iter()
                .map(|code| code.content)
                .collect::<Vec<_>>()
                .join("\n")
        } else if let Some(ocr) = ocr {
            ocr.join()
                .map_err(|_| anyhow::anyhow!("Text recognition stopped unexpectedly"))??
        } else {
            anyhow::bail!("No QR code found in the selected image");
        };
        if text.trim().is_empty() {
            anyhow::bail!("No text found in the selected image");
        }
        let copied = ORDER
            .write(request, || Ok(Clipboard::new()?.set_text(text.clone())?))
            .map(|written| written.is_some());
        Ok(Recognized { text, copied })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_old_copy_cannot_overwrite_the_latest_request() {
        let order = CopyOrder::default();
        let old = order.request();
        let latest = order.request();
        let mut value = "initial";
        order
            .write(latest, || {
                value = "latest";
                Ok(())
            })
            .unwrap();
        assert!(
            order
                .write(old, || {
                    value = "old";
                    Ok(())
                })
                .unwrap()
                .is_none()
        );
        assert_eq!(value, "latest");
    }

    #[test]
    fn failed_recognition_does_not_discard_a_waiting_capture_copy() {
        let order = CopyOrder::default();
        let capture = order.request();
        let recognition = order.request();
        assert!(
            order
                .write::<()>(recognition, || anyhow::bail!("No text found"))
                .is_err()
        );
        assert!(order.write(capture, || Ok(())).unwrap().is_some());
    }
}
