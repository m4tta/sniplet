//! The Settings window's view: the title band, four tabs and the selected page. The card,
//! section and row helpers are shared with the Updates section in `updates.rs`.
use rune::{
    Color, Cursor, Cx, Drag, El, Phase, Pixels, ScrollSpec, ShortcutError, SurfaceFit, TextEdit,
    TextStyle, Tx, col, el, icon, row, text,
};
use rune_kit::{Kit, SegmentedControl, ShortcutRecorder, ShortcutRecorderState, Switch, TextInput};
use sniplet_platform::{
    ExportFormat, HotkeySettings, Settings, ThemePreference, WindowBackground, WindowCaptureStyle,
};

use super::{
    Field, Page, SHORTCUT_GROUPS, SettingsUi, Slider, UploadKind, hotkey_text, shortcut_name,
    step_limit,
};
use crate::ui::{BODY, Button, Palette, SEMIBOLD, SMALL, icons, rgb};

const SMALL_SEMIBOLD: TextStyle = TextStyle::new(12.0, 600.0);

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

const UPLOAD_PLACEHOLDERS: [&str; 7] = [
    "https://… signed PUT URL",
    "https://… public image link",
    "screenshots",
    "us-east-1",
    "Leave empty for Amazon S3",
    "sniplet/",
    "https://… public base URL",
];

pub(super) fn view(s: &SettingsUi, cx: &mut Cx<SettingsUi>) -> El {
    let p = Palette::for_preference(s.settings.theme, cx.system_dark());
    cx.caption_color(p.foreground);
    if s.snapshot.installing {
        return crate::updates::installing_screen(&p);
    }
    let k = kit(&p);
    let page = match s.page {
        Page::General => general(s, &p, &k),
        Page::Capture => capture(s, &p, &k),
        Page::Shortcuts => shortcuts(s, &p, &k),
        Page::Share => share(s, &p, &k),
    };
    let insets = cx.chrome_insets();
    let mut root = col()
        .key("settings-window")
        .w_full()
        .h_full()
        .fill(p.background);
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
                .child(text("Settings").font(SEMIBOLD).color(p.foreground)),
        );
    }
    root.child(tabs(s, &p))
        .child(
            col()
                .key(format!("settings-content-{}", s.page as usize))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(col().w_full().padding(24.0).child(page)),
        )
        .children(toast(s, &p))
}

/// Kit components in Sniplet's colors, with white switch knobs in both appearances.
fn kit(p: &Palette) -> Kit {
    let mut kit = p.kit();
    kit.c.knob = Color::WHITE;
    kit.c.knob_on = Color::WHITE;
    kit
}

fn tabs(s: &SettingsUi, p: &Palette) -> El {
    let mut tabs = row()
        .key("settings-tabs")
        .w_full()
        .padding(8.0)
        .gap(4.0)
        .justify_center()
        .fill(p.title_bar);
    for (index, page) in Page::ALL.into_iter().enumerate() {
        let selected = s.page == page;
        let ink = if selected {
            p.foreground
        } else {
            p.muted_foreground
        };
        tabs = tabs.child(
            col()
                .key(format!("settings-nav-{index}"))
                .role("tab")
                .label(page.title())
                .checked(selected)
                .size(96.0, 56.0)
                .radius(6.0)
                .center()
                .gap(4.0)
                .fill(if selected {
                    p.foreground.alpha(0.1)
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.foreground.alpha(0.06))
                .cursor(Cursor::Pointer)
                .on_press_tx(move |s: &mut SettingsUi, tx: &mut Tx| {
                    // Pages switch at once, as tabs do.
                    tx.instant();
                    s.select_page(page);
                })
                .child(icon(page.icon(), 20.0).color(if selected {
                    p.accent
                } else {
                    p.muted_foreground
                }))
                .child(
                    text(page.title())
                        .font(if selected { SMALL_SEMIBOLD } else { SMALL })
                        .color(ink)
                        .single_line(),
                ),
        );
    }
    col()
        .w_full()
        .child(tabs)
        .child(el().w_full().h(1.0).fill(p.border))
}

fn toast(s: &SettingsUi, p: &Palette) -> Option<El> {
    let notice = s.notice.as_ref().filter(|notice| notice.field.is_none())?;
    let ink = if notice.error { p.danger } else { p.background };
    let mut toast = row()
        .key("settings-toast")
        .role("status")
        .pin(None, None, Some(16.0), Some(16.0))
        .max_w(420.0)
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .gap(8.0)
        .center_y()
        .shadow(4.0, 16.0, Color::BLACK.alpha(0.2));
    toast = if notice.error {
        toast
            .fill(p.background)
            .border(1.0, p.danger)
            .cursor(Cursor::Pointer)
            .on_press(|s: &mut SettingsUi| s.clear_notice())
    } else {
        toast.fill(p.foreground)
    };
    Some(
        toast
            .child(
                icon(
                    if notice.error {
                        &icons::CIRCLE_ALERT
                    } else {
                        &icons::CHECK
                    },
                    14.0,
                )
                .color(ink),
            )
            .child(
                text(notice.text.clone())
                    .font(SMALL)
                    .color(ink)
                    .shrink()
                    .wrap(),
            ),
    )
}

fn general(s: &SettingsUi, p: &Palette, k: &Kit) -> El {
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
                tile(p, key, s.settings.theme == preference, 132.0, 106.0)
                    .label(label)
                    .on_press(move |s: &mut SettingsUi| s.change(move |s| s.theme = preference))
                    .child(appearance_preview(preference))
                    .child(text(label).font(BODY).color(p.foreground))
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
                .font(SMALL)
                .color(p.muted_foreground),
        )
        .child(text("·").font(SMALL).color(p.muted_foreground))
        .child(
            Button::new("reveal-settings")
                .label("Show settings file")
                .compact()
                .build(p, |_: &mut SettingsUi, _| {
                    if let Ok(store) = sniplet_platform::SettingsStore::for_app() {
                        super::reveal(store.path());
                    }
                }),
        );
    page()
        .child(section(
            p,
            "Appearance",
            card(p).padding(12.0).child(appearance),
        ))
        .child(section(
            p,
            "App",
            card(p)
                .child(setting_row_with_error(
                    p,
                    "Launch at startup",
                    "Start in the tray when you sign in. The editor stays closed.",
                    s.field_error(Field::Startup),
                    startup,
                ))
                .child(rule(p))
                .child(toggle(
                    p,
                    k,
                    "always-on-top",
                    "Keep editor on top",
                    "Takes effect after restarting Sniplet.",
                    s.settings.always_on_top,
                    |s, on| s.always_on_top = on,
                )),
        ))
        .child(crate::updates::settings_section::<SettingsUi>(
            p,
            &s.snapshot.updates,
            &s.editor,
        ))
        .child(about)
}

fn capture(s: &SettingsUi, p: &Palette, k: &Kit) -> El {
    let limit = s.settings.scroll_max_frames;
    let step = |key: &'static str, glyph, help: &'static str, disabled: bool, up: bool| {
        Button::new(key)
            .icon(glyph)
            .help(help)
            .size(28.0, 28.0)
            .disabled(disabled)
            .build(p, move |s: &mut SettingsUi, _| {
                s.change(move |s| s.scroll_max_frames = step_limit(limit, up))
            })
    };
    let stepper = row()
        .h(32.0)
        .px(2.0)
        .center_y()
        .radius(6.0)
        .border(1.0, p.border)
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
                .font(BODY)
                .color(p.foreground)
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
            p,
            Slider::Speed,
            "scroll-speed",
            "Scrolling speed",
            200.0,
        ))
        .child(
            row()
                .justify_between()
                .child(text("Slower").font(SMALL).color(p.muted_foreground))
                .child(text("Faster").font(SMALL).color(p.muted_foreground)),
        );
    page()
        .child(window_background(s, p))
        .child(section(
            p,
            "After capture",
            card(p)
                .child(toggle(
                    p,
                    k,
                    "auto-copy",
                    "Copy captures automatically",
                    "A fresh capture is ready to paste.",
                    s.settings.auto_copy,
                    |s, on| s.auto_copy = on,
                ))
                .child(rule(p))
                .child(toggle(
                    p,
                    k,
                    "hide-after-export",
                    "Hide editor after copy or save",
                    "Return to the app you were working in.",
                    s.settings.hide_after_export,
                    |s, on| s.hide_after_export = on,
                )),
        ))
        .child(section(
            p,
            "Scrolling capture",
            card(p)
                .child(setting_row(
                    p,
                    "Capture limit",
                    "Most frames in one scrolling capture (2–200).",
                    stepper,
                ))
                .child(rule(p))
                .child(setting_row(
                    p,
                    "Scrolling speed",
                    "Slow down for pages that need time to load.",
                    speed,
                )),
        ))
}

fn window_background(s: &SettingsUi, p: &Palette) -> El {
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
                    tile(p, key, selected, 1.0, 110.0)
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
                                .border(2.0, if selected { p.accent } else { p.border })
                                .child(
                                    el().w_full()
                                        .h_full()
                                        .radius(4.0)
                                        .pixels_fit(&s.previews[index], SurfaceFit::Fit),
                                ),
                        )
                        .child(text(title).font(SMALL).color(p.foreground).truncate())
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
                        p.accent
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
                        .border(1.0, p.border)
                        .fill(rgb(color_value(color))),
                )
        }));
    let color_field = TextInput::new(&s.background_color)
        .label("Background color")
        .width(Some(96.0))
        .invalid(s.field_error(Field::BackgroundColor).is_some())
        .on_edit(color_access)
        .on_submit(|s: &mut SettingsUi, _| s.save_background_color())
        .build(&kit(p))
        .key("window-background-color");
    let wallpaper = style.background == WindowBackground::Wallpaper;
    let mut card = card(p)
        .child(col().padding(12.0).child(choices))
        .child(col().px(16.0).pb(12.0).child(note(
            p,
            "Applied to window captures. Padded modes add a soft shadow. Trim shadow keeps the window's exact size.",
        )));
    if style.background != WindowBackground::TrimShadow {
        let padding = s.slider_value(Slider::Padding);
        card = card.child(rule(p)).child(setting_row(
            p,
            "Padding",
            "Space around the window at 1× resolution.",
            row()
                .center_y()
                .gap(12.0)
                .child(slider(
                    s,
                    p,
                    Slider::Padding,
                    "window-padding",
                    "Padding",
                    150.0,
                ))
                .child(
                    text(format!("{padding} pt"))
                        .w(44.0)
                        .font(BODY)
                        .color(p.foreground),
                ),
        ));
    }
    if matches!(
        style.background,
        WindowBackground::Solid | WindowBackground::Wallpaper
    ) {
        card = card.child(rule(p)).child(setting_row_with_error(
            p,
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
            .child(rule(p))
            .child(setting_row_with_error(
                p,
                "Wallpaper image",
                "Use your desktop wallpaper or choose an image.",
                s.field_error(Field::Wallpaper),
                row()
                    .gap(8.0)
                    .child(
                        Button::new("desktop-wallpaper")
                            .label("Use desktop")
                            .disabled(style.wallpaper.is_none())
                            .build(p, |s: &mut SettingsUi, _| {
                                s.change(|s| s.window_capture.wallpaper = None)
                            }),
                    )
                    .child(outlined(
                        p,
                        Button::new("choose-wallpaper")
                            .label("Choose…")
                            .build(p, |s: &mut SettingsUi, tx: &mut Tx| s.choose_wallpaper(tx)),
                    )),
            ))
            .child(path_line(p, path));
    }
    section(p, "Window screenshot background", card)
}

fn shortcuts(s: &SettingsUi, p: &Palette, k: &Kit) -> El {
    let recording = s.recorders.iter().any(ShortcutRecorderState::is_recording);
    let mut page = page();
    if let Some(error) = s.snapshot.hotkey_error.as_ref().filter(|_| !recording) {
        page = page.child(
            el().key("hotkey-error")
                .role("alert")
                .padding(12.0)
                .radius(12.0)
                .border(1.0, p.danger)
                .child(text(error.clone()).font(SMALL).color(p.danger).wrap()),
        );
    }
    for (group, items) in SHORTCUT_GROUPS {
        let mut rows = card(p);
        for (position, &(index, label)) in items.iter().enumerate() {
            if position > 0 {
                rows = rows.child(rule(p));
            }
            rows = rows.child(setting_row(p, label, "", recorder(s, p, k, index)));
        }
        page = page.child(section(p, group, rows));
    }
    page.child(
        row()
            .justify_between()
            .center_y()
            .gap(16.0)
            .child(el().grow(1.0).shrink().min_w(0.0).child(note(
                p,
                "Click a shortcut, then press the new keys. Esc cancels. Backspace turns it off.",
            )))
            .child(
                Button::new("reset-hotkeys")
                    .icon(&icons::ROTATE_CCW)
                    .label("Restore all defaults")
                    .build(p, |s: &mut SettingsUi, _| {
                        s.change(|s| s.hotkeys = HotkeySettings::default())
                    }),
            ),
    )
}

/// A click-to-record shortcut field, with buttons to restore its default or turn it off.
fn recorder(s: &SettingsUi, p: &Palette, k: &Kit, index: usize) -> El {
    let state = &s.recorders[index];
    let value = s.settings.hotkeys.slots()[index];
    let default = HotkeySettings::default().slots()[index].to_owned();
    let field = ShortcutRecorder::new(format!("hotkey-{index}"), state, recorder_access(index))
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
        row()
            .key(key)
            .role("button")
            .label(help)
            .help(help)
            .size(24.0, 24.0)
            .center()
            .radius(4.0)
            .hover(p.hover)
            .cursor(Cursor::Pointer)
            .child(icon(glyph, 14.0).color(p.muted_foreground))
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

fn recorder_access(
    index: usize,
) -> impl for<'a> Fn(&'a mut SettingsUi) -> &'a mut ShortcutRecorderState {
    move |s| {
        s.wake();
        &mut s.recorders[index]
    }
}

fn color_access(s: &mut SettingsUi) -> &mut TextEdit {
    s.wake();
    &mut s.background_color
}

fn upload_access(index: usize) -> impl for<'a> Fn(&'a mut SettingsUi) -> &'a mut TextEdit {
    move |s| &mut s.upload[index]
}

fn share(s: &SettingsUi, p: &Palette, k: &Kit) -> El {
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
        .child(outlined(
            p,
            Button::new("choose-screenshot-folder")
                .label("Choose…")
                .build(p, |s: &mut SettingsUi, tx: &mut Tx| s.choose_folder(tx)),
        ))
        .child(
            Button::new("reset-screenshot-folder")
                .icon(&icons::ROTATE_CCW)
                .help("Use the default folder")
                .build(p, |s: &mut SettingsUi, _| {
                    s.change(|s| s.screenshot_directory = None)
                }),
        );
    page()
        .child(section(
            p,
            "Saving",
            card(p)
                .child(setting_row(
                    p,
                    "Screenshots folder",
                    "The starting folder when you save a screenshot.",
                    folder,
                ))
                .child(path_line(p, directory))
                .child(rule(p))
                .child(setting_row(
                    p,
                    "Save format",
                    "The default image format.",
                    format,
                ))
                .child(rule(p))
                .child(toggle(
                    p,
                    k,
                    "downscale-on-save",
                    "Save Retina screenshots at 1×",
                    "The editor and clipboard keep full resolution.",
                    settings.downscale_on_save,
                    |s, on| s.downscale_on_save = on,
                )),
        ))
        .child(upload_section(s, p, k))
}

fn upload_section(s: &SettingsUi, p: &Palette, k: &Kit) -> El {
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
    let mut upload = card(p).child(setting_row(p, "Upload to the cloud", status, enabled));
    if !s.upload_open {
        return section(p, "Upload", upload);
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
    let fields: &[(usize, &str)] = match s.upload_kind {
        UploadKind::SignedUrl => &[(0, "Signed PUT URL"), (1, "Image link (optional)")],
        UploadKind::S3 => &[
            (2, "Bucket"),
            (3, "Region"),
            (4, "Endpoint (optional)"),
            (5, "Folder prefix (optional)"),
            (6, "Public base URL (optional)"),
        ],
    };
    upload = upload
        .child(rule(p))
        .child(setting_row(p, "Destination", "", kind))
        .child(rule(p));
    for &(index, label) in fields {
        upload = upload.child(
            col()
                .gap(8.0)
                .px(16.0)
                .py(12.0)
                .child(text(label).font(BODY).color(p.foreground))
                .child(
                    TextInput::new(&s.upload[index])
                        .label(label)
                        .placeholder(UPLOAD_PLACEHOLDERS[index])
                        .width(None)
                        .on_edit(upload_access(index))
                        .on_submit(|s: &mut SettingsUi, _| s.save_upload())
                        .build(k)
                        .key(format!("upload-input-{index}")),
                ),
        );
    }
    upload = match s.upload_kind {
        UploadKind::SignedUrl => upload.child(col().px(16.0).pb(12.0).child(note(
            p,
            "Paste a signed upload URL from your provider. Signed URLs expire, so paste a new one when uploads fail.",
        ))),
        UploadKind::S3 => {
            let found = sniplet_platform::s3_credentials_available();
            let ink = match (found, p.dark) {
                (true, true) => rgb(0x4ade80),
                (true, false) => rgb(0x16a34a),
                (false, _) => p.danger,
            };
            upload.child(rule(p)).child(setting_row(
                p,
                "AWS credentials",
                if found {
                    "Read from AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY. AWS_SESSION_TOKEN is optional."
                } else {
                    "Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, then restart Sniplet."
                },
                row()
                    .key("aws-credentials")
                    .center_y()
                    .gap(4.0)
                    .child(
                        icon(
                            if found {
                                &icons::CIRCLE_CHECK
                            } else {
                                &icons::CIRCLE_ALERT
                            },
                            14.0,
                        )
                        .color(ink),
                    )
                    .child(
                        text(if found { "Found" } else { "Not found" })
                            .font(SMALL)
                            .color(ink),
                    ),
            ))
        }
    };
    let message = match s.field_error(Field::Upload) {
        Some(error) => text(error.to_owned())
            .key("upload-error")
            .role("alert")
            .font(SMALL)
            .color(p.danger)
            .wrap(),
        None => note(
            p,
            if configured {
                "Save again after you change a field."
            } else {
                "Upload stays off until you save a destination."
            },
        ),
    };
    upload = upload.child(el().h(1.0).fill(p.border)).child(
        row()
            .px(16.0)
            .py(12.0)
            .fill(p.muted.alpha(0.4))
            .justify_between()
            .center_y()
            .gap(12.0)
            .child(el().grow(1.0).shrink().min_w(0.0).child(message))
            .child(
                Button::new("save-upload")
                    .label("Save destination")
                    .primary()
                    .build(p, |s: &mut SettingsUi, _| s.save_upload()),
            ),
    );
    section(p, "Upload", upload)
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
fn slider(s: &SettingsUi, p: &Palette, which: Slider, key: &str, label: &str, width: f32) -> El {
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
                .fill(p.muted)
                .a11y_hidden(),
        )
        .child(
            el().size(knob / 2.0 + fraction * travel, bar)
                .absolute(0.0, (height - bar) / 2.0)
                .radius(bar / 2.0)
                .fill(p.accent)
                .a11y_hidden(),
        )
        .child(
            el().size(knob, knob)
                .absolute(fraction * travel, (height - knob) / 2.0)
                .radius(knob / 2.0)
                .fill(Color::WHITE)
                .border(1.0, p.border)
                .shadow(1.0, 3.0, Color::BLACK.alpha(0.2))
                .a11y_hidden(),
        )
}

/// A row with a switch that saves `set` with the new value.
fn toggle(
    p: &Palette,
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
    setting_row(p, title, description, switch)
}

/// A pressable choice tile, filled while it is the current choice.
fn tile(p: &Palette, key: &str, selected: bool, width: f32, height: f32) -> El {
    col()
        .key(key)
        .role("radio")
        .checked(selected)
        .size(width, height)
        .radius(6.0)
        .center()
        .gap(8.0)
        .fill(if selected {
            p.hover
        } else {
            Color::TRANSPARENT
        })
        .hover(p.hover)
        .cursor(Cursor::Pointer)
}

/// A secondary button: Sniplet's ghost button with a border.
fn outlined(p: &Palette, button: El) -> El {
    button.border(1.0, p.border)
}

/// A muted path under a row, cut short when it does not fit.
fn path_line(p: &Palette, path: String) -> El {
    col()
        .px(16.0)
        .pb(12.0)
        .child(text(path).font(SMALL).color(p.muted_foreground).truncate())
}

/// One column of sections with the page's spacing.
fn page() -> El {
    col().w_full().gap(20.0)
}

/// Muted help text that wraps.
pub(crate) fn note(p: &Palette, value: impl Into<String>) -> El {
    text(value.into())
        .font(SMALL)
        .color(p.muted_foreground)
        .wrap()
}

/// A bordered, rounded group of rows.
pub(crate) fn card(p: &Palette) -> El {
    col()
        .w_full()
        .radius(12.0)
        .border(1.0, p.border)
        .fill(p.background)
        .clip_rounded(12.0)
}

/// A titled group: a small muted heading over its content.
pub(crate) fn section(p: &Palette, title: &str, content: El) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(
            text(title)
                .px(4.0)
                .font(SMALL_SEMIBOLD)
                .color(p.muted_foreground),
        )
        .child(content)
}

/// The hairline between rows of a card.
pub(crate) fn rule(p: &Palette) -> El {
    el().mx(16.0).h(1.0).fill(p.border)
}

/// A settings row: a title and description on the left, the control on the right.
pub(crate) fn setting_row(p: &Palette, title: &str, description: &str, control: El) -> El {
    setting_row_with_error(p, title, description, None, control)
}

/// A settings row whose validation error shows under its title, next to the control it is
/// about.
fn setting_row_with_error(
    p: &Palette,
    title: &str,
    description: &str,
    error: Option<&str>,
    control: El,
) -> El {
    let mut label = col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(4.0)
        .child(text(title).font(BODY).color(p.foreground).wrap());
    if !description.is_empty() {
        label = label.child(note(p, description));
    }
    if let Some(error) = error {
        label = label.child(
            text(error.to_owned())
                .role("alert")
                .font(SMALL)
                .color(p.danger)
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

fn color_value(color: [u8; 3]) -> u32 {
    (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2])
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
