use std::rc::Rc;

use rune::{El, Icon, Tx, col, el, icon, row, text};
use sniplet_platform::HotkeySettings;

use crate::editor::Command;
use crate::ui::{Button, Palette, SMALL, icons};

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Run(Command),
    Help,
}

pub(crate) struct Item {
    label: &'static str,
    icon: &'static Icon,
    action: Action,
}

const fn run(label: &'static str, icon: &'static Icon, command: Command) -> Item {
    Item {
        label,
        icon,
        action: Action::Run(command),
    }
}

/// The sections a host shows, each row drawn in order. An empty `image` row is omitted.
pub(crate) struct Layout {
    tiles: &'static [Item],
    more: &'static [Item],
    image: &'static [Item],
    footer: &'static [Item],
}

const TILES: &[Item] = &[
    run("Screen", &icons::MONITOR, Command::Screen),
    run("Area", &icons::SCAN, Command::Area),
    run("Scrolling", &icons::CHEVRONS_DOWN, Command::Scroll),
    run("Text & QR", &icons::SCAN_TEXT, Command::CaptureOcr),
];

const SETTINGS: Item = run("Settings", &icons::SETTINGS, Command::Settings);
const QUIT: Item = run("Quit", &icons::POWER, Command::Quit);

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) const TRAY: Layout = Layout {
    tiles: TILES,
    more: &[
        run("Repeat Area", &icons::REPEAT, Command::Repeat),
        run("Active Window", &icons::APP_WINDOW, Command::ActiveWindow),
        run("Pick Window", &icons::SQUARE_MOUSE_POINTER, Command::Window),
        run("Delayed 3s", &icons::TIMER, Command::Delayed),
        run("Scroll Up", &icons::CHEVRONS_UP, Command::ScrollUp),
        run("Open File", &icons::FOLDER_OPEN, Command::Open),
        run(
            "Paste From Clipboard",
            &icons::CLIPBOARD_PASTE,
            Command::LoadClipboard,
        ),
    ],
    image: &[],
    footer: &[
        run("Open Editor", &icons::SCISSORS, Command::ShowEditor),
        SETTINGS,
        QUIT,
    ],
};

pub(crate) const EDITOR: Layout = Layout {
    tiles: TILES,
    more: &[
        run("Repeat Area", &icons::REPEAT, Command::Repeat),
        run("Active Window", &icons::APP_WINDOW, Command::ActiveWindow),
        run("Pick Window", &icons::SQUARE_MOUSE_POINTER, Command::Window),
        run("Delayed 3s", &icons::TIMER, Command::Delayed),
        run("Scroll Up", &icons::CHEVRONS_UP, Command::ScrollUp),
        run(
            "Manual Scrolling Capture",
            &icons::MOUSE,
            Command::ManualScroll,
        ),
        run("Open File", &icons::FOLDER_OPEN, Command::Open),
        // Pasting into an open image keeps it, as Ctrl+V does in the editor.
        run(
            "Paste From Clipboard",
            &icons::CLIPBOARD_PASTE,
            Command::Paste,
        ),
    ],
    image: &[
        run("Recognize Text", &icons::SQUARE_DASHED_TEXT, Command::Ocr),
        run("Read QR Code", &icons::QR_CODE, Command::Qr),
        run(
            "Auto-adjust Selection",
            &icons::WAND_SPARKLES,
            Command::AutoAdjust,
        ),
        run("Reset Crop", &icons::CROP, Command::ResetCrop),
        run("Add Capture", &icons::IMAGE_PLUS, Command::AddCapture),
        run("Upload Image", &icons::CLOUD_UPLOAD, Command::Upload),
    ],
    footer: &[
        Item {
            label: "Keyboard Shortcuts",
            icon: &icons::KEYBOARD,
            action: Action::Help,
        },
        SETTINGS,
        QUIT,
    ],
};

pub(crate) const WIDTH: f32 = 352.0;
const BORDER: f32 = 1.0;
const PADDING: f32 = 8.0;
const GAP: f32 = 8.0;
const TILE_HEIGHT: f32 = 80.0;
const ROW_HEIGHT: f32 = 32.0;
const SLOT_WIDTH: f32 = 40.0;

impl Layout {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    pub(crate) fn height(&self) -> f32 {
        let rows = [self.more, self.image, self.footer]
            .iter()
            .filter(|row| !row.is_empty())
            .count() as f32;
        2.0 * PADDING + TILE_HEIGHT + rows * (ROW_HEIGHT + 1.0 + 2.0 * GAP)
    }
}

/// A readable shortcut such as `Ctrl+Shift+2`, from a settings string such as
/// `CommandOrControl+Shift+2`.
fn shortcut_label(key: &str, mac: bool) -> String {
    key.split('+')
        .map(|part| match part {
            "CommandOrControl" | "CmdOrCtrl" if mac => "Cmd",
            "CommandOrControl" | "CmdOrCtrl" | "Control" => "Ctrl",
            "Super" | "Command" | "Meta" if mac => "Cmd",
            "Super" | "Command" | "Meta" => "Win",
            "Alt" if mac => "Option",
            "Comma" => ",",
            part => part
                .strip_prefix("Key")
                .or_else(|| part.strip_prefix("Digit"))
                .unwrap_or(part),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn shortcut(action: Action, hotkeys: &HotkeySettings) -> Option<String> {
    let Action::Run(command) = action else {
        return None;
    };
    crate::menu_bar::shortcut(command, hotkeys)
        .map(|key| shortcut_label(key, cfg!(target_os = "macos")))
}

fn divider(p: &Palette) -> El {
    el().h(1.0).w_full().fill(p.border)
}

/// The quick menu shared by the tray panel and the editor's app menu. `choose` runs when an
/// item is pressed; closing the menu is up to the host.
pub(crate) fn view<S: 'static>(
    p: &Palette,
    layout: &'static Layout,
    hotkeys: &HotkeySettings,
    choose: impl Fn(&mut S, Action, &mut Tx) + 'static,
) -> El {
    let choose = Rc::new(choose);
    let on_press = |action: Action| {
        let choose = choose.clone();
        move |s: &mut S, tx: &mut Tx| choose(s, action, tx)
    };
    let tile_fill = p.border;
    let tiles = layout.tiles.iter().enumerate().map(|(index, item)| {
        col()
            .key(format!("menu-tile-{index}"))
            .role("button")
            .label(item.label)
            .grow(1.0)
            .min_w(0.0)
            .h(TILE_HEIGHT)
            .center()
            .gap(4.0)
            .radius(6.0)
            .fill(tile_fill)
            .hover(tile_fill.mix(p.foreground, 0.08))
            .cursor(rune::Cursor::Pointer)
            .child(icon(item.icon, 24.0).color(p.foreground))
            .child(
                text(item.label)
                    .font(rune::TextStyle::new(14.0, 500.0))
                    .color(p.foreground),
            )
            .child(
                text(shortcut(item.action, hotkeys).unwrap_or_default())
                    .font(SMALL)
                    .color(p.muted_foreground),
            )
            .on_press_tx(on_press(item.action))
    });
    // Every icon row shares one slot pitch, so a shorter row lines up under the one above.
    let slots = layout.more.len().max(layout.image.len()) as f32;
    let inner_width = WIDTH - 2.0 * (BORDER + PADDING);
    let slot_gap = (inner_width - slots * SLOT_WIDTH) / (slots - 1.0).max(1.0);
    let icon_row = |id: &'static str, items: &'static [Item]| {
        row()
            .gap(slot_gap)
            .children(items.iter().enumerate().map(|(index, item)| {
                let help = match shortcut(item.action, hotkeys) {
                    Some(key) => format!("{} · {key}", item.label),
                    None => item.label.to_owned(),
                };
                Button::new(format!("{id}-{index}"))
                    .icon(item.icon)
                    .help(help)
                    .size(SLOT_WIDTH, ROW_HEIGHT)
                    .build(p, on_press(item.action))
            }))
    };
    let mut footer: Vec<El> = layout
        .footer
        .iter()
        .enumerate()
        .map(|(index, item)| {
            Button::new(format!("menu-footer-{index}"))
                .icon(item.icon)
                .label(item.label)
                .compact()
                .build(p, on_press(item.action))
        })
        .collect();
    let last = footer.pop();
    let mut menu = col()
        .key("quick-menu")
        .w(WIDTH)
        .gap(GAP)
        .padding(PADDING)
        .fill(p.popover)
        .border(BORDER, p.border)
        .radius(8.0)
        .child(row().gap(GAP).children(tiles))
        .child(divider(p))
        .child(icon_row("menu-more", layout.more));
    if !layout.image.is_empty() {
        menu = menu
            .child(divider(p))
            .child(icon_row("menu-image", layout.image));
    }
    menu.child(divider(p)).child(
        row()
            .justify_between()
            .child(row().children(footer))
            .children(last),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn shortcut_labels_are_readable_on_each_platform() {
        assert_eq!(
            shortcut_label("CommandOrControl+Shift+2", false),
            "Ctrl+Shift+2"
        );
        assert_eq!(
            shortcut_label("CommandOrControl+Shift+2", true),
            "Cmd+Shift+2"
        );
        assert_eq!(shortcut_label("Control+Alt+KeyA", false), "Ctrl+Alt+A");
        assert_eq!(shortcut_label("Super+Alt+Digit9", true), "Cmd+Option+9");
        assert_eq!(shortcut_label("CommandOrControl+Comma", false), "Ctrl+,");
    }

    #[test]
    fn shortcuts_follow_the_saved_hotkeys_and_skip_unbound_items() {
        let hotkeys = HotkeySettings {
            capture_area: "Super+Shift+9".into(),
            ..HotkeySettings::default()
        };
        let label = |action| shortcut(action, &hotkeys);
        let mac = cfg!(target_os = "macos");
        assert_eq!(
            label(Action::Run(Command::Area)).as_deref(),
            Some(if mac { "Cmd+Shift+9" } else { "Win+Shift+9" })
        );
        assert_eq!(label(Action::Run(Command::ManualScroll)), None);
        assert_eq!(label(Action::Help), None);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn tray_height_fits_two_rows_and_the_editor_adds_the_image_row() {
        assert_eq!(TRAY.height(), 194.0);
        assert_eq!(EDITOR.height(), 243.0);
    }
}
