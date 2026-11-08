use rune::Icon as Glyph;
use sniplet_platform::HotkeySettings;
#[cfg(target_os = "linux")]
use tray_icon::menu::{
    CheckMenuItem, Icon, IconMenuItem, Menu, MenuId, PredefinedMenuItem, Submenu,
    accelerator::Accelerator,
};

use crate::editor::Command;
#[cfg(target_os = "linux")]
use crate::ui::icons;

#[cfg(target_os = "linux")]
pub struct NativeMenu {
    pub menu: Menu,
    items: Vec<(IconMenuItem, Command)>,
    hotkeys: HotkeySettings,
}

#[cfg(target_os = "linux")]
impl NativeMenu {
    pub fn new(hotkeys: &HotkeySettings) -> anyhow::Result<Self> {
        let menu = Menu::new();
        let mut items = Vec::new();
        for (label, icon, command) in [
            ("Reopen Sniplet", &icons::SCISSORS, Command::ShowEditor),
            ("Capture Screen", &icons::MONITOR, Command::Screen),
            ("Capture Area", &icons::SCAN, Command::Area),
            ("Scrolling Capture", &icons::CHEVRONS_DOWN, Command::Scroll),
            ("Recognize Text/QR", &icons::SCAN_TEXT, Command::CaptureOcr),
        ] {
            let item = menu_item(label, icon, accelerator(command, hotkeys));
            menu.append(&item)?;
            items.push((item, command));
            if matches!(command, Command::ShowEditor) {
                menu.append(&PredefinedMenuItem::separator())?;
            }
        }
        let more = Submenu::new("More", true);
        for (label, icon, command) in [
            ("Repeat Area Capture", &icons::SCAN, Command::Repeat),
            (
                "Capture Active Window",
                &icons::MONITOR,
                Command::ActiveWindow,
            ),
            ("Capture Any Window", &icons::MONITOR, Command::Window),
            ("Delayed Screenshot (3s)", &icons::TIMER, Command::Delayed),
            ("Scrolling (Up)", &icons::CHEVRONS_UP, Command::ScrollUp),
            ("Open File…", &icons::FILE, Command::Open),
            (
                "Load From Clipboard",
                &icons::CLIPBOARD,
                Command::LoadClipboard,
            ),
        ] {
            if matches!(command, Command::Open) {
                more.append(&PredefinedMenuItem::separator())?;
            }
            let item = menu_item(label, icon, accelerator(command, hotkeys));
            more.append(&item)?;
            items.push((item, command));
        }
        menu.append(&more)?;
        menu.append(&PredefinedMenuItem::separator())?;
        let github = menu_item("Sniplet on GitHub", &icons::GITHUB, None);
        menu.append(&github)?;
        items.push((github, Command::GitHub));
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&CheckMenuItem::new("Launch at Startup", false, false, None))?;
        for (label, icon, command) in [
            ("Settings…", &icons::SETTINGS, Command::Settings),
            ("Quit Sniplet", &icons::POWER, Command::Quit),
        ] {
            let item = menu_item(label, icon, accelerator(command, hotkeys));
            menu.append(&item)?;
            items.push((item, command));
        }
        Ok(Self {
            menu,
            items,
            hotkeys: hotkeys.clone(),
        })
    }

    pub fn command(&self, id: &MenuId) -> Option<Command> {
        self.items
            .iter()
            .find(|(item, _)| item.id() == id)
            .map(|(_, command)| *command)
    }

    /// Keep native shortcut labels in sync with the user's saved settings.
    pub fn update_hotkeys(&mut self, hotkeys: &HotkeySettings) -> anyhow::Result<bool> {
        if self.hotkeys == *hotkeys {
            return Ok(false);
        }
        for (item, command) in &self.items {
            item.set_accelerator(accelerator(*command, hotkeys))?;
        }
        self.hotkeys = hotkeys.clone();
        Ok(true)
    }
}

/// The shortcut text bound to `command`, such as `CommandOrControl+Shift+2`.
pub fn shortcut(command: Command, hotkeys: &HotkeySettings) -> Option<&str> {
    let key = match command {
        Command::ShowEditor => &hotkeys.show_editor,
        Command::Screen => &hotkeys.capture_screen,
        Command::Area => &hotkeys.capture_area,
        Command::Scroll => &hotkeys.scrolling_capture,
        Command::CaptureOcr => &hotkeys.capture_ocr,
        Command::Repeat => &hotkeys.repeat_area,
        Command::ActiveWindow => &hotkeys.active_window,
        Command::Window => &hotkeys.capture_window,
        Command::Open => "CommandOrControl+O",
        Command::Paste | Command::LoadClipboard => "CommandOrControl+V",
        Command::Settings => "CommandOrControl+Comma",
        Command::Quit => "CommandOrControl+Q",
        _ => return None,
    };
    (!key.is_empty()).then_some(key)
}

#[cfg(target_os = "linux")]
fn accelerator(command: Command, hotkeys: &HotkeySettings) -> Option<Accelerator> {
    shortcut(command, hotkeys)?
        .replace("CommandOrControl", "Ctrl")
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn menu_item(label: &str, icon: &'static Glyph, accelerator: Option<Accelerator>) -> IconMenuItem {
    let icon = icon_pixels(icon).and_then(|pixels| Icon::from_rgba(pixels, 32, 32).ok());
    IconMenuItem::new(label, true, icon, accelerator)
}

/// Rasterize a bundled icon at the tray's 32 px size, in black.
pub fn icon_pixels(icon: &Glyph) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(icon.body.as_bytes(), &resvg::usvg::Options::default())
        .ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(32, 32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            32.0 / tree.size().width(),
            32.0 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    Some(pixmap.take())
}

/// Use the menu bar scissors in the tray, with ink suited to its background.
pub fn tray_icon(light_background: bool) -> anyhow::Result<tray_icon::Icon> {
    Ok(tray_icon::Icon::from_rgba(
        tray_icon_pixels(light_background)?,
        32,
        32,
    )?)
}

fn tray_icon_pixels(light_background: bool) -> anyhow::Result<Vec<u8>> {
    let mut pixels = icon_pixels(&crate::ui::icons::SCISSORS)
        .ok_or_else(|| anyhow::anyhow!("The menu bar icon could not be loaded"))?;
    let ink = if light_background { 0 } else { 255 };
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[..3].fill(ink);
    }
    Ok(pixels)
}

#[cfg(target_os = "windows")]
pub fn taskbar_uses_light_theme() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_u32("SystemUsesLightTheme"))
        .is_ok_and(|value| value != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_follow_custom_and_disabled_capture_keys() {
        let mut hotkeys = HotkeySettings {
            capture_area: "Super+Shift+9".into(),
            ..HotkeySettings::default()
        };
        assert_eq!(shortcut(Command::Area, &hotkeys), Some("Super+Shift+9"));
        hotkeys.capture_area.clear();
        assert_eq!(shortcut(Command::Area, &hotkeys), None);
        assert_eq!(
            shortcut(Command::Settings, &hotkeys),
            Some("CommandOrControl+Comma")
        );
        assert_eq!(shortcut(Command::Delayed, &hotkeys), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_menu_accelerators_use_control() {
        assert_eq!(
            accelerator(Command::Screen, &HotkeySettings::default()),
            Some("Ctrl+Shift+1".parse::<Accelerator>().unwrap())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_menu_icons_are_available_in_the_bundle() {
        for (name, icon) in [
            ("scissors", &icons::SCISSORS),
            ("monitor", &icons::MONITOR),
            ("scan", &icons::SCAN),
            ("chevrons-down", &icons::CHEVRONS_DOWN),
            ("scan-text", &icons::SCAN_TEXT),
            ("timer", &icons::TIMER),
            ("chevrons-up", &icons::CHEVRONS_UP),
            ("file", &icons::FILE),
            ("clipboard", &icons::CLIPBOARD),
            ("github", &icons::GITHUB),
            ("settings", &icons::SETTINGS),
            ("power", &icons::POWER),
        ] {
            let pixels = icon_pixels(icon).expect(name);
            assert_eq!(pixels.len(), 32 * 32 * 4);
            assert!(
                pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0),
                "{name}"
            );
        }
    }

    #[test]
    fn tray_uses_the_same_scissors_mask_in_both_themes() {
        let light = tray_icon_pixels(true).unwrap();
        let dark = tray_icon_pixels(false).unwrap();
        let scissors = icon_pixels(&crate::ui::icons::SCISSORS).unwrap();
        for ((light, dark), source) in light
            .as_chunks::<4>()
            .0
            .iter()
            .zip(dark.as_chunks::<4>().0)
            .zip(scissors.as_chunks::<4>().0)
        {
            assert_eq!(&light[..3], &[0, 0, 0]);
            assert_eq!(&dark[..3], &[255, 255, 255]);
            assert_eq!(light[3], source[3]);
            assert_eq!(dark[3], source[3]);
        }
    }
}
