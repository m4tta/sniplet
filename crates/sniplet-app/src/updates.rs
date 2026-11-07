//! PORT STUB: update checks, downloads and installs. The gpui version is
//! `git show HEAD:crates/sniplet-app/src/updates.rs`.
use rune::El;

use crate::{editor::Editor, ui::Palette};

pub(crate) const RESTORE_ARG: &str = "--restore-update";

pub(crate) struct Updates;

impl Updates {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn installing(&self) -> bool {
        false
    }
}

/// The whole editor window while an update installs.
pub(crate) fn installing_screen(p: &Palette) -> El {
    rune::el().w_full().h_full().fill(p.background)
}

pub(crate) fn restore(
    store: &sniplet_platform::SettingsStore,
) -> anyhow::Result<sniplet_core::Document> {
    let _ = store;
    anyhow::bail!("Updates are not ported yet")
}

pub(crate) fn clear_recovery(store: &sniplet_platform::SettingsStore) {
    let _ = store;
}

impl Editor {
    /// The app menu's update button label, when an update waits to install.
    pub(crate) fn waiting_update_label(&self) -> Option<String> {
        None
    }

    pub(crate) fn show_update_notice(&mut self) {}

    /// The update card on the canvas, if one shows.
    pub(crate) fn render_update_notice(&self, p: &Palette) -> Option<El> {
        let _ = p;
        None
    }

    /// Start the automatic update checks. Called once after launch.
    pub(crate) fn schedule_updates(&mut self) {}
}
