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
    Recognize { document: Document, qr_only: bool },
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
pub(crate) fn copy(source: Source, cx: &App) -> Task<anyhow::Result<Option<usize>>> {
    let request = ORDER.request();
    cx.background_executor().spawn(async move {
        let source = match source {
            Source::Recognize { document, qr_only } => {
                let image = document.render(&crate::editor::render_options())?;
                let codes = sniplet_platform::scan_qr_codes(&image)?;
                let text = if !codes.is_empty() {
                    codes
                        .into_iter()
                        .map(|code| code.content)
                        .collect::<Vec<_>>()
                        .join("\n")
                } else if qr_only {
                    anyhow::bail!("No QR code found in the selected image");
                } else {
                    sniplet_platform::recognize_text(&image, &Default::default())?
                };
                Source::Text(text)
            }
            other => other,
        };
        // Rendering is outside the write lock. Clipboard writes alone are ordered.
        let rendered = match &source {
            Source::Rendered(document) => Some(document.render(&crate::editor::render_options())?),
            _ => None,
        };
        ORDER.write(request, || {
            let mut clipboard = Clipboard::new()?;
            match source {
                Source::Original(document) => {
                    clipboard.set_image(document.original())?;
                    Ok(0)
                }
                Source::Rendered(_) => {
                    clipboard.set_image(rendered.as_ref().unwrap())?;
                    Ok(0)
                }
                Source::Text(text) => {
                    let count = text.chars().count();
                    clipboard.set_text(text)?;
                    Ok(count)
                }
                Source::Recognize { .. } => unreachable!(),
            }
        })
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
