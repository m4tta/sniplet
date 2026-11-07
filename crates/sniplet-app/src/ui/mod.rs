//! Sniplet's shared UI parts on Rune: colors, icons, buttons and the inbox that carries
//! background results and messages between windows.
mod button;
pub mod icons;
mod inbox;
mod palette;

pub use button::{BODY, Button, SEMIBOLD, SMALL, divider};
pub use inbox::Inbox;
pub use palette::{ACCENT, Palette, core_color, is_dark, rgb};
