//! PORT STUB: screen, window, delayed and scrolling captures, and pinned images.
//! The gpui version is `git show HEAD:crates/sniplet-app/src/capture.rs`.
use sniplet_core::ImageRect;

use crate::editor::{Command, Editor};

pub struct CaptureRequest {
    pub command: Command,
    pub monitor: usize,
    pub region: Option<ImageRect>,
    pub last_capture: Option<crate::area_capture::Region>,
}

/// Start the capture `request` names. Results reach the editor through its inbox.
pub fn start(request: CaptureRequest, editor: &mut Editor) -> Result<(), String> {
    let _ = (request, editor);
    Err("Capture is not ported yet".into())
}

pub fn active_window(editor: &mut Editor) -> Result<(), String> {
    let _ = editor;
    Err("Capture is not ported yet".into())
}

/// Pin `image` on screen in its own always-on-top window.
pub fn pin(image: image::RgbaImage) {
    let _ = image;
}
