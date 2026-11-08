//! Sniplet's shared UI parts on Rune: the rune-kit theme, the few controls rune-kit does not
//! have, icons, and the inbox that carries background results and messages between windows.
mod controls;
pub mod icons;
mod inbox;
mod theme;

pub use controls::{chip, segment, segments, selected, surface};
pub use inbox::Inbox;
#[cfg(test)]
pub use theme::kit;
pub use theme::{accent, core_color, hud, rgb, window_kit};
