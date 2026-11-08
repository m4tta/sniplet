use rune::Color;
use sniplet_platform::ThemePreference;

/// Sniplet's accent blue for selected and active states.
pub const ACCENT: u32 = 0x0a84ff;

/// An sRGB `0xRRGGBB` color as Rune's linear color.
pub fn rgb(hex: u32) -> Color {
    let channel = |shift: u32| rune::color::srgb_decode(((hex >> shift) & 0xff) as f32 / 255.0);
    Color {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

/// A Sniplet annotation color as Rune's linear color.
pub fn core_color(color: sniplet_core::Color) -> Color {
    let channel = |value: u8| rune::color::srgb_decode(f32::from(value) / 255.0);
    Color {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
        a: f32::from(color.a) / 255.0,
    }
}

/// The colors every Sniplet window draws with, for one appearance. The values follow the
/// neutral theme Sniplet used before it moved to Rune, so the app keeps its look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// The window and canvas surround.
    pub background: Color,
    pub foreground: Color,
    pub title_bar: Color,
    /// Floating surfaces: the dock, menus, panels and dialogs.
    pub popover: Color,
    pub border: Color,
    /// Quiet fills, such as the canvas behind the image.
    pub muted: Color,
    pub muted_foreground: Color,
    /// A ghost control's hover and selected fill.
    pub hover: Color,
    /// The neutral filled button.
    pub primary: Color,
    pub primary_foreground: Color,
    pub accent: Color,
    pub danger: Color,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        // Tailwind's neutral scale.
        let neutral = |step: u32| {
            rgb(match step {
                50 => 0xfafafa,
                100 => 0xf5f5f5,
                200 => 0xe5e5e5,
                400 => 0xa3a3a3,
                500 => 0x737373,
                800 => 0x262626,
                900 => 0x171717,
                _ => 0x0a0a0a,
            })
        };
        if dark {
            Self {
                dark,
                background: neutral(950),
                foreground: neutral(50),
                title_bar: neutral(900),
                popover: neutral(950),
                border: neutral(800),
                muted: neutral(800),
                muted_foreground: neutral(400),
                hover: neutral(800),
                primary: neutral(50),
                primary_foreground: neutral(900),
                accent: rgb(ACCENT),
                danger: rgb(0xf87171),
            }
        } else {
            Self {
                dark,
                background: Color::WHITE,
                foreground: neutral(950),
                title_bar: rgb(0xf8f8f8),
                popover: Color::WHITE,
                border: neutral(200),
                muted: neutral(100),
                muted_foreground: neutral(500),
                hover: neutral(100),
                primary: neutral(900),
                primary_foreground: neutral(50),
                accent: rgb(ACCENT),
                danger: rgb(0xef4444),
            }
        }
    }

    /// The palette for a preference, where `System` follows the window's appearance.
    pub fn for_preference(preference: ThemePreference, system_dark: bool) -> Self {
        Self::new(is_dark(preference, system_dark))
    }

    /// Kit components (fields, switches, selects) themed to match.
    pub fn kit(&self) -> rune_kit::Kit {
        rune_kit::Kit::from_palette(rune_kit::PaletteSeed {
            bg: self.background,
            fg: self.foreground,
            accent: self.accent,
        })
    }
}

/// Whether a preference draws dark, where `System` follows the system appearance.
pub fn is_dark(preference: ThemePreference, system_dark: bool) -> bool {
    match preference {
        ThemePreference::System => system_dark,
        ThemePreference::Light => false,
        ThemePreference::Dark => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_follows_the_appearance_and_explicit_choices_ignore_it() {
        for system_dark in [false, true] {
            assert_eq!(is_dark(ThemePreference::System, system_dark), system_dark);
            assert!(!is_dark(ThemePreference::Light, system_dark));
            assert!(is_dark(ThemePreference::Dark, system_dark));
        }
    }

    #[test]
    fn hex_colors_round_trip_through_linear_light() {
        assert_eq!(rgb(ACCENT).hex(), "#0a84ffff");
    }
}
