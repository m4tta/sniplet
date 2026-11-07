//! PORT STUB: the panel the tray icon opens. The gpui version is
//! `git show HEAD:crates/sniplet-app/src/tray_panel.rs`.
use crate::editor::Editor;

/// The tray icon was clicked at `rect`: open the panel there, or close it.
pub(crate) fn toggle(rect: tray_icon::Rect, editor: &mut Editor) {
    let _ = (rect, editor);
}
