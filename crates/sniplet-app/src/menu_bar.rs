use gpui_kit::{AssetSource, assets::AllAssets};
use sniplet_platform::HotkeySettings;
use tray_icon::menu::{
    CheckMenuItem, Icon, IconMenuItem, Menu, MenuId, PredefinedMenuItem, Submenu,
    accelerator::Accelerator,
};

use crate::editor::Command;

pub struct NativeMenu {
    pub menu: Menu,
    pub startup: CheckMenuItem,
    items: Vec<(IconMenuItem, Command)>,
    hotkeys: HotkeySettings,
}

impl NativeMenu {
    pub fn new(hotkeys: &HotkeySettings) -> anyhow::Result<Self> {
        let menu = Menu::new();
        let mut items = Vec::new();
        for (label, icon, command) in [
            ("Reopen Sniplet", "scissors", Command::ShowEditor),
            ("Capture Screen", "monitor", Command::Screen),
            ("Capture Area", "scan", Command::Area),
            ("Scrolling Capture", "chevrons-down", Command::Scroll),
            ("Recognize Text/QR", "scan-text", Command::CaptureOcr),
        ] {
            let item = menu_item(label, icon, shortcut(command, hotkeys));
            menu.append(&item)?;
            items.push((item, command));
            if matches!(command, Command::ShowEditor) {
                menu.append(&PredefinedMenuItem::separator())?;
            }
        }
        let more = Submenu::new("More", true);
        #[cfg(target_os = "macos")]
        more.set_icon_templated(menu_icon("ellipsis"));
        for (label, icon, command) in [
            ("Repeat Area Capture", "scan", Command::Repeat),
            ("Capture Active Window", "monitor", Command::ActiveWindow),
            ("Capture Any Window", "monitor", Command::Window),
            ("Delayed Screenshot (3s)", "timer", Command::Delayed),
            ("Scrolling (Up)", "chevrons-up", Command::ScrollUp),
            ("Open File…", "file", Command::Open),
            ("Load From Clipboard", "clipboard", Command::LoadClipboard),
        ] {
            if matches!(command, Command::Open) {
                more.append(&PredefinedMenuItem::separator())?;
            }
            let item = menu_item(label, icon, shortcut(command, hotkeys));
            more.append(&item)?;
            items.push((item, command));
        }
        menu.append(&more)?;
        menu.append(&PredefinedMenuItem::separator())?;
        let github = menu_item("Sniplet on GitHub", "github", None);
        menu.append(&github)?;
        items.push((github, Command::GitHub));
        menu.append(&PredefinedMenuItem::separator())?;
        #[cfg(target_os = "macos")]
        let enabled = crate::macos::launch_at_startup();
        #[cfg(not(target_os = "macos"))]
        let enabled = false;
        let startup = CheckMenuItem::new(
            "Launch at Startup",
            cfg!(target_os = "macos"),
            enabled,
            None,
        );
        menu.append(&startup)?;
        for (label, icon, command) in [
            ("Settings…", "settings", Command::Settings),
            ("Quit Sniplet", "power", Command::Quit),
        ] {
            let item = menu_item(label, icon, shortcut(command, hotkeys));
            menu.append(&item)?;
            items.push((item, command));
        }
        Ok(Self {
            menu,
            startup,
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
            item.set_accelerator(shortcut(*command, hotkeys))?;
        }
        self.hotkeys = hotkeys.clone();
        Ok(true)
    }
}

fn shortcut(command: Command, hotkeys: &HotkeySettings) -> Option<Accelerator> {
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
        Command::LoadClipboard => "CommandOrControl+V",
        Command::Settings => "CommandOrControl+Comma",
        Command::Quit => "CommandOrControl+Q",
        _ => return None,
    };
    if key.is_empty() {
        return None;
    }
    key.replace(
        "CommandOrControl",
        if cfg!(target_os = "macos") {
            "Super"
        } else {
            "Ctrl"
        },
    )
    .parse()
    .ok()
}

fn menu_item(label: &str, name: &str, accelerator: Option<Accelerator>) -> IconMenuItem {
    let icon = menu_icon(name);
    let item = IconMenuItem::new(label, true, icon.clone(), accelerator);
    #[cfg(target_os = "macos")]
    item.set_icon_templated(icon);
    item
}

fn menu_icon(name: &str) -> Option<Icon> {
    Icon::from_rgba(icon_pixels(name)?, 32, 32).ok()
}

/// Rasterize the same bundled icons used by the editor for the native menu.
pub fn icon_pixels(name: &str) -> Option<Vec<u8>> {
    let svg = AllAssets.load(&format!("icons/{name}.svg")).ok()??;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default()).ok()?;
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
    let mut pixels = icon_pixels("scissors")
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
    fn menu_shortcuts_follow_custom_and_disabled_capture_keys() {
        let mut hotkeys = HotkeySettings {
            capture_area: "Super+Shift+9".into(),
            ..HotkeySettings::default()
        };
        assert_eq!(
            shortcut(Command::Area, &hotkeys),
            Some("Super+Shift+9".parse::<Accelerator>().unwrap())
        );
        hotkeys.capture_area.clear();
        assert_eq!(shortcut(Command::Area, &hotkeys), None);
        assert!(shortcut(Command::Settings, &hotkeys).is_some());
    }

    #[test]
    fn native_menu_icons_are_available_in_the_bundle() {
        for name in [
            "scissors",
            "monitor",
            "scan",
            "chevrons-down",
            "scan-text",
            "ellipsis",
            "timer",
            "chevrons-up",
            "file",
            "clipboard",
            "github",
            "settings",
            "power",
        ] {
            let pixels = icon_pixels(name).expect(name);
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
        let scissors = icon_pixels("scissors").unwrap();
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
