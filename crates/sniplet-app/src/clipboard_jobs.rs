use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use gpui_kit::{App, Task};
use sniplet_core::Document;
use sniplet_platform::Clipboard;

pub(crate) enum Source {
    Original(Document),
    Rendered(Document),
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

/// Prepare and write on a worker. An old render cannot replace a newer copy.
pub(crate) fn copy(source: Source, cx: &App) -> Task<anyhow::Result<Option<()>>> {
    let request = ORDER.request();
    cx.background_executor().spawn(async move {
        // Rendering is outside the write lock. Clipboard writes alone are ordered.
        let rendered = match &source {
            Source::Rendered(document) => Some(document.render(&crate::editor::render_options())?),
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
    })
}

/// Read QR contents, or OCR text when there is no QR code, then copy the result.
/// The order slot is taken now, so a slow recognition cannot replace a newer copy.
/// The text comes back even when the clipboard write fails, so the editor can show it.
pub(crate) fn recognize(
    document: Document,
    qr_only: bool,
    cx: &App,
) -> Task<anyhow::Result<Recognized>> {
    let request = ORDER.request();
    cx.background_executor().spawn(async move {
        let image = document.render(&crate::editor::render_options())?;
        let codes = sniplet_platform::scan_qr_codes(&image)?;
        let text = if !codes.is_empty() {
            codes
                .into_iter()
                .map(|code| code.content)
                .collect::<Vec<_>>()
                .join(
                    "
",
                )
        } else if qr_only {
            anyhow::bail!("No QR code found in the selected image");
        } else {
            sniplet_platform::recognize_text(&image, &Default::default())?
        };
        if text.trim().is_empty() {
            anyhow::bail!("No text found in the selected image");
        }
        let copied = ORDER
            .write(request, || Ok(Clipboard::new()?.set_text(text.clone())?))
            .map(|written| written.is_some());
        Ok(Recognized { text, copied })
    })
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
