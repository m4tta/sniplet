//! PORT STUB: the window capture picker. The gpui version is
//! `git show HEAD:crates/sniplet-app/src/window_picker.rs`.
use crate::editor::Editor;

pub(crate) fn open(editor: &mut Editor) -> Result<(), String> {
    let _ = editor;
    Err("Window capture is not ported yet".into())
}
