use rune::{Color, Cx};
use rune_kit::{Kit, PaletteFocus, PaletteSeed, TooltipAppearance, TooltipStyle};
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

/// rune-kit's tokens for one appearance, derived from Sniplet's neutral colors and accent.
/// Every window and component draws with these.
pub fn kit(dark: bool) -> Kit {
    let (bg, fg) = if dark {
        (rgb(0x0a0a0a), rgb(0xfafafa))
    } else {
        (Color::WHITE, rgb(0x0a0a0a))
    };
    let seed = PaletteSeed {
        bg,
        fg,
        accent: rgb(ACCENT),
    };
    let mut kit = Kit::from_palette_focus(seed, PaletteFocus::Accent);
    // White switch and slider knobs in both appearances, as macOS draws them.
    kit.c.knob = Color::WHITE;
    kit.c.knob_on = Color::WHITE;
    kit
}

/// The kit for a window whose theme follows `preference`, where `System` follows the
/// window's appearance. Also gives the window kit-styled tooltips and caption buttons.
pub fn window_kit<S>(cx: &mut Cx<S>, preference: ThemePreference) -> Kit {
    let kit = kit(is_dark(preference, cx.system_dark()));
    cx.caption_color(kit.c.text_base);
    cx.help_look(TooltipStyle::from_kit(&kit, TooltipAppearance::Compact).help_look());
    kit
}

/// The dark kit that capture overlays and pins draw their controls with, over any image.
pub fn hud() -> Kit {
    kit(true)
}

/// `k` with its contrast buttons filled with the accent, for a window's main action.
pub fn accent(k: &Kit) -> Kit {
    let mut accent = k.clone();
    accent.c.bg_contrast = k.c.bg_accent;
    accent.c.text_contrast = Color::WHITE;
    accent.c.contrast_hover = Color::WHITE.alpha(0.12);
    accent.c.contrast_pressed = Color::BLACK.alpha(0.12);
    accent
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

    #[test]
    fn the_kit_takes_sniplets_accent_and_appearance() {
        for dark in [false, true] {
            let kit = kit(dark);
            assert_eq!(kit.dark, dark);
            assert_eq!(kit.c.bg_accent.hex(), "#0a84ffff");
            assert_eq!(accent(&kit).c.bg_contrast.hex(), "#0a84ffff");
        }
    }
}
