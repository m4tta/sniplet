//! PORT STUB: the settings window. The gpui version is
//! `git show HEAD:crates/sniplet-app/src/settings_window.rs`.
use crate::editor::Editor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Capture,
    Shortcuts,
    Share,
}

/// Open the settings window on `page`, or bring it forward on that page.
pub(crate) fn open(editor: &mut Editor, page: Page) -> anyhow::Result<()> {
    let _ = (editor, page);
    anyhow::bail!("Settings are not ported yet")
}
