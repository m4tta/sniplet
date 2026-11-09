//! The Settings window's view: the title band, four tabs and the selected page. The card,
//! section and row helpers are shared with the Updates section in `updates.rs`.
use rune::{
    Color, Cursor, Cx, Drag, El, Phase, Pixels, ScrollSpec, ShortcutError, SurfaceFit, Tx, col, el,
    icon, row, text,
};
use rune_kit::{
    Button, ButtonVariant, Card, ControlSize, Divider, Field as KitField, IconButton,
    IconButtonVariant, Kit, SegmentedControl, ShortcutRecorder, ShortcutRecorderState, Switch,
    TextInput, Toast, ToastRegion,
};
use sniplet_platform::{
    ExportFormat, HotkeySettings, Settings, ThemePreference, WindowBackground, WindowCaptureStyle,
};

use super::{
    Field, Page, SHORTCUT_GROUPS, SettingsUi, Slider, UploadKind, hotkey_text, shortcut_name,
    step_limit,
};
use crate::ui::{self, icons, rgb};

/// Preset window backgrounds. The hex field still accepts any color.
pub(super) const SWATCHES: [[u8; 3]; 6] = [
    [0xF2, 0xF2, 0xF7],
    [0xFF, 0xFF, 0xFF],
    [0x1C, 0x1C, 0x1E],
    [0xE8, 0xF0, 0xFE],
    [0xFD, 0xF2, 0xE9],
    [0x0A, 0x84, 0xFF],
];

const BACKGROUNDS: [(&str, &str, WindowBackground); 4] = [
    ("window-wallpaper", "Wallpaper", WindowBackground::Wallpaper),
    (
        "window-transparent",
        "Transparent",
        WindowBackground::Transparent,
    ),
    ("window-solid", "Solid color", WindowBackground::Solid),
    ("window-trim", "Trim shadow", WindowBackground::TrimShadow),
];

/// The upload form's fields as (label, placeholder), in `SettingsUi::upload` order: two for a
/// signed URL, then five for S3.
const UPLOAD_FIELDS: [(&str, &str); 7] = [
    ("Signed PUT URL", "https://… signed PUT URL"),
    ("Image link (optional)", "https://… public image link"),
    ("Bucket", "screenshots"),
    ("Region", "us-east-1"),
    ("Endpoint (optional)", "Leave empty for Amazon S3"),
    ("Folder prefix (optional)", "sniplet/"),
    ("Public base URL (optional)", "https://… public base URL"),
];

pub(super) fn view(s: &SettingsUi, cx: &mut Cx<SettingsUi>) -> El {
    let k = ui::window_kit(cx, s.settings.theme);
    if s.snapshot.installing {
        return crate::updates::installing_screen(&k);
    }
    let page = match s.page {
        Page::General => general(s, &k),
        Page::Capture => capture(s, &k),
        Page::Shortcuts => shortcuts(s, &k),
        Page::Share => share(s, &k),
    };
    let insets = cx.chrome_insets();
    let mut root = col()
        .key("settings-window")
        .w_full()
        .h_full()
        .fill(k.c.page);
    // Where the native frame stays (Linux), it already shows the title.
    if insets.height > 0.0 {
        root = root.child(
            row()
                .key("settings-title")
                .w_full()
                .h(insets.height)
                .px(insets.leading.max(insets.trailing))
                .center()
                .window_drag()
                .child(text("Settings").font(k.ty.ui_medium).color(k.c.text_base)),
        );
    }
    let content = col()
        .key(format!("settings-content-{}", s.page as usize))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(col().w_full().padding(24.0).child(page));
    root.child(tabs(s, &k))
        .child(k.scroll_bar(content).w_full().grow(1.0).min_h(0.0))
        .children(toast(s, &k))
}

/// The page tabs, as a preferences window's toolbar shows them: an icon over a title.
fn tabs(s: &SettingsUi, k: &Kit) -> El {
    let tabs = row()
        .key("settings-tabs")
        .role("tablist")
        .w_full()
        .padding(8.0)
        .gap(4.0)
        .justify_center()
        .fill(k.c.bg_deep)
        .children(Page::ALL.into_iter().enumerate().map(|(index, page)| {
            let selected = s.page == page;
            let (glyph, font, ink) = if selected {
                (k.c.icon_accent, k.ty.text_xsmall_medium, k.c.text_base)
            } else {
                (k.c.icon_muted, k.ty.text_xsmall, k.c.text_muted)
            };
            tile(k, &format!("settings-nav-{index}"), selected, 96.0, 56.0)
                .role("tab")
                .label(page.title())
                .gap(4.0)
                .on_press_tx(move |s: &mut SettingsUi, tx: &mut Tx| {
                    // Pages switch at once, as tabs do.
                    tx.instant();
                    s.select_page(page);
                })
                .child(icon(page.icon(), 20.0).color(glyph))
                .child(text(page.title()).font(font).color(ink).single_line())
        }));
    col()
        .w_full()
        .child(tabs)
        .child(Divider::horizontal().build(k))
}

/// The save result, as a kit toast at the window's bottom right. An error stays until it is
/// closed.
fn toast(s: &SettingsUi, k: &Kit) -> Option<El> {
    let notice = s.notice.as_ref().filter(|notice| notice.field.is_none())?;
    // The kit cuts a title to one line, so an error's full text goes in the description.
    let toast = if notice.error {
        Toast::new("settings-toast", "Something went wrong")
            .description(notice.text.clone())
            .on_close(|s: &mut SettingsUi| s.clear_notice())
            .lead(icon(&icons::CIRCLE_ALERT, 16.0).color(k.c.danger.icon))
    } else {
        Toast::new("settings-toast", notice.text.clone())
            .lead(icon(&icons::CHECK, 16.0).color(k.c.success.icon))
    };
    Some(
        ToastRegion::new()
            .toast(toast.build(k))
            .build(k)
            .pin(None, None, Some(16.0), Some(16.0)),
    )
}

fn general(s: &SettingsUi, k: &Kit) -> El {
    let themes = [
        ("theme-system", "System", ThemePreference::System),
        ("theme-light", "Light", ThemePreference::Light),
        ("theme-dark", "Dark", ThemePreference::Dark),
    ];
    let appearance =
        row()
            .flex_wrap()
            .gap(12.0)
            .children(themes.map(|(key, label, preference)| {
                tile(k, key, s.settings.theme == preference, 132.0, 106.0)
                    .label(label)
                    .on_press(move |s: &mut SettingsUi| s.change(move |s| s.theme = preference))
                    .child(appearance_preview(preference))
                    .child(text(label).font(k.ty.ui).color(k.c.text_base))
            }));
    let startup = Switch::new(s.startup.enabled)
        .label("Launch at startup")
        .hide_label()
        .disabled(s.startup.busy)
        .on_toggle(|s: &mut SettingsUi| s.set_startup(!s.startup.enabled))
        .build(k)
        .key("launch-at-startup");
    let about = row()
        .justify_center()
        .center_y()
        .gap(8.0)
        .child(
            text(format!("Sniplet {}", sniplet_platform::updates::VERSION))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted),
        )
        .child(text("·").font(k.ty.text_xsmall).color(k.c.text_muted))
        .child(
            Button::new("Show settings file")
                .variant(ButtonVariant::GhostMuted)
                .on_press(|_: &mut SettingsUi| {
                    if let Ok(store) = sniplet_platform::SettingsStore::for_app() {
                        super::reveal(store.path());
                    }
                })
                .build(k)
                .key("reveal-settings"),
        );
    page()
        .child(section(
            k,
            "Appearance",
            card(k).padding(12.0).child(appearance),
        ))
        .child(section(
            k,
            "App",
            card(k)
                .child(setting_row_with_error(
                    k,
                    "Launch at startup",
                    "Start in the tray when you sign in. The editor stays closed.",
                    s.field_error(Field::Startup),
                    startup,
                ))
                .child(rule(k))
                .child(toggle(
                    k,
                    "always-on-top",
                    "Keep editor on top",
                    "Takes effect after restarting Sniplet.",
                    s.settings.always_on_top,
                    |s, on| s.always_on_top = on,
                )),
        ))
        .child(crate::updates::settings_section::<SettingsUi>(
            k,
            &s.snapshot.updates,
            &s.editor,
        ))
        .child(about)
}

fn capture(s: &SettingsUi, k: &Kit) -> El {
    let limit = s.settings.scroll_max_frames;
    let step = |key: &'static str, glyph, help: &'static str, disabled: bool, up: bool| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::Ghost)
            .help(help)
            .disabled(disabled)
            .on_press(move |s: &mut SettingsUi| {
                s.change(move |s| s.scroll_max_frames = step_limit(limit, up))
            })
            .build(k)
            .key(key)
    };
    let stepper = row()
        .h(28.0)
        .px(2.0)
        .center_y()
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_base)
        .child(step(
            "scroll-limit-down",
            &icons::MINUS,
            "Fewer frames",
            limit <= 2,
            false,
        ))
        .child(
            text(limit.to_string())
                .key("scroll-limit")
                .w(40.0)
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .align(rune::Align::Center),
        )
        .child(step(
            "scroll-limit-up",
            &icons::PLUS,
            "More frames",
            limit >= 200,
            true,
        ));
    let speed = col()
        .w(200.0)
        .gap(8.0)
        .child(slider(
            s,
            k,
            Slider::Speed,
            "scroll-speed",
            "Scrolling speed",
            200.0,
        ))
        .child(
            row()
                .justify_between()
                .child(text("Slower").font(k.ty.text_xsmall).color(k.c.text_muted))
                .child(text("Faster").font(k.ty.text_xsmall).color(k.c.text_muted)),
        );
    page()
        .child(window_background(s, k))
        .child(section(
            k,
            "After capture",
            card(k)
                .child(toggle(
                    k,
                    "auto-copy",
                    "Copy captures automatically",
                    "A fresh capture is ready to paste.",
                    s.settings.auto_copy,
                    |s, on| s.auto_copy = on,
                ))
                .child(rule(k))
                .child(toggle(
                    k,
                    "hide-after-export",
                    "Hide editor after copy or save",
                    "Return to the app you were working in.",
                    s.settings.hide_after_export,
                    |s, on| s.hide_after_export = on,
                )),
        ))
        .child(section(
            k,
            "Scrolling capture",
            card(k)
                .child(setting_row(
                    k,
                    "Capture limit",
                    "Most frames in one scrolling capture (2–200).",
                    stepper,
                ))
                .child(rule(k))
                .child(setting_row(
                    k,
                    "Scrolling speed",
                    "Slow down for pages that need time to load.",
                    speed,
                )),
        ))
}

fn window_background(s: &SettingsUi, k: &Kit) -> El {
    let style = &s.settings.window_capture;
    let choices = row()
        .w_full()
        .gap(4.0)
        .children(
            BACKGROUNDS
                .iter()
                .enumerate()
                .map(|(index, &(key, title, background))| {
                    let selected = style.background == background;
                    tile(k, key, selected, 1.0, 110.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .label(title)
                        .on_press(move |s: &mut SettingsUi| {
                            s.change(move |s| s.window_capture.background = background)
                        })
                        .child(
                            el().size(96.0, 66.0)
                                .radius(6.0)
                                .padding(2.0)
                                .border(
                                    2.0,
                                    if selected {
                                        k.c.bg_accent
                                    } else {
                                        k.c.border_base
                                    },
                                )
                                .child(
                                    el().w_full()
                                        .h_full()
                                        .radius(4.0)
                                        .pixels_fit(&s.previews[index], SurfaceFit::Fit),
                                ),
                        )
                        .child(
                            text(title)
                                .font(k.ty.text_xsmall)
                                .color(k.c.text_base)
                                .truncate(),
                        )
                }),
        );
    let swatches = row()
        .center_y()
        .children(SWATCHES.iter().enumerate().map(|(index, &color)| {
            el().key(format!("swatch-{index}"))
                .role("radio")
                .label(super::color_hex(color))
                .checked(style.color == color)
                .size(28.0, 28.0)
                .radius(14.0)
                .border(
                    2.0,
                    if style.color == color {
                        k.c.bg_accent
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .center()
                .cursor(Cursor::Pointer)
                .on_press(move |s: &mut SettingsUi| s.set_background_color(color))
                .child(
                    el().size(20.0, 20.0)
                        .radius(10.0)
                        .border(1.0, k.c.border_base)
                        .fill(rgb(u32::from_be_bytes([0, color[0], color[1], color[2]]))),
                )
        }));
    let color_field = TextInput::new(&s.background_color)
        .label("Background color")
        .width(Some(96.0))
        .invalid(s.field_error(Field::BackgroundColor).is_some())
        .on_edit(|s: &mut SettingsUi| {
            s.wake();
            &mut s.background_color
        })
        .on_submit(|s: &mut SettingsUi, _| s.save_background_color())
        .build(k)
        .key("window-background-color");
    let wallpaper = style.background == WindowBackground::Wallpaper;
    let mut card = card(k)
        .child(col().padding(12.0).child(choices))
        .child(col().px(16.0).pb(12.0).child(note(
            k,
            "Applied to window captures. Padded modes add a soft shadow. Trim shadow keeps the window's exact size.",
        )));
    if style.background != WindowBackground::TrimShadow {
        let padding = s.slider_value(Slider::Padding);
        card = card.child(rule(k)).child(setting_row(
            k,
            "Padding",
            "Space around the window at 1× resolution.",
            row()
                .center_y()
                .gap(12.0)
                .child(slider(
                    s,
                    k,
                    Slider::Padding,
                    "window-padding",
                    "Padding",
                    150.0,
                ))
                .child(
                    text(format!("{padding} pt"))
                        .w(44.0)
                        .font(k.ty.ui)
                        .color(k.c.text_base),
                ),
        ));
    }
    if matches!(
        style.background,
        WindowBackground::Solid | WindowBackground::Wallpaper
    ) {
        card = card.child(rule(k)).child(setting_row_with_error(
            k,
            if wallpaper {
                "Fallback color"
            } else {
                "Background color"
            },
            if wallpaper {
                "Used if the wallpaper image is unavailable."
            } else {
                "A solid color behind the window and its shadow."
            },
            s.field_error(Field::BackgroundColor),
            row().center_y().gap(8.0).child(swatches).child(color_field),
        ));
    }
    if wallpaper {
        let path = style.wallpaper.as_ref().map_or_else(
            || "Desktop wallpaper".into(),
            |path| path.display().to_string(),
        );
        card = card
            .child(rule(k))
            .child(setting_row_with_error(
                k,
                "Wallpaper image",
                "Use your desktop wallpaper or choose an image.",
                s.field_error(Field::Wallpaper),
                row()
                    .gap(8.0)
                    .child(
                        Button::new("Use desktop")
                            .variant(ButtonVariant::Ghost)
                            .disabled(style.wallpaper.is_none())
                            .on_press(|s: &mut SettingsUi| {
                                s.change(|s| s.window_capture.wallpaper = None)
                            })
                            .build(k)
                            .key("desktop-wallpaper"),
                    )
                    .child(
                        Button::new("Choose…")
                            .on_press_tx(|s: &mut SettingsUi, tx: &mut Tx| s.choose_wallpaper(tx))
                            .build(k)
                            .key("choose-wallpaper"),
                    ),
            ))
            .child(path_line(k, path));
    }
    section(k, "Window screenshot background", card)
}

fn shortcuts(s: &SettingsUi, k: &Kit) -> El {
    let recording = s.recorders.iter().any(ShortcutRecorderState::is_recording);
    let mut page = page();
    if let Some(error) = s.snapshot.hotkey_error.as_ref().filter(|_| !recording) {
        page = page.child(
            Card::new()
                .accent(k.c.danger.icon)
                .description(error.clone())
                .build(k)
                .key("hotkey-error")
                .role("alert")
                .fill(k.c.danger.bg),
        );
    }
    for (group, items) in SHORTCUT_GROUPS {
        let mut rows = card(k);
        for (position, &(index, label)) in items.iter().enumerate() {
            if position > 0 {
                rows = rows.child(rule(k));
            }
            rows = rows.child(setting_row(k, label, "", recorder(s, k, index)));
        }
        page = page.child(section(k, group, rows));
    }
    page.child(
        row()
            .justify_between()
            .center_y()
            .gap(16.0)
            .child(el().grow(1.0).shrink().min_w(0.0).child(note(
                k,
                "Click a shortcut, then press the new keys. Esc cancels. Backspace turns it off.",
            )))
            .child(
                Button::new("Restore all defaults")
                    .icon(&icons::ROTATE_CCW)
                    .on_press(|s: &mut SettingsUi| {
                        s.change(|s| s.hotkeys = HotkeySettings::default())
                    })
                    .build(k)
                    .key("reset-hotkeys"),
            ),
    )
}

/// A click-to-record shortcut field, with buttons to restore its default or turn it off.
fn recorder(s: &SettingsUi, k: &Kit, index: usize) -> El {
    let state = &s.recorders[index];
    let value = s.settings.hotkeys.slots()[index];
    let default = HotkeySettings::default().slots()[index].to_owned();
    let field = ShortcutRecorder::new(
        format!("hotkey-{index}"),
        state,
        move |s: &mut SettingsUi| {
            s.wake();
            &mut s.recorders[index]
        },
    )
    .label(shortcut_name(index))
    .width(230.0)
    .error_message(|error, shortcut| match error {
        ShortcutError::Backend(message) => message.clone(),
        _ => rune_kit::shortcut_error_message(error, shortcut),
    })
    .on_change(move |s: &mut SettingsUi, shortcut, _| {
        s.set_hotkey(index, shortcut.map(hotkey_text).unwrap_or_default())
            .map_err(ShortcutError::Backend)
    })
    .build(k);
    // Small icon buttons inside the field's trailing end.
    let action = |key: String, glyph: &'static rune::Icon, help: &'static str| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .help(help)
            .build(k)
            .key(key)
    };
    let mut actions = row().gap(2.0);
    if state.is_recording() {
        actions = actions.child(
            action(format!("hotkey-cancel-{index}"), &icons::X, "Cancel").on_press(
                move |s: &mut SettingsUi| {
                    s.recorders[index].cancel();
                    s.wake();
                },
            ),
        );
    } else {
        if value != default {
            actions = actions.child(
                action(
                    format!("hotkey-reset-{index}"),
                    &icons::ROTATE_CCW,
                    "Restore default",
                )
                .on_press(move |s: &mut SettingsUi| s.press_hotkey(index, default.clone())),
            );
        }
        if !value.is_empty() {
            actions = actions.child(
                action(format!("hotkey-clear-{index}"), &icons::X, "Turn off")
                    .on_press(move |s: &mut SettingsUi| s.press_hotkey(index, String::new())),
            );
        }
    }
    el().child(field)
        .child(actions.pin(None, Some(4.0), Some(4.0), None))
}

fn share(s: &SettingsUi, k: &Kit) -> El {
    let settings = &s.settings;
    let directory = settings
        .screenshot_directory
        .as_ref()
        .or(s.default_directory.as_ref())
        .map_or_else(
            || "Choose a folder".into(),
            |path| path.display().to_string(),
        );
    let formats = [ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::Webp];
    let format = SegmentedControl::new(
        &["PNG", "JPEG", "WebP"],
        formats.iter().position(|format| *format == settings.format),
    )
    .label("Save format")
    .fit()
    .on_pick(move |s: &mut SettingsUi, index, _| {
        let format = formats[index];
        s.change(move |s| s.format = format)
    })
    .build(k)
    .key("save-format");
    let folder = row()
        .center_y()
        .gap(8.0)
        .child(
            Button::new("Choose…")
                .on_press_tx(|s: &mut SettingsUi, tx: &mut Tx| s.choose_folder(tx))
                .build(k)
                .key("choose-screenshot-folder"),
        )
        .child(
            IconButton::new(&icons::ROTATE_CCW)
                .variant(IconButtonVariant::Ghost)
                .help("Use the default folder")
                .on_press(|s: &mut SettingsUi| s.change(|s| s.screenshot_directory = None))
                .build(k)
                .key("reset-screenshot-folder"),
        );
    page()
        .child(section(
            k,
            "Saving",
            card(k)
                .child(setting_row(
                    k,
                    "Screenshots folder",
                    "The starting folder when you save a screenshot.",
                    folder,
                ))
                .child(path_line(k, directory))
                .child(rule(k))
                .child(setting_row(
                    k,
                    "Save format",
                    "The default image format.",
                    format,
                ))
                .child(rule(k))
                .child(toggle(
                    k,
                    "downscale-on-save",
                    "Save Retina screenshots at 1×",
                    "The editor and clipboard keep full resolution.",
                    settings.downscale_on_save,
                    |s, on| s.downscale_on_save = on,
                )),
        ))
        .child(upload_section(s, k))
}

fn upload_section(s: &SettingsUi, k: &Kit) -> El {
    let configured = s.settings.cloud_upload.is_some();
    let status = if configured {
        "On. Screenshots upload only when you choose Upload."
    } else if s.upload_open {
        "Save a destination below to turn it on."
    } else {
        "Off. Turn on to set up a destination."
    };
    let enabled = Switch::new(s.upload_open)
        .label("Upload to the cloud")
        .hide_label()
        .on_toggle(|s: &mut SettingsUi| s.set_upload_open(!s.upload_open))
        .build(k)
        .key("upload-enabled");
    let mut upload = card(k).child(setting_row(k, "Upload to the cloud", status, enabled));
    if !s.upload_open {
        return section(k, "Upload", upload);
    }
    let kinds = [UploadKind::SignedUrl, UploadKind::S3];
    let kind = SegmentedControl::new(
        &["Signed URL", "S3 compatible"],
        kinds.iter().position(|kind| *kind == s.upload_kind),
    )
    .label("Destination")
    .fit()
    .on_pick(move |s: &mut SettingsUi, index, _| {
        s.upload_kind = kinds[index];
        s.clear_notice();
    })
    .build(k)
    .key("upload-kind");
    let fields = match s.upload_kind {
        UploadKind::SignedUrl => 0..2,
        UploadKind::S3 => 2..7,
    };
    upload = upload
        .child(rule(k))
        .child(setting_row(k, "Destination", "", kind))
        .child(rule(k));
    for index in fields {
        let (label, placeholder) = UPLOAD_FIELDS[index];
        upload = upload.child(
            col().px(16.0).py(12.0).child(
                KitField::new()
                    .label(label)
                    .control(
                        TextInput::new(&s.upload[index])
                            .label(label)
                            .placeholder(placeholder)
                            .width(None)
                            .on_edit(move |s: &mut SettingsUi| &mut s.upload[index])
                            .on_submit(|s: &mut SettingsUi, _| s.save_upload())
                            .build(k)
                            .key(format!("upload-input-{index}")),
                    )
                    .build(k),
            ),
        );
    }
    upload = match s.upload_kind {
        UploadKind::SignedUrl => upload.child(col().px(16.0).pb(12.0).child(note(
            k,
            "Paste a signed upload URL from your provider. Each signed URL uploads one image, so paste a new one for the next upload.",
        ))),
        UploadKind::S3 => {
            let (glyph, ink, status, description) = if sniplet_platform::s3_credentials_available() {
                (
                    &icons::CIRCLE_CHECK,
                    k.c.success.fg,
                    "Found",
                    "Read from AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY. AWS_SESSION_TOKEN is optional.",
                )
            } else {
                (
                    &icons::CIRCLE_ALERT,
                    k.c.danger.fg,
                    "Not found",
                    "Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, then restart Sniplet.",
                )
            };
            upload.child(rule(k)).child(setting_row(
                k,
                "AWS credentials",
                description,
                row()
                    .key("aws-credentials")
                    .center_y()
                    .gap(4.0)
                    .child(icon(glyph, 14.0).color(ink))
                    .child(text(status).font(k.ty.text_xsmall_medium).color(ink)),
            ))
        }
    };
    let message = match s.field_error(Field::Upload) {
        Some(error) => text(error.to_owned())
            .key("upload-error")
            .role("alert")
            .font(k.ty.text_xsmall)
            .color(k.c.danger.fg)
            .wrap(),
        None => note(
            k,
            if configured {
                "Save again after you change a field."
            } else {
                "Upload stays off until you save a destination."
            },
        ),
    };
    upload = upload.child(Divider::horizontal().build(k)).child(
        row()
            .px(16.0)
            .py(12.0)
            .fill(k.c.layer_02)
            .justify_between()
            .center_y()
            .gap(12.0)
            .child(el().grow(1.0).shrink().min_w(0.0).child(message))
            .child(
                Button::new("Save destination")
                    .variant(ButtonVariant::Contrast)
                    .on_press(|s: &mut SettingsUi| s.save_upload())
                    .build(k)
                    .key("save-upload"),
            ),
    );
    section(k, "Upload", upload)
}

impl SettingsUi {
    /// A slider's value: the dragged one while it moves, else the saved one.
    fn slider_value(&self, slider: Slider) -> u32 {
        match self.sliding {
            Some((which, value)) if which == slider => value,
            _ => slider.value(&self.settings),
        }
    }
}

/// A horizontal slider over `slider`'s range. The value follows the drag and saves on release;
/// arrow keys step it.
fn slider(s: &SettingsUi, k: &Kit, which: Slider, key: &str, label: &str, width: f32) -> El {
    let (min, max, step) = which.range();
    let value = s.slider_value(which);
    let (height, knob, bar) = (24.0, 16.0, 4.0);
    let travel = width - knob;
    let fraction = (value - min) as f32 / (max - min) as f32;
    let at = move |x: f32| {
        let raw = ((x - knob / 2.0) / travel).clamp(0.0, 1.0) * (max - min) as f32;
        min + ((raw / step as f32).round() as u32 * step).min(max - min)
    };
    el().key(key)
        .role("slider")
        .label(label)
        .range_value(f64::from(value), f64::from(min), f64::from(max))
        .range_step(f64::from(step))
        .on_step(move |s: &mut SettingsUi, steps, _| {
            let value = (which.value(&s.settings) as i64 + i64::from(steps) * i64::from(step))
                .clamp(i64::from(min), i64::from(max)) as u32;
            s.change(move |s| which.apply(s, value));
        })
        .size(width, height)
        .cursor(Cursor::Pointer)
        .on_drag(move |s: &mut SettingsUi, drag: &mut Drag, tx: &mut Tx| {
            let value = at(drag.x);
            if drag.phase == Phase::Up {
                s.sliding = None;
                if !drag.cancelled() {
                    s.change(move |s| which.apply(s, value));
                }
            } else {
                s.sliding = Some((which, value));
                tx.instant();
            }
        })
        .child(
            el().size(width, bar)
                .absolute(0.0, (height - bar) / 2.0)
                .radius(bar / 2.0)
                .fill(k.c.layer_03)
                .a11y_hidden(),
        )
        .child(
            el().size(knob / 2.0 + fraction * travel, bar)
                .absolute(0.0, (height - bar) / 2.0)
                .radius(bar / 2.0)
                .fill(k.c.bg_accent)
                .a11y_hidden(),
        )
        .child(
            // The kit's raised knob, as its switches draw theirs.
            k.elevate(
                el().size(knob, knob)
                    .absolute(fraction * travel, (height - knob) / 2.0)
                    .radius(knob / 2.0)
                    .fill(k.c.knob),
                &k.elevation.knob,
            )
            .border(0.5, k.c.border_strong)
            .a11y_hidden(),
        )
}

/// A row with a switch that saves `set` with the new value.
fn toggle(
    k: &Kit,
    key: &str,
    title: &str,
    description: &str,
    checked: bool,
    set: fn(&mut Settings, bool),
) -> El {
    let switch = Switch::new(checked)
        .label(title)
        .hide_label()
        .on_toggle(move |s: &mut SettingsUi| s.change(move |settings| set(settings, !checked)))
        .build(k)
        .key(key);
    setting_row(k, title, description, switch)
}

/// A pressable choice tile, filled while it is the current choice.
fn tile(k: &Kit, key: &str, selected: bool, width: f32, height: f32) -> El {
    let tile = col()
        .key(key)
        .role("radio")
        .checked(selected)
        .size(width, height)
        .radius(k.radius.lg)
        .center()
        .gap(8.0)
        .cursor(Cursor::Pointer);
    if selected {
        tile.fill(k.c.pressed)
    } else {
        k.interactive(tile)
    }
}

/// A muted path under a row, cut short when it does not fit.
fn path_line(k: &Kit, path: String) -> El {
    col().px(16.0).pb(12.0).child(
        text(path)
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted)
            .truncate(),
    )
}

/// One column of sections with the page's spacing.
fn page() -> El {
    col().w_full().gap(20.0)
}

/// Muted help text that wraps.
pub(crate) fn note(k: &Kit, value: impl Into<String>) -> El {
    text(value.into())
        .font(k.ty.text_xsmall)
        .color(k.c.text_muted)
        .wrap()
}

/// A rounded group of rows on the kit's first layer.
pub(crate) fn card(k: &Kit) -> El {
    k.elevate(
        col()
            .w_full()
            .radius(k.radius.xl)
            .fill(k.c.layer_01)
            .clip_rounded(k.radius.xl),
        &k.elevation.raised,
    )
}

/// A titled group: a small muted heading over its content.
pub(crate) fn section(k: &Kit, title: &str, content: El) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(
            text(title)
                .px(4.0)
                .font(k.ty.text_xsmall_medium)
                .color(k.c.text_muted),
        )
        .child(content)
}

/// The hairline between rows of a card.
pub(crate) fn rule(k: &Kit) -> El {
    Divider::horizontal().build(k).mx(16.0)
}

/// A settings row: a title and description on the left, the control on the right.
pub(crate) fn setting_row(k: &Kit, title: &str, description: &str, control: El) -> El {
    setting_row_with_error(k, title, description, None, control)
}

/// A settings row whose validation error shows under its title, next to the control it is
/// about.
fn setting_row_with_error(
    k: &Kit,
    title: &str,
    description: &str,
    error: Option<&str>,
    control: El,
) -> El {
    let mut label = col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(2.0)
        .child(text(title).font(k.ty.ui).color(k.c.text_base).wrap());
    if !description.is_empty() {
        label = label.child(note(k, description));
    }
    if let Some(error) = error {
        label = label.child(
            text(error.to_owned())
                .role("alert")
                .font(k.ty.text_xsmall)
                .color(k.c.danger.fg)
                .wrap(),
        );
    }
    row()
        .w_full()
        .padding(16.0)
        .gap(16.0)
        .center_y()
        .justify_between()
        .child(label)
        .child(control)
}

/// Draw the four window background thumbnails for `color` into `previews`.
pub(super) fn render_previews(previews: &[Pixels; 4], color: [u8; 3]) {
    use image::{Rgba, RgbaImage, imageops};
    let mut source = RgbaImage::from_fn(88, 56, |x, y| {
        if !(4..84).contains(&x) && !(4..52).contains(&y) {
            Rgba([0, 0, 0, 0])
        } else if y < 16 {
            Rgba([63, 66, 70, 255])
        } else {
            Rgba([38, 40, 43, 255])
        }
    });
    for (center, light) in [
        (10_i32, [255, 95, 87, 255]),
        (20, [254, 188, 46, 255]),
        (30, [40, 200, 64, 255]),
    ] {
        for y in 5..11 {
            for x in center - 3..center + 3 {
                if (x - center).pow(2) + (y - 8_i32).pow(2) <= 9 {
                    source.put_pixel(x as u32, y as u32, Rgba(light));
                }
            }
        }
    }
    let wallpaper = RgbaImage::from_fn(112, 80, |x, y| {
        Rgba([
            80 + (x / 3) as u8,
            125 + (y / 3) as u8,
            185 + (x / 6) as u8,
            255,
        ])
    });
    for (pixels, (_, _, background)) in previews.iter().zip(BACKGROUNDS) {
        let style = WindowCaptureStyle {
            background,
            padding: 12,
            color,
            wallpaper: None,
        };
        let preview =
            sniplet_platform::compose_window_capture(source.clone(), &style, 1.0, Some(&wallpaper));
        // The checkerboard is only a thumbnail aid, never part of the capture.
        let mut thumbnail = RgbaImage::from_fn(preview.width(), preview.height(), |x, y| {
            let value = if (x / 7 + y / 7) % 2 == 0 { 68 } else { 58 };
            Rgba([value, value, value, 255])
        });
        imageops::overlay(&mut thumbnail, &preview, 0, 0);
        pixels.set(thumbnail.width(), thumbnail.height(), thumbnail.into_raw());
    }
}

/// A small picture of a window in one appearance. System shows a dark corner.
fn appearance_preview(preference: ThemePreference) -> El {
    let dark = preference == ThemePreference::Dark;
    let background = rgb(if dark { 0x24272c } else { 0xf1f2f5 });
    let sidebar = rgb(if dark { 0x353940 } else { 0xe0e3e9 });
    let dots = [0xff6058, 0xfebb2e, 0x28c840]
        .map(|color| el().size(4.0, 4.0).radius(2.0).fill(rgb(color)));
    let mut preview = col()
        .size(100.0, 58.0)
        .radius(6.0)
        .border(1.0, rgb(0x8a8e96))
        .fill(background)
        .clip_rounded(6.0)
        .child(
            row()
                .w_full()
                .h(13.0)
                .px(5.0)
                .gap(3.0)
                .center_y()
                .fill(sidebar)
                .children(dots),
        )
        .child(
            row()
                .w_full()
                .grow(1.0)
                .child(el().w(28.0).h_full().fill(sidebar))
                .child(
                    col()
                        .grow(1.0)
                        .padding(7.0)
                        .gap(5.0)
                        .child(el().w_full().h(5.0).radius(2.0).fill(rgb(0x2684ff)))
                        .child(el().w(30.0).h(4.0).radius(2.0).fill(sidebar)),
                ),
        );
    if preference == ThemePreference::System {
        preview = preview.child(
            el().size(24.0, 30.0)
                .pin(None, None, Some(0.0), Some(0.0))
                .fill(rgb(0x24272c)),
        );
    }
    preview.a11y_hidden()
}
