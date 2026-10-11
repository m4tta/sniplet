use std::rc::Rc;

use gpui_kit::{
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};
use sniplet_platform::HotkeySettings;

use crate::editor::{Command, tool_icon};

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Run(Command),
    Help,
}

pub(crate) struct Item {
    label: &'static str,
    icon: &'static str,
    action: Action,
}

const fn run(label: &'static str, icon: &'static str, command: Command) -> Item {
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
    run("Screen", "monitor", Command::Screen),
    run("Area", "scan", Command::Area),
    run("Scrolling", "chevrons-down", Command::Scroll),
    run("Text & QR", "scan-text", Command::CaptureOcr),
];

const SETTINGS: Item = run("Settings", "settings", Command::Settings);
const QUIT: Item = run("Quit", "power", Command::Quit);

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) const TRAY: Layout = Layout {
    tiles: TILES,
    more: &[
        run("Repeat Area", "repeat", Command::Repeat),
        run("Active Window", "app-window", Command::ActiveWindow),
        run("Pick Window", "square-mouse-pointer", Command::Window),
        run("Delayed 3s", "timer", Command::Delayed),
        run("Scroll Up", "chevrons-up", Command::ScrollUp),
        run("Open File", "folder-open", Command::Open),
        run(
            "Paste From Clipboard",
            "clipboard-paste",
            Command::LoadClipboard,
        ),
    ],
    image: &[],
    footer: &[
        run("Open Editor", "scissors", Command::ShowEditor),
        SETTINGS,
        QUIT,
    ],
};

pub(crate) const EDITOR: Layout = Layout {
    tiles: TILES,
    more: &[
        run("Repeat Area", "repeat", Command::Repeat),
        run("Active Window", "app-window", Command::ActiveWindow),
        run("Pick Window", "square-mouse-pointer", Command::Window),
        run("Delayed 3s", "timer", Command::Delayed),
        run("Scroll Up", "chevrons-up", Command::ScrollUp),
        run("Manual Scrolling Capture", "mouse", Command::ManualScroll),
        run("Open File", "folder-open", Command::Open),
        // Pasting into an open image keeps it, as Ctrl+V does in the editor.
        run("Paste From Clipboard", "clipboard-paste", Command::Paste),
    ],
    image: &[
        run("Recognize Text", "square-dashed-text", Command::Ocr),
        run("Read QR Code", "qr-code", Command::Qr),
        run(
            "Auto-adjust Selection",
            "wand-sparkles",
            Command::AutoAdjust,
        ),
        run("Reset Crop", "crop", Command::ResetCrop),
        run("Upload Image", "cloud-upload", Command::Upload),
    ],
    footer: &[
        Item {
            label: "Keyboard Shortcuts",
            icon: "keyboard",
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

    #[cfg(test)]
    fn items(&self) -> impl Iterator<Item = &'static Item> + use<> {
        self.tiles
            .iter()
            .chain(self.more)
            .chain(self.image)
            .chain(self.footer)
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

fn divider(cx: &App) -> Div {
    div().h(px(1.0)).bg(cx.theme().border)
}

/// The quick menu shared by the tray panel and the editor's app menu. `choose`
/// runs when an item is clicked; closing the menu is up to the host.
pub(crate) fn view(
    layout: &'static Layout,
    hotkeys: &HotkeySettings,
    choose: impl Fn(Action, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let choose = Rc::new(choose);
    let on_click = |action: Action| {
        let choose = choose.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| choose(action, window, cx)
    };
    let hover = cx.theme().secondary_hover;
    let tiles = layout.tiles.iter().enumerate().map(|(index, item)| {
        let tile = div()
            .id(("menu-tile", index))
            .flex_1()
            .min_w_0()
            .h(px(TILE_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .rounded_md()
            .bg(cx.theme().secondary)
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .child(tool_icon(item.icon).size(px(24.0)))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(item.label),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .children(shortcut(item.action, hotkeys)),
            )
            .on_click(on_click(item.action));
        #[cfg(feature = "ui-tests")]
        let tile = {
            use gpui_kit::test::TestSupportExt;
            tile.test_support()
        };
        tile
    });
    // Every icon row shares one slot pitch, so a shorter row lines up under the one above.
    let slots = layout.more.len().max(layout.image.len()) as f32;
    let inner_width = WIDTH - 2.0 * (BORDER + PADDING);
    let slot_gap = (inner_width - slots * SLOT_WIDTH) / (slots - 1.0).max(1.0);
    let icon_row = |id: &'static str, items: &'static [Item]| {
        div()
            .flex()
            .gap(px(slot_gap))
            .children(items.iter().enumerate().map(|(index, item)| {
                let tooltip = match shortcut(item.action, hotkeys) {
                    Some(key) => format!("{} · {key}", item.label),
                    None => item.label.to_owned(),
                };
                Button::new((id, index))
                    .ghost()
                    .icon(tool_icon(item.icon))
                    .tooltip(tooltip)
                    .w(px(SLOT_WIDTH))
                    .h(px(ROW_HEIGHT))
                    .on_click(on_click(item.action))
            }))
    };
    let mut footer = layout.footer.iter().enumerate().map(|(index, item)| {
        Button::new(("menu-footer", index))
            .ghost()
            .icon(tool_icon(item.icon))
            .label(item.label)
            .small()
            .h(px(ROW_HEIGHT))
            .on_click(on_click(item.action))
    });
    let last = footer.next_back();
    let mut menu = div()
        .id("quick-menu")
        .w(px(WIDTH))
        .flex()
        .flex_col()
        .gap(px(GAP))
        .p(px(PADDING))
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .child(div().flex().gap(px(GAP)).children(tiles))
        .child(divider(cx))
        .child(icon_row("menu-more", layout.more));
    if !layout.image.is_empty() {
        menu = menu
            .child(divider(cx))
            .child(icon_row("menu-image", layout.image));
    }
    menu.child(divider(cx)).child(
        div()
            .flex()
            .justify_between()
            .child(div().flex().children(footer))
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

    #[test]
    fn every_menu_icon_is_bundled() {
        let layouts = [
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            &TRAY,
            &EDITOR,
        ];
        for item in layouts.into_iter().flat_map(Layout::items) {
            assert!(
                AssetSource::load(
                    &gpui_kit::assets::AllAssets,
                    &format!("icons/{}.svg", item.icon)
                )
                .unwrap()
                .is_some(),
                "{}",
                item.icon
            );
        }
    }
}
