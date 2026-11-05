use std::time::Duration;

use gpui_kit::{
    base::Disableable,
    base::slider::{SliderEvent, SliderState},
    component::{
        ActiveTheme, Selectable, Sizable, TitleBar,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
        slider::Slider,
        switch::Switch,
    },
    prelude::*,
    *,
};
use sniplet_platform::{
    CloudUploadConfig, ExportFormat, HotkeySettings, Settings, ThemePreference, WindowBackground,
    WindowCaptureStyle,
};

use crate::editor::{Editor, tool_icon};

/// Settings pages in tab order. `page as usize` is part of the tab and content element IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    General,
    Capture,
    Shortcuts,
    Share,
}

impl Page {
    const ALL: [Self; 4] = [Self::General, Self::Capture, Self::Shortcuts, Self::Share];

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Capture => "Capture",
            Self::Shortcuts => "Shortcuts",
            Self::Share => "Save & Share",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => "settings",
            Self::Capture => "scan",
            Self::Shortcuts => "keyboard",
            Self::Share => "cloud-upload",
        }
    }
}

/// Shortcut rows by group. Each index points into `HotkeySettings::slots`.
const SHORTCUT_GROUPS: [(&str, &[(usize, &str)]); 3] = [
    (
        "Capture",
        &[
            (0, "Capture area"),
            (1, "Capture screen"),
            (2, "Capture window"),
            (3, "Scrolling capture"),
            (6, "Capture text / QR"),
        ],
    ),
    (
        "Repeat",
        &[(4, "Repeat last area"), (5, "Capture active window")],
    ),
    ("App", &[(7, "Reopen editor")]),
];

/// Preset window backgrounds. The hex field still accepts any color.
const SWATCHES: [[u8; 3]; 6] = [
    [0xF2, 0xF2, 0xF7],
    [0xFF, 0xFF, 0xFF],
    [0x1C, 0x1C, 0x1E],
    [0xE8, 0xF0, 0xFE],
    [0xFD, 0xF2, 0xE9],
    [0x0A, 0x84, 0xFF],
];

/// A control that owns a validation message. A notice without one shows as a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Startup,
    BackgroundColor,
    Wallpaper,
    Hotkey(usize),
    Upload,
}

struct Notice {
    text: String,
    error: bool,
    field: Option<Field>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UploadKind {
    SignedUrl,
    S3,
}

pub(crate) struct SettingsWindow {
    editor: Entity<Editor>,
    page: Page,
    focus: FocusHandle,
    /// The shortcut slot that is waiting for a key combination.
    recording: Option<usize>,
    live_modifiers: Modifiers,
    upload: [Entity<InputState>; 7],
    upload_kind: UploadKind,
    /// Shows the destination form. It can be open before a destination is saved.
    upload_open: bool,
    scroll_speed: Entity<SliderState>,
    window_padding: Entity<SliderState>,
    background_color: Entity<InputState>,
    background_previews: [std::sync::Arc<RenderImage>; 4],
    notice: Option<Notice>,
    notice_generation: u64,
    settings_save_revision: u64,
    _subscriptions: Vec<Subscription>,
}

pub(crate) fn open(
    editor: Entity<Editor>,
    settings: Settings,
    page: Page,
    parent: &mut Window,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<SettingsWindow>)> {
    let mut options = TitleBar::window_options();
    #[cfg(target_os = "macos")]
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.traffic_light_position = Some(point(px(12.0), px(17.0)));
    }
    options.window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
        parent.display(cx).map(|display| display.id()),
        size(px(680.0), px(720.0)),
        cx,
    )));
    options.window_min_size = Some(size(px(560.0), px(480.0)));
    options.app_id = Some(sniplet_platform::APP_ID.into());
    gpui_kit::open_window(options, cx, move |window, cx| {
        window.set_window_title("Sniplet Settings");
        cx.new(|cx| SettingsWindow::new(editor, settings, page, window, cx))
    })
}

/// Text fields save when the user presses Enter or leaves the field.
fn commits(event: &InputEvent) -> bool {
    matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur)
}

impl SettingsWindow {
    fn new(
        editor: Entity<Editor>,
        settings: Settings,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut upload_values = std::array::from_fn::<_, 7, _>(|_| String::new());
        let mut upload_kind = UploadKind::SignedUrl;
        match settings.cloud_upload.clone() {
            Some(CloudUploadConfig::PresignedPut { url, public_url }) => {
                upload_values[0] = url;
                upload_values[1] = public_url.unwrap_or_default();
            }
            Some(CloudUploadConfig::S3 {
                bucket,
                region,
                endpoint,
                key_prefix,
                public_base_url,
            }) => {
                upload_kind = UploadKind::S3;
                upload_values[2] = bucket;
                upload_values[3] = region;
                upload_values[4] = endpoint.unwrap_or_default();
                upload_values[5] = key_prefix;
                upload_values[6] = public_base_url.unwrap_or_default();
            }
            None => upload_values[3] = "us-east-1".into(),
        }
        let placeholders = [
            "https://… signed PUT URL",
            "https://… public image link",
            "screenshots",
            "us-east-1",
            "Leave empty for Amazon S3",
            "sniplet/",
            "https://… public base URL",
        ];
        let upload = std::array::from_fn(|index| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(upload_values[index].clone())
                    .placeholder(placeholders[index])
            })
        });
        let scroll_speed = cx.new(|_| {
            SliderState::new()
                .min(100.0)
                .max(700.0)
                .step(50.0)
                .default_value((800 - settings.scroll_settle_ms.clamp(100, 700)) as f32)
        });
        let window_padding = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(120.0)
                .step(4.0)
                .default_value(settings.window_capture.padding.min(120) as f32)
        });
        let background_color = cx.new(|cx| {
            InputState::new(window, cx).default_value(color_hex(settings.window_capture.color))
        });
        let subscriptions = vec![
            cx.subscribe(&scroll_speed, |this, _, event, cx| {
                if let SliderEvent::Release(value) = event {
                    let milliseconds = 800 - value.end() as u64;
                    this.change(cx, |settings| settings.scroll_settle_ms = milliseconds);
                } else {
                    cx.notify();
                }
            }),
            cx.subscribe(&window_padding, |this, _, event, cx| {
                if let SliderEvent::Release(value) = event {
                    this.change(cx, |settings| {
                        settings.window_capture.padding = value.end() as u32
                    });
                } else {
                    cx.notify();
                }
            }),
            cx.subscribe(&background_color, |this, _, event, cx| {
                if commits(event) {
                    this.save_background_color(cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                let preference = this.editor.read(cx).settings.theme;
                if preference == ThemePreference::System {
                    crate::theme::apply(preference, window, cx);
                }
            }),
            // Leaving the window ends recording, so global shortcuts work again.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.stop_recording(cx);
                }
            }),
            cx.observe(&editor, |this: &mut Self, editor, cx| {
                if let Some((revision, error)) = editor.read(cx).settings_save_result.clone()
                    && revision != this.settings_save_revision
                {
                    this.settings_save_revision = revision;
                    let failed = error.is_some();
                    this.show(error.unwrap_or_else(|| "Saved".into()), failed, None, cx);
                }
                cx.notify();
            }),
            cx.on_release(|this, cx| {
                this.editor
                    .update(cx, |editor, _| editor.hotkeys_paused = false)
            }),
        ];
        Self {
            editor,
            page,
            focus,
            recording: None,
            live_modifiers: Modifiers::default(),
            upload,
            upload_kind,
            upload_open: settings.cloud_upload.is_some(),
            scroll_speed,
            window_padding,
            background_color,
            background_previews: background_previews(settings.window_capture.color),
            notice: None,
            notice_generation: 0,
            settings_save_revision: 0,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn select_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.stop_recording(cx);
        self.clear_notice(cx);
        cx.notify();
    }

    /// Applies one settings change and saves it in the background. The save
    /// result comes back through the editor observer as a toast.
    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        self.stop_recording(cx);
        self.editor.update(cx, |editor, cx| {
            change(&mut editor.settings);
            editor.persist_settings(cx);
        });
        self.clear_notice(cx);
        self.background_previews =
            background_previews(self.editor.read(cx).settings.window_capture.color);
        cx.notify();
    }

    /// Shows a message. Success messages hide after a short time; errors stay
    /// until the next change.
    fn show(
        &mut self,
        text: impl Into<String>,
        error: bool,
        field: Option<Field>,
        cx: &mut Context<Self>,
    ) {
        self.notice_generation += 1;
        self.notice = Some(Notice {
            text: text.into(),
            error,
            field,
        });
        if !error {
            let generation = self.notice_generation;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(1600))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.notice_generation == generation {
                        this.clear_notice(cx);
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn fail(&mut self, text: impl Into<String>, field: Option<Field>, cx: &mut Context<Self>) {
        self.show(text, true, field, cx);
    }

    fn clear_notice(&mut self, cx: &mut Context<Self>) {
        if self.notice.take().is_some() {
            self.notice_generation += 1;
            cx.notify();
        }
    }

    fn field_error(&self, field: Field) -> Option<SharedString> {
        self.notice
            .as_ref()
            .filter(|notice| notice.error && notice.field == Some(field))
            .map(|notice| notice.text.clone().into())
    }

    fn toggle(
        &self,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        checked: bool,
        set: fn(&mut Settings, bool),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        row(
            title,
            description,
            Switch::new(id)
                .checked(checked)
                .accessibility_label(title)
                .on_click(cx.listener(move |this, value: &bool, _, cx| {
                    this.change(cx, |settings| set(settings, *value))
                })),
            cx,
        )
    }

    fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose screenshots folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        this.change(cx, |settings| settings.screenshot_directory = Some(path))
                    });
                }
            }
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| this.fail(error.to_string(), None, cx));
            }
            _ => {}
        })
        .detach();
    }

    fn choose_wallpaper(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose wallpaper image".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let result = cx
                    .background_executor()
                    .spawn(
                        async move { sniplet_platform::load_wallpaper_image(&path).map(|_| path) },
                    )
                    .await;
                let _ = this.update(cx, |this, cx| match result {
                    Ok(path) => this.change(cx, |settings| {
                        settings.window_capture.wallpaper = Some(path)
                    }),
                    Err(error) => this.fail(error, Some(Field::Wallpaper), cx),
                });
            }
        })
        .detach();
    }

    fn set_startup(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let task = crate::runtime::change_startup(enabled, cx);
        let editor = self.editor.clone();
        self.clear_notice(cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(actual) if actual == enabled => this.show("Saved", false, None, cx),
                    Ok(_) => this.fail(
                        if cfg!(target_os = "macos") {
                            "Turn on Sniplet in System Settings → General → Login Items."
                        } else {
                            "Allow Sniplet to start in your system's startup settings."
                        },
                        Some(Field::Startup),
                        cx,
                    ),
                    Err(error) => this.fail(error, Some(Field::Startup), cx),
                }
                editor.update(cx, |_, cx| cx.notify());
            });
        })
        .detach();
    }

    fn set_background_color(
        &mut self,
        color: [u8; 3],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.background_color.update(cx, |input, cx| {
            input.set_value(color_hex(color), window, cx)
        });
        self.change(cx, |settings| settings.window_capture.color = color);
    }

    fn save_background_color(&mut self, cx: &mut Context<Self>) {
        let value = self.background_color.read(cx).value();
        let value = value.trim();
        let value = value.strip_prefix('#').unwrap_or(value);
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.fail(
                "Use a six-digit hex color, such as #F2F2F7.",
                Some(Field::BackgroundColor),
                cx,
            );
            return;
        }
        let color = u32::from_str_radix(value, 16).unwrap();
        let color = [(color >> 16) as u8, (color >> 8) as u8, color as u8];
        if self.editor.read(cx).settings.window_capture.color == color {
            self.clear_notice(cx);
            return;
        }
        self.change(cx, |settings| settings.window_capture.color = color);
    }

    fn start_recording(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = Some(index);
        self.live_modifiers = Modifiers::default();
        self.clear_notice(cx);
        // The root takes focus so the key reaches `record_key`, not a text field.
        self.focus.focus(window, cx);
        self.editor
            .update(cx, |editor, _| editor.hotkeys_paused = true);
        cx.notify();
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording.take().is_some() {
            self.editor
                .update(cx, |editor, _| editor.hotkeys_paused = false);
            cx.notify();
        }
    }

    /// Handles a key press while a shortcut field is recording.
    fn record_key(&mut self, index: usize, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let plain = !keystroke.modifiers.modified();
        match keystroke.key.as_str() {
            "escape" if plain => return self.stop_recording(cx),
            "backspace" | "delete" if plain => return self.set_hotkey(index, String::new(), cx),
            "shift" | "control" | "ctrl" | "alt" | "platform" | "cmd" | "super" | "win"
            | "function" | "fn" | "capslock" => return,
            _ => {}
        }
        match hotkey_text(keystroke) {
            Ok(text) => self.set_hotkey(index, text, cx),
            Err(message) => self.fail(message, Some(Field::Hotkey(index)), cx),
        }
    }

    /// Saves one shortcut, or explains why it cannot be used. An empty value turns it off.
    fn set_hotkey(&mut self, index: usize, value: String, cx: &mut Context<Self>) {
        let mut hotkeys = self.editor.read(cx).settings.hotkeys.clone();
        *hotkeys.slots_mut()[index] = value.clone();
        if !value.is_empty() {
            let conflict = crate::runtime::conflicting_hotkey(&hotkeys.slots(), index);
            let message = match conflict {
                Ok(None) => None,
                Ok(Some(other)) => Some(format!(
                    "{} is already used by {}.",
                    shortcut_label(&value),
                    shortcut_name(other)
                )),
                Err(_) => Some("That key can't be used in a shortcut.".to_owned()),
            };
            if let Some(message) = message {
                self.stop_recording(cx);
                self.fail(message, Some(Field::Hotkey(index)), cx);
                return;
            }
        }
        self.change(cx, |settings| settings.hotkeys = hotkeys);
    }

    fn set_upload_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.upload_open = open;
        if !open && self.editor.read(cx).settings.cloud_upload.is_some() {
            self.change(cx, |settings| settings.cloud_upload = None);
        } else {
            self.clear_notice(cx);
            cx.notify();
        }
    }

    fn save_upload(&mut self, cx: &mut Context<Self>) {
        let values = self
            .upload
            .each_ref()
            .map(|input| input.read(cx).value().trim().to_owned());
        let config = match self.upload_kind {
            UploadKind::SignedUrl => CloudUploadConfig::PresignedPut {
                url: values[0].clone(),
                public_url: optional(&values[1]),
            },
            UploadKind::S3 => CloudUploadConfig::S3 {
                bucket: values[2].clone(),
                region: values[3].clone(),
                endpoint: optional(&values[4]),
                key_prefix: values[5].clone(),
                public_base_url: optional(&values[6]),
            },
        };
        if let Err(error) = config.validate() {
            self.fail(error.to_string(), Some(Field::Upload), cx);
            return;
        }
        self.change(cx, |settings| settings.cloud_upload = Some(config));
    }

    fn general(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.editor.read(cx).settings.clone();
        let appearance = div().flex().flex_wrap().gap_3().children(
            [
                ("theme-system", "System", ThemePreference::System),
                ("theme-light", "Light", ThemePreference::Light),
                ("theme-dark", "Dark", ThemePreference::Dark),
            ]
            .map(|(id, label, preference)| {
                Button::new(id)
                    .ghost()
                    .selected(settings.theme == preference)
                    .w(px(132.0))
                    .h(px(106.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(appearance_preview(preference))
                            .child(div().text_sm().child(label)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change(cx, |settings| settings.theme = preference);
                        crate::theme::apply(preference, window, cx);
                    }))
            }),
        );
        let (startup_on, startup_busy) = crate::runtime::startup_state(cx);
        let about = div()
            .flex()
            .justify_center()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(format!("Sniplet {}", sniplet_platform::updates::VERSION))
            .child("·")
            .child(
                Button::new("reveal-settings")
                    .ghost()
                    .small()
                    .label("Show settings file")
                    .on_click(|_, _, cx| {
                        if let Ok(store) = sniplet_platform::SettingsStore::for_app() {
                            cx.reveal_path(store.path());
                        }
                    }),
            );
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section("Appearance", card(cx).p_3().child(appearance), cx))
            .child(section(
                "App",
                card(cx)
                    .child(row_with_error(
                        "Launch at startup",
                        "Start in the tray when you sign in. The editor stays closed.",
                        self.field_error(Field::Startup),
                        Switch::new("launch-at-startup")
                            .checked(startup_on)
                            .disabled(startup_busy)
                            .accessibility_label("Launch at startup")
                            .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                                this.set_startup(*enabled, cx)
                            })),
                        cx,
                    ))
                    .child(divider(cx))
                    .child(self.toggle(
                        "always-on-top",
                        "Keep editor on top",
                        "Takes effect after restarting Sniplet.",
                        settings.always_on_top,
                        |s, v| s.always_on_top = v,
                        cx,
                    )),
                cx,
            ))
            .child(crate::updates::settings_section(&self.editor, cx))
            .child(about)
            .into_any_element()
    }

    fn window_background(&self, cx: &mut Context<Self>) -> AnyElement {
        let style = self.editor.read(cx).settings.window_capture.clone();
        let choices = div().flex().gap_1().children(
            [
                ("window-wallpaper", "Wallpaper", WindowBackground::Wallpaper),
                (
                    "window-transparent",
                    "Transparent",
                    WindowBackground::Transparent,
                ),
                ("window-solid", "Solid color", WindowBackground::Solid),
                ("window-trim", "Trim shadow", WindowBackground::TrimShadow),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (id, title, background))| {
                Button::new(id)
                    .ghost()
                    .selected(style.background == background)
                    .flex_1()
                    .min_w_0()
                    .h(px(110.0))
                    .accessibility_label(title)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(96.0))
                                    .h(px(66.0))
                                    .rounded(px(6.0))
                                    .overflow_hidden()
                                    .border_2()
                                    .border_color(if style.background == background {
                                        rgb(crate::theme::ACCENT).into()
                                    } else {
                                        cx.theme().border
                                    })
                                    .child(
                                        img(self.background_previews[index].clone())
                                            .size_full()
                                            .object_fit(ObjectFit::Contain),
                                    ),
                            )
                            .child(div().text_xs().child(title)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |settings| {
                            settings.window_capture.background = background
                        })
                    }))
            }),
        );
        let swatches = div()
            .flex()
            .items_center()
            .children(SWATCHES.into_iter().enumerate().map(|(index, color)| {
                let selected = style.color == color;
                div()
                    .id(("swatch", index))
                    .test_support()
                    .size(px(28.0))
                    .rounded_full()
                    .border_2()
                    .border_color(if selected {
                        rgb(crate::theme::ACCENT).into()
                    } else {
                        cx.theme().transparent
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(20.0))
                            .rounded_full()
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(rgb(color_value(color))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_background_color(color, window, cx)
                    }))
            }));
        let wallpaper = style.background == WindowBackground::Wallpaper;
        section(
            "Window screenshot background",
            card(cx)
                .child(div().p_3().child(choices))
                .child(div().px_4().pb_3().child(note(
                    "Applied to window captures. Padded modes add a soft shadow. Trim shadow keeps the window's exact size.",
                    cx,
                )))
                .when(style.background != WindowBackground::TrimShadow, |card| {
                    card.child(divider(cx)).child(row(
                        "Padding",
                        "Space around the window at 1× resolution.",
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .id("window-padding")
                                    .test_support()
                                    .w(px(150.0))
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .child(Slider::new(&self.window_padding)),
                            )
                            .child(div().w(px(44.0)).text_sm().child(format!(
                                "{} pt",
                                self.window_padding.read(cx).value().end() as u32
                            ))),
                        cx,
                    ))
                })
                .when(
                    matches!(
                        style.background,
                        WindowBackground::Solid | WindowBackground::Wallpaper
                    ),
                    |card| {
                        card.child(divider(cx)).child(row_with_error(
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
                            self.field_error(Field::BackgroundColor),
                            div().flex().items_center().gap_2().child(swatches).child(
                                Input::new(&self.background_color)
                                    .id("window-background-color")
                                    .w(px(96.0)),
                            ),
                            cx,
                        ))
                    },
                )
                .when(wallpaper, |card| {
                    card.child(divider(cx))
                        .child(row_with_error(
                            "Wallpaper image",
                            "Use your desktop wallpaper or choose an image.",
                            self.field_error(Field::Wallpaper),
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new("desktop-wallpaper")
                                        .ghost()
                                        .label("Use desktop")
                                        .disabled(style.wallpaper.is_none())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.change(cx, |settings| {
                                                settings.window_capture.wallpaper = None
                                            })
                                        })),
                                )
                                .child(Button::new("choose-wallpaper").label("Choose…").on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.choose_wallpaper(window, cx)
                                    }),
                                )),
                            cx,
                        ))
                        .child(
                            div()
                                .px_4()
                                .pb_3()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .truncate()
                                .child(style.wallpaper.as_ref().map_or_else(
                                    || "Desktop wallpaper".into(),
                                    |path| path.display().to_string(),
                                )),
                        )
                }),
            cx,
        )
        .into_any_element()
    }

    fn capture(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.editor.read(cx).settings.clone();
        let limit = settings.scroll_max_frames;
        let stepper = div()
            .flex()
            .items_center()
            .h(px(32.0))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("scroll-limit-down")
                    .ghost()
                    .small()
                    .icon(tool_icon("minus"))
                    .accessibility_label("Fewer frames")
                    .disabled(limit <= 2)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |s| s.scroll_max_frames = step_limit(limit, false))
                    })),
            )
            .child(
                div()
                    .id("scroll-limit")
                    .test_support()
                    .w(px(40.0))
                    .flex()
                    .justify_center()
                    .child(limit.to_string()),
            )
            .child(
                Button::new("scroll-limit-up")
                    .ghost()
                    .small()
                    .icon(tool_icon("plus"))
                    .accessibility_label("More frames")
                    .disabled(limit >= 200)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |s| s.scroll_max_frames = step_limit(limit, true))
                    })),
            );
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(self.window_background(cx))
            .child(section(
                "After capture",
                card(cx)
                    .child(self.toggle(
                        "auto-copy",
                        "Copy captures automatically",
                        "A fresh capture is ready to paste.",
                        settings.auto_copy,
                        |s, v| s.auto_copy = v,
                        cx,
                    ))
                    .child(divider(cx))
                    .child(self.toggle(
                        "hide-after-export",
                        "Hide editor after copy or save",
                        "Return to the app you were working in.",
                        settings.hide_after_export,
                        |s, v| s.hide_after_export = v,
                        cx,
                    )),
                cx,
            ))
            .child(section(
                "Scrolling capture",
                card(cx)
                    .child(row(
                        "Capture limit",
                        "Most frames in one scrolling capture (2–200).",
                        stepper,
                        cx,
                    ))
                    .child(divider(cx))
                    .child(row(
                        "Scrolling speed",
                        "Slow down for pages that need time to load.",
                        div()
                            .w(px(200.0))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .id("scroll-speed")
                                    .test_support()
                                    .w_full()
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .child(Slider::new(&self.scroll_speed)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Slower")
                                    .child("Faster"),
                            ),
                        cx,
                    )),
                cx,
            ))
            .into_any_element()
    }

    /// A click-to-record shortcut field that shows the saved keys as key caps.
    fn recorder(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.editor.read(cx).settings.hotkeys.slots()[index].to_owned();
        let changed = text != HotkeySettings::default().slots()[index];
        let live = self.recording == Some(index);
        let border = if self.field_error(Field::Hotkey(index)).is_some() {
            cx.theme().danger
        } else if live {
            rgb(crate::theme::ACCENT).into()
        } else {
            cx.theme().border
        };
        let content = if live {
            let held = modifier_tokens(self.live_modifiers);
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(held.into_iter().map(|token| key_cap(&key_label(token), cx)))
                .child(div().text_xs().text_color(rgb(crate::theme::ACCENT)).child(
                    if self.live_modifiers.modified() {
                        "…"
                    } else {
                        "Press keys…"
                    },
                ))
        } else if text.is_empty() {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Off")
        } else {
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(text.split('+').map(|token| key_cap(&key_label(token), cx)))
        };
        let action = |id: (&'static str, usize), icon: &'static str, label: &'static str| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(tool_icon(icon).size(px(14.0)))
                .accessibility_label(label)
                .tooltip(label)
        };
        div()
            .id(("hotkey", index))
            .test_support()
            .w(px(230.0))
            .h(px(32.0))
            .pl_2()
            .pr_1()
            .rounded_md()
            .border_1()
            .border_color(border)
            .flex()
            .items_center()
            .justify_between()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.recording == Some(index) {
                    this.stop_recording(cx);
                } else {
                    this.start_recording(index, window, cx);
                }
            }))
            .child(content)
            .child(
                div()
                    .flex()
                    .when(live, |actions| {
                        actions.child(action(("hotkey-cancel", index), "x", "Cancel").on_click(
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.stop_recording(cx);
                            }),
                        ))
                    })
                    .when(!live && changed, |actions| {
                        actions.child(
                            action(("hotkey-reset", index), "rotate-ccw", "Restore default")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let value = HotkeySettings::default().slots()[index].to_owned();
                                    this.set_hotkey(index, value, cx);
                                })),
                        )
                    })
                    .when(!live && !text.is_empty(), |actions| {
                        actions.child(action(("hotkey-clear", index), "x", "Turn off").on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.set_hotkey(index, String::new(), cx);
                            }),
                        ))
                    }),
            )
    }

    fn shortcuts(&self, cx: &mut Context<Self>) -> AnyElement {
        let registration_error = self
            .recording
            .is_none()
            .then(|| self.editor.read(cx).hotkey_error.clone())
            .flatten();
        let groups = SHORTCUT_GROUPS.map(|(group, items)| {
            let mut rows = card(cx);
            for (position, &(index, label)) in items.iter().enumerate() {
                if position > 0 {
                    rows = rows.child(divider(cx));
                }
                let recorder = self.recorder(index, cx).into_any_element();
                rows = rows.child(row_with_error(
                    label,
                    "",
                    self.field_error(Field::Hotkey(index)),
                    recorder,
                    cx,
                ));
            }
            section(group, rows, cx)
        });
        div()
            .flex()
            .flex_col()
            .gap_5()
            .when_some(registration_error, |page, error| {
                page.child(
                    div()
                        .p_3()
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .children(groups)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap_4()
                    .child(div().flex_1().min_w_0().child(note(
                        "Click a shortcut, then press the new keys. Esc cancels. Backspace turns it off.",
                        cx,
                    )))
                    .child(
                        Button::new("reset-hotkeys")
                            .ghost()
                            .icon(tool_icon("rotate-ccw"))
                            .label("Restore all defaults")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.change(cx, |s| s.hotkeys = HotkeySettings::default())
                            })),
                    ),
            )
            .into_any_element()
    }

    fn share(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.editor.read(cx).settings.clone();
        let directory = settings
            .screenshot_directory
            .clone()
            .or_else(|| sniplet_platform::default_export_directory().ok());
        let format = div().flex().gap_1().children(
            [
                ("png", "PNG", ExportFormat::Png),
                ("jpg", "JPEG", ExportFormat::Jpeg),
                ("webp", "WebP", ExportFormat::Webp),
            ]
            .map(|(id, label, format)| {
                Button::new(id)
                    .ghost()
                    .label(label)
                    .selected(settings.format == format)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |settings| settings.format = format)
                    }))
            }),
        );
        let folder = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("choose-screenshot-folder")
                    .label("Choose…")
                    .on_click(cx.listener(|this, _, window, cx| this.choose_folder(window, cx))),
            )
            .child(
                Button::new("reset-screenshot-folder")
                    .ghost()
                    .icon(tool_icon("rotate-ccw"))
                    .tooltip("Use the default folder")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change(cx, |settings| settings.screenshot_directory = None)
                    })),
            );
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section(
                "Saving",
                card(cx)
                    .child(row(
                        "Screenshots folder",
                        "The starting folder when you save a screenshot.",
                        folder,
                        cx,
                    ))
                    .child(
                        div()
                            .px_4()
                            .pb_3()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(directory.map_or_else(
                                || "Choose a folder".into(),
                                |path| path.display().to_string(),
                            )),
                    )
                    .child(divider(cx))
                    .child(row("Save format", "The default image format.", format, cx))
                    .child(divider(cx))
                    .child(self.toggle(
                        "downscale-on-save",
                        "Save Retina screenshots at 1×",
                        "The editor and clipboard keep full resolution.",
                        settings.downscale_on_save,
                        |s, v| s.downscale_on_save = v,
                        cx,
                    )),
                cx,
            ))
            .child(self.upload_section(settings.cloud_upload.is_some(), cx))
            .into_any_element()
    }

    fn upload_section(&self, configured: bool, cx: &mut Context<Self>) -> AnyElement {
        let status = if configured {
            "On. Screenshots upload only when you choose Upload."
        } else if self.upload_open {
            "Save a destination below to turn it on."
        } else {
            "Off. Turn on to set up a destination."
        };
        let mut upload = card(cx).child(row(
            "Upload to the cloud",
            status,
            Switch::new("upload-enabled")
                .checked(self.upload_open)
                .accessibility_label("Upload to the cloud")
                .on_click(cx.listener(|this, open: &bool, _, cx| this.set_upload_open(*open, cx))),
            cx,
        ));
        if !self.upload_open {
            return section("Upload", upload, cx).into_any_element();
        }
        let kinds = div().flex().gap_1().children(
            [
                ("upload-signed-url", "Signed URL", UploadKind::SignedUrl),
                ("upload-s3", "S3 compatible", UploadKind::S3),
            ]
            .map(|(id, label, kind)| {
                Button::new(id)
                    .ghost()
                    .label(label)
                    .selected(self.upload_kind == kind)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.upload_kind = kind;
                        this.clear_notice(cx);
                        cx.notify();
                    }))
            }),
        );
        let fields: &[(usize, &str)] = match self.upload_kind {
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
            .child(divider(cx))
            .child(row("Destination", "", kinds, cx))
            .child(divider(cx));
        for &(index, label) in fields {
            upload = upload.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .child(label)
                    .child(Input::new(&self.upload[index]).id(("upload-input", index))),
            );
        }
        upload = match self.upload_kind {
            UploadKind::SignedUrl => upload.child(div().px_4().pb_3().child(note(
                "Paste a signed upload URL from your provider. Signed URLs expire, so paste a new one when uploads fail.",
                cx,
            ))),
            UploadKind::S3 => {
                let found = sniplet_platform::s3_credentials_available();
                upload.child(divider(cx)).child(row(
                    "AWS credentials",
                    if found {
                        "Read from AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY. AWS_SESSION_TOKEN is optional."
                    } else {
                        "Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, then restart Sniplet."
                    },
                    div()
                        .id("aws-credentials")
                        .test_support()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(if found {
                            cx.theme().success
                        } else {
                            cx.theme().danger
                        })
                        .child(
                            tool_icon(if found { "circle-check" } else { "circle-alert" })
                                .size(px(14.0)),
                        )
                        .child(if found { "Found" } else { "Not found" }),
                    cx,
                ))
            }
        };
        let footer_message = match self.field_error(Field::Upload) {
            Some(error) => div().text_xs().text_color(cx.theme().danger).child(error),
            None => note(
                if configured {
                    "Save again after you change a field."
                } else {
                    "Upload stays off until you save a destination."
                },
                cx,
            ),
        };
        upload = upload.child(
            div()
                .px_4()
                .py_3()
                .border_t_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted.opacity(0.4))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(div().flex_1().min_w_0().child(footer_message))
                .child(
                    Button::new("save-upload")
                        .primary()
                        .label("Save destination")
                        .on_click(cx.listener(|this, _, _, cx| this.save_upload(cx))),
                ),
        );
        section("Upload", upload, cx).into_any_element()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.editor.read(cx).updates.installing() {
            return crate::updates::installing_screen(cx);
        }
        let page = match self.page {
            Page::General => self.general(cx),
            Page::Capture => self.capture(cx),
            Page::Shortcuts => self.shortcuts(cx),
            Page::Share => self.share(cx),
        };
        let tabs = div()
            .flex()
            .flex_shrink_0()
            .justify_center()
            .gap_1()
            .p_2()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .children(Page::ALL.into_iter().enumerate().map(|(index, page)| {
                let selected = self.page == page;
                let hover = cx.theme().foreground.opacity(0.06);
                div()
                    .id(("settings-nav", index))
                    .test_support()
                    .w(px(96.0))
                    .h(px(56.0))
                    .rounded_md()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_xs()
                    .cursor_pointer()
                    .text_color(if selected {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .hover(move |style| style.bg(hover))
                    .when(selected, |tab| {
                        tab.bg(cx.theme().foreground.opacity(0.1))
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(
                        tool_icon(page.icon())
                            .size(px(20.0))
                            .text_color(if selected {
                                rgb(crate::theme::ACCENT).into()
                            } else {
                                cx.theme().muted_foreground
                            }),
                    )
                    .child(page.title())
                    .on_click(cx.listener(move |this, _, _, cx| this.select_page(page, cx)))
            }));
        let toast = self
            .notice
            .as_ref()
            .filter(|notice| notice.field.is_none())
            .map(|notice| {
                let error = notice.error;
                div()
                    .id("settings-toast")
                    .test_support()
                    .absolute()
                    .right_4()
                    .bottom_4()
                    .max_w(px(420.0))
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .shadow_lg()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .when(error, |toast| {
                        toast
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().danger)
                            .text_color(cx.theme().danger)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.clear_notice(cx)))
                    })
                    .when(!error, |toast| {
                        toast
                            .bg(cx.theme().foreground)
                            .text_color(cx.theme().background)
                    })
                    .child(tool_icon(if error { "circle-alert" } else { "check" }).size(px(14.0)))
                    .child(notice.text.clone())
            });
        div()
            .id("settings-window")
            .test_support()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .track_focus(&self.focus)
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                if this.recording.is_some() {
                    this.live_modifiers = event.modifiers;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if let Some(index) = this.recording {
                    this.record_key(index, &event.keystroke, cx);
                    cx.stop_propagation();
                    return;
                }
                let key = event.keystroke.key.as_str();
                if key == "escape"
                    || (key == "w"
                        && (event.keystroke.modifiers.control
                            || event.keystroke.modifiers.platform))
                {
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .child(
                TitleBar::new()
                    .h(px(48.0))
                    .on_close_window(|_, window, _| window.remove_window())
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    ),
            )
            .child(tabs)
            .child(
                div()
                    .id(("settings-content", self.page as usize))
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .child(page),
            )
            .children(toast)
            .into_any_element()
    }
}

fn optional(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

/// Moves the scrolling capture limit to the next multiple of 10, within 2–200.
fn step_limit(limit: usize, up: bool) -> usize {
    if up {
        (limit / 10 * 10 + 10).min(200)
    } else {
        (limit.div_ceil(10) * 10).saturating_sub(10).max(2)
    }
}

/// Held modifiers as shortcut tokens. The platform's main modifier (Cmd on
/// Mac, Ctrl elsewhere) becomes `CommandOrControl`, like the default shortcuts.
fn modifier_tokens(modifiers: Modifiers) -> Vec<&'static str> {
    let mac = cfg!(target_os = "macos");
    let (primary, secondary) = if mac {
        (modifiers.platform, modifiers.control)
    } else {
        (modifiers.control, modifiers.platform)
    };
    [
        (primary, "CommandOrControl"),
        (secondary, if mac { "Control" } else { "Super" }),
        (modifiers.alt, "Alt"),
        (modifiers.shift, "Shift"),
    ]
    .into_iter()
    .filter_map(|(held, token)| held.then_some(token))
    .collect()
}

/// Converts a recorded key press into the shortcut format that `parse_hotkey` reads.
fn hotkey_text(keystroke: &Keystroke) -> Result<String, &'static str> {
    const NAMED: [(&str, &str); 14] = [
        ("space", "Space"),
        ("enter", "Enter"),
        ("tab", "Tab"),
        ("home", "Home"),
        ("end", "End"),
        ("pageup", "PageUp"),
        ("pagedown", "PageDown"),
        ("insert", "Insert"),
        ("delete", "Delete"),
        ("backspace", "Backspace"),
        ("up", "Up"),
        ("down", "Down"),
        ("left", "Left"),
        ("right", "Right"),
    ];
    // GPUI reports Shift+1 as "!" with Shift cleared. Shortcuts use physical
    // keys, so map the symbol back to its key on a US layout and restore Shift.
    const SHIFTED: [(&str, &str); 21] = [
        ("!", "1"),
        ("@", "2"),
        ("#", "3"),
        ("$", "4"),
        ("%", "5"),
        ("^", "6"),
        ("&", "7"),
        ("*", "8"),
        ("(", "9"),
        (")", "0"),
        ("_", "-"),
        ("+", "="),
        ("{", "["),
        ("}", "]"),
        ("|", "\\"),
        (":", ";"),
        ("\"", "'"),
        ("<", ","),
        (">", "."),
        ("?", "/"),
        ("~", "`"),
    ];
    let mut modifiers = keystroke.modifiers;
    let mut key = keystroke.key.as_str();
    if let Some((_, base)) = SHIFTED.iter().find(|(shifted, _)| *shifted == key) {
        key = base;
        modifiers.shift = true;
    }
    let function_key = key.len() > 1 && key.starts_with('f') && key[1..].parse::<u8>().is_ok();
    let key = if let Some((_, name)) = NAMED.iter().find(|(raw, _)| *raw == key) {
        (*name).to_owned()
    } else if key.chars().count() == 1 || function_key {
        key.to_uppercase()
    } else {
        return Err("That key can't be used in a shortcut.");
    };
    if !(modifiers.control || modifiers.platform || modifiers.alt) && !function_key {
        return Err(if cfg!(target_os = "macos") {
            "Add ⌘, ⌥, or ⌃, or use a function key."
        } else {
            "Add Ctrl or Alt, or use a function key."
        });
    }
    let mut tokens = modifier_tokens(modifiers);
    tokens.push(&key);
    Ok(tokens.join("+"))
}

/// The label for one shortcut token, such as `Ctrl`, `⌘`, or `2`.
fn key_label(token: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let label = match token.to_ascii_lowercase().as_str() {
        "commandorcontrol" | "commandorctrl" | "cmdorctrl" | "cmdorcontrol" => {
            if mac {
                "⌘"
            } else {
                "Ctrl"
            }
        }
        "control" | "ctrl" => {
            if mac {
                "⌃"
            } else {
                "Ctrl"
            }
        }
        "alt" | "option" => {
            if mac {
                "⌥"
            } else {
                "Alt"
            }
        }
        "shift" => {
            if mac {
                "⇧"
            } else {
                "Shift"
            }
        }
        "super" | "cmd" | "command" => {
            if mac {
                "⌘"
            } else {
                "Win"
            }
        }
        "up" | "arrowup" => "↑",
        "down" | "arrowdown" => "↓",
        "left" | "arrowleft" => "←",
        "right" | "arrowright" => "→",
        _ => {
            return token
                .strip_prefix("Key")
                .or_else(|| token.strip_prefix("Digit"))
                .filter(|rest| !rest.is_empty())
                .unwrap_or(token)
                .to_owned();
        }
    };
    label.to_owned()
}

/// A shortcut in readable form, such as `Ctrl+Shift+2`.
fn shortcut_label(text: &str) -> String {
    text.split('+').map(key_label).collect::<Vec<_>>().join("+")
}

fn shortcut_name(index: usize) -> &'static str {
    SHORTCUT_GROUPS
        .iter()
        .flat_map(|(_, items)| items.iter())
        .find(|(slot, _)| *slot == index)
        .map_or("another shortcut", |(_, name)| name)
}

fn key_cap(label: &str, cx: &App) -> Div {
    div()
        .h(px(20.0))
        .min_w(px(20.0))
        .px(px(6.0))
        .rounded(px(4.0))
        .border_1()
        .border_b_2()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .child(label.to_owned())
}

pub(crate) fn note(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub(crate) fn card(cx: &App) -> Div {
    div()
        .rounded(px(12.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .overflow_hidden()
}

pub(crate) fn section(title: &'static str, content: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
        .child(content)
}

pub(crate) fn divider(cx: &App) -> Div {
    div().mx_4().h(px(1.0)).bg(cx.theme().border)
}

pub(crate) fn row(
    title: &'static str,
    description: &'static str,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    row_with_error(title, description, None, control, cx)
}

/// A settings row. A validation error shows under the title, next to the control it is about.
fn row_with_error(
    title: &'static str,
    description: &'static str,
    error: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .p_4()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(title)
                .when(!description.is_empty(), |label| {
                    label.child(note(description, cx))
                })
                .when_some(error, |label, error| {
                    label.child(div().text_xs().text_color(cx.theme().danger).child(error))
                }),
        )
        .child(div().flex_shrink_0().child(control))
}

fn color_value(color: [u8; 3]) -> u32 {
    (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2])
}

fn color_hex(color: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

fn background_previews(color: [u8; 3]) -> [std::sync::Arc<RenderImage>; 4] {
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
    for (center, color) in [
        (10_i32, [255, 95, 87, 255]),
        (20, [254, 188, 46, 255]),
        (30, [40, 200, 64, 255]),
    ] {
        for y in 5..11 {
            for x in center - 3..center + 3 {
                if (x - center).pow(2) + (y - 8_i32).pow(2) <= 9 {
                    source.put_pixel(x as u32, y as u32, Rgba(color));
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
    [
        WindowBackground::Wallpaper,
        WindowBackground::Transparent,
        WindowBackground::Solid,
        WindowBackground::TrimShadow,
    ]
    .map(|background| {
        let style = WindowCaptureStyle {
            background,
            padding: 12,
            color,
            wallpaper: None,
        };
        let preview =
            sniplet_platform::compose_window_capture(source.clone(), &style, 1.0, Some(&wallpaper));
        // Checkerboard is only a thumbnail aid, never part of the capture.
        let mut thumbnail = RgbaImage::from_fn(preview.width(), preview.height(), |x, y| {
            let value = if (x / 7 + y / 7) % 2 == 0 { 68 } else { 58 };
            Rgba([value, value, value, 255])
        });
        imageops::overlay(&mut thumbnail, &preview, 0, 0);
        crate::editor::display_image(thumbnail)
    })
}

fn appearance_preview(preference: ThemePreference) -> Div {
    let dark = preference == ThemePreference::Dark;
    let background = if dark { rgb(0x24272c) } else { rgb(0xf1f2f5) };
    let sidebar = if dark { rgb(0x353940) } else { rgb(0xe0e3e9) };
    div()
        .relative()
        .w(px(100.0))
        .h(px(58.0))
        .rounded(px(6.0))
        .overflow_hidden()
        .border_1()
        .border_color(rgb(0x8a8e96))
        .bg(background)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(13.0))
                .px(px(5.0))
                .flex()
                .items_center()
                .gap(px(3.0))
                .bg(sidebar)
                .children(
                    [0xff6058, 0xfebb2e, 0x28c840]
                        .map(|color| div().size(px(4.0)).rounded_full().bg(rgb(color))),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .child(div().w(px(28.0)).bg(sidebar))
                .child(
                    div()
                        .flex_1()
                        .p(px(7.0))
                        .child(div().h(px(5.0)).rounded(px(2.0)).bg(rgb(0x2684ff)))
                        .child(
                            div()
                                .mt(px(5.0))
                                .h(px(4.0))
                                .w(px(30.0))
                                .rounded(px(2.0))
                                .bg(sidebar),
                        ),
                ),
        )
        .when(preference == ThemePreference::System, |tile| {
            tile.child(
                div()
                    .absolute()
                    .right_0()
                    .bottom_0()
                    .w(px(24.0))
                    .h(px(30.0))
                    .bg(rgb(0x24272c)),
            )
        })
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui_kit::test::TestWindowExt;
    use sniplet_platform::SettingsStore;
    use std::prelude::v1::test;

    /// The main modifier in test keystrokes: Cmd on Mac, Ctrl elsewhere.
    const PRIMARY: &str = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };

    /// Types `value` into a settings text field and presses Enter. Input events
    /// reach subscribers after the update ends, so this runs as its own update.
    fn enter(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        input: &Entity<InputState>,
        value: &str,
    ) {
        handle
            .update(cx, |_, window, cx| {
                input.update(cx, |input, cx| {
                    input.set_value(value, window, cx);
                    cx.emit(InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    });
                });
            })
            .unwrap();
    }

    fn fixture(
        cx: &mut TestAppContext,
        page: Page,
    ) -> (
        AnyWindowHandle,
        Entity<SettingsWindow>,
        Entity<Editor>,
        std::path::PathBuf,
    ) {
        let path = std::env::temp_dir().join(format!(
            "sniplet-settings-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (parent, editor) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Some(sniplet_core::Document::new(image::RgbaImage::from_pixel(
                            80,
                            60,
                            image::Rgba([20, 40, 60, 255]),
                        ))),
                        Settings::default(),
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        parent
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.settings_path = Some(path.clone());
                    editor.open_settings(page, window, cx);
                })
            })
            .unwrap();
        let (handle, entity) = cx.read(|cx| {
            let (handle, entity) = editor.read(cx).settings_window.as_ref().unwrap();
            (*handle, entity.upgrade().unwrap())
        });
        (handle, entity, editor, path)
    }

    fn scroll_bottom(window: &mut Window, page: Page, cx: &mut App) {
        window.scroll(
            ("settings-content", page as usize),
            ScrollDelta::Pixels(point(px(0.0), px(-1600.0))),
            cx,
        );
    }

    fn notice(entity: &Entity<SettingsWindow>, cx: &App) -> Option<(String, bool)> {
        entity
            .read(cx)
            .notice
            .as_ref()
            .map(|notice| (notice.text.clone(), notice.error))
    }

    #[gpui_kit::test]
    fn settings_writes_coalesce_and_report_disk_errors(cx: &mut TestAppContext) {
        let (handle, entity, editor, path) = fixture(cx, Page::General);
        handle
            .update(cx, |_, _, cx| {
                entity.update(cx, |settings, cx| {
                    settings.change(cx, |s| s.scroll_max_frames = 30);
                    settings.change(cx, |s| s.scroll_max_frames = 80);
                });
                assert!(
                    !path.exists(),
                    "settings writes must not block the input handler"
                );
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            SettingsStore::at(&path).load().unwrap().scroll_max_frames,
            80
        );
        assert_eq!(
            cx.read(|cx| notice(&entity, cx)),
            Some(("Saved".into(), false))
        );
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("settings-toast").visible());
                editor.update(cx, |editor, _| editor.settings_path = Some(path.clone()));
            })
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        handle
            .update(cx, |_, _, cx| {
                entity.update(cx, |settings, cx| {
                    settings.change(cx, |s| s.scroll_max_frames = 40)
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert!(cx.read(|cx| notice(&entity, cx)).unwrap().1);
        std::fs::remove_dir(path).unwrap();
    }

    #[gpui_kit::test]
    fn window_background_choices_padding_swatches_and_color_persist_without_changing_open_capture(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, path) = fixture(cx, Page::Capture);
        let original = cx.read(|cx| editor.read(cx).export_pixels().unwrap());
        let store = SettingsStore::at(&path);
        handle
            .update(cx, |_, window, cx| {
                window.resize(size(px(560.0), px(480.0)));
                for (id, mode) in [
                    ("window-wallpaper", WindowBackground::Wallpaper),
                    ("window-trim", WindowBackground::TrimShadow),
                    ("window-transparent", WindowBackground::Transparent),
                    ("window-solid", WindowBackground::Solid),
                ] {
                    window.render_frame(cx);
                    window.click(id, cx);
                    assert_eq!(editor.read(cx).settings.window_capture.background, mode);
                    assert!(window.find(id).bounds().right() <= window.viewport_size().width);
                }
                window.render_frame(cx);
                for index in 0..SWATCHES.len() {
                    assert!(
                        window.find(("swatch", index)).bounds().right()
                            <= window.viewport_size().width
                    );
                }
                window.click(("swatch", 2_usize), cx);
                assert_eq!(
                    editor.read(cx).settings.window_capture.color,
                    [0x1C, 0x1C, 0x1E]
                );
                assert_eq!(
                    entity.read(cx).background_color.read(cx).value().as_str(),
                    "#1C1C1E"
                );
            })
            .unwrap();
        let color = cx.read(|cx| entity.read(cx).background_color.clone());
        enter(cx, handle, &color, "invalid");
        cx.read(|cx| {
            assert!(
                entity
                    .read(cx)
                    .field_error(Field::BackgroundColor)
                    .is_some()
            );
            assert_eq!(
                editor.read(cx).settings.window_capture.color,
                [0x1C, 0x1C, 0x1E]
            );
        });
        enter(cx, handle, &color, "#123456");
        cx.read(|cx| {
            assert!(
                entity
                    .read(cx)
                    .field_error(Field::BackgroundColor)
                    .is_none()
            );
            assert_eq!(editor.read(cx).settings.window_capture.color, [18, 52, 86]);
        });
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click_at("window-padding", point(px(120.0), px(12.0)), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(store.load().unwrap().window_capture.padding > 32);
        assert_eq!(store.load().unwrap().window_capture.color, [18, 52, 86]);
        assert_eq!(
            store.load().unwrap().window_capture.background,
            WindowBackground::Solid
        );
        assert_eq!(
            cx.read(|cx| editor.read(cx).export_pixels().unwrap()),
            original
        );
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn wallpaper_picker_cancel_invalid_and_valid_image_then_reset_to_desktop(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, path) = fixture(cx, Page::Capture);
        let wallpaper_path = path.with_extension("wallpaper.png");
        image::RgbaImage::from_pixel(40, 30, image::Rgba([40, 100, 180, 255]))
            .save(&wallpaper_path)
            .unwrap();
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click("window-wallpaper", cx);
                window.scroll(
                    ("settings-content", Page::Capture as usize),
                    ScrollDelta::Pixels(point(px(0.0), px(-260.0))),
                    cx,
                );
                window.render_frame(cx);
                window.click("choose-wallpaper", cx);
            })
            .unwrap();
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files && !options.directories && !options.multiple);
            None
        });
        cx.run_until_parked();
        assert!(cx.read(|cx| editor.read(cx).settings.window_capture.wallpaper.is_none()));
        for candidate in [path.with_extension("missing.png"), wallpaper_path.clone()] {
            handle
                .update(cx, |_, window, cx| {
                    window.render_frame(cx);
                    window.click("choose-wallpaper", cx);
                })
                .unwrap();
            cx.simulate_path_prompt_response(|_| Some(vec![candidate.clone()]));
            cx.run_until_parked();
            assert_eq!(
                cx.read(|cx| entity.read(cx).field_error(Field::Wallpaper).is_some()),
                !candidate.exists()
            );
        }
        assert_eq!(
            SettingsStore::at(&path)
                .load()
                .unwrap()
                .window_capture
                .wallpaper,
            Some(wallpaper_path.clone())
        );
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click("desktop-wallpaper", cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(
            SettingsStore::at(&path)
                .load()
                .unwrap()
                .window_capture
                .wallpaper
                .is_none()
        );
        std::fs::remove_file(wallpaper_path).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn settings_folder_picker_cancel_and_retina_save_use_the_saved_preferences(
        cx: &mut TestAppContext,
    ) {
        let (handle, _, editor, path) = fixture(cx, Page::Share);
        let store = SettingsStore::at(&path);
        let directory = std::env::temp_dir();
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click("choose-screenshot-folder", cx);
            })
            .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert!(options.directories && !options.files && !options.multiple);
            Some(vec![directory.clone()])
        });
        cx.run_until_parked();
        assert_eq!(
            store.load().unwrap().screenshot_directory,
            Some(directory.clone())
        );
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click("choose-screenshot-folder", cx);
            })
            .unwrap();
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert_eq!(
            store.load().unwrap().screenshot_directory,
            Some(directory.clone())
        );
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click("jpg", cx);
                window.click("downscale-on-save", cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(store.load().unwrap().format, ExportFormat::Jpeg);
        assert!(store.load().unwrap().downscale_on_save);
        let parent = cx
            .windows()
            .into_iter()
            .find(|window| *window != handle)
            .unwrap();
        parent
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.set_measure_scale(2.0);
                    editor.command(crate::editor::Command::Save, window, cx);
                })
            })
            .unwrap();
        let image_path = path.with_extension("jpg");
        cx.simulate_new_path_selection(|suggested| {
            assert_eq!(suggested, directory.as_path());
            Some(image_path.clone())
        });
        cx.run_until_parked();
        assert_eq!(
            image::open(&image_path).unwrap().to_rgba8().dimensions(),
            (40, 30)
        );
        assert_eq!(
            cx.read(|cx| editor.read(cx).export_pixels().unwrap().dimensions()),
            (80, 60)
        );
        std::fs::remove_file(image_path).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn settings_reopens_the_same_window_and_creates_a_new_one_after_close(cx: &mut TestAppContext) {
        let (handle, entity, editor, _) = fixture(cx, Page::General);
        let parent = cx
            .windows()
            .into_iter()
            .find(|window| *window != handle)
            .unwrap();
        parent
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.open_settings(Page::Share, window, cx)
                })
            })
            .unwrap();
        assert_eq!(cx.windows().len(), 2);
        assert_eq!(cx.read(|cx| entity.read(cx).page), Page::Share);
        assert_eq!(
            cx.read(|cx| editor.read(cx).settings_window.as_ref().unwrap().0),
            handle
        );
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        assert_eq!(cx.windows().len(), 1);
        parent
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.open_settings(Page::General, window, cx)
                })
            })
            .unwrap();
        assert_eq!(cx.windows().len(), 2);
        assert_ne!(
            cx.read(|cx| editor.read(cx).settings_window.as_ref().unwrap().0),
            handle
        );
    }

    #[gpui_kit::test]
    fn settings_tabs_fit_the_minimum_window_in_both_themes_and_escape_preserves_editor(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, _) = fixture(cx, Page::General);
        let original = cx.read(|cx| editor.read(cx).export_pixels().unwrap());
        handle
            .update(cx, |_, window, _| window.resize(size(px(560.0), px(480.0))))
            .unwrap();
        handle
            .update(cx, |_, window, cx| {
                for theme in [ThemePreference::Light, ThemePreference::Dark] {
                    crate::theme::apply(theme, window, cx);
                    for (index, page) in Page::ALL.into_iter().enumerate() {
                        window.render_frame(cx);
                        window.click(("settings-nav", index), cx);
                        assert_eq!(entity.read(cx).page, page);
                        window.render_frame(cx);
                        let width = window.viewport_size().width;
                        assert!(window.find(("settings-nav", index)).bounds().right() <= width);
                        let content = window.find(("settings-content", index));
                        assert!(content.bounds().right() <= width);
                        if page == Page::Shortcuts {
                            for slot in 0_usize..8 {
                                assert!(window.find(("hotkey", slot)).bounds().right() <= width);
                            }
                            scroll_bottom(window, page, cx);
                            window.render_frame(cx);
                            let reset = window.find("reset-hotkeys");
                            assert!(reset.visible());
                            assert!(reset.bounds().right() <= width);
                        }
                    }
                }
                entity.read(cx).focus.clone().focus(window, cx);
                window.press("escape", cx);
            })
            .unwrap();
        cx.read(|cx| {
            assert_eq!(editor.read(cx).export_pixels().unwrap(), original);
            assert_eq!(cx.windows().len(), 1);
            assert!(entity.read(cx).editor == editor);
        });
    }

    #[gpui_kit::test]
    fn shortcut_recorder_saves_keys_rejects_conflicts_and_pauses_global_shortcuts(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, path) = fixture(cx, Page::Shortcuts);
        let store = SettingsStore::at(&path);
        let defaults = HotkeySettings::default();
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click(("hotkey", 0_usize), cx);
                assert_eq!(entity.read(cx).recording, Some(0));
                assert!(editor.read(cx).hotkeys_paused);

                // A plain letter would block typing everywhere, so it is refused.
                window.press("a", cx);
                assert_eq!(entity.read(cx).recording, Some(0));
                assert!(entity.read(cx).field_error(Field::Hotkey(0)).is_some());

                window.press(&format!("{PRIMARY}-alt-9"), cx);
                assert_eq!(entity.read(cx).recording, None);
                assert!(!editor.read(cx).hotkeys_paused);
                assert_eq!(
                    editor.read(cx).settings.hotkeys.capture_area,
                    "CommandOrControl+Alt+9"
                );

                // The capture screen keys are taken, so nothing changes.
                window.click(("hotkey", 0_usize), cx);
                window.press(&format!("{PRIMARY}-shift-1"), cx);
                let error = entity.read(cx).field_error(Field::Hotkey(0)).unwrap();
                assert!(error.contains("Capture screen"), "{error}");
                assert_eq!(entity.read(cx).recording, None);
                assert!(!editor.read(cx).hotkeys_paused);
                assert_eq!(
                    editor.read(cx).settings.hotkeys.capture_area,
                    "CommandOrControl+Alt+9"
                );

                // Escape cancels recording and keeps the window open.
                window.click(("hotkey", 1_usize), cx);
                window.press("escape", cx);
                assert_eq!(entity.read(cx).recording, None);
                assert_eq!(
                    editor.read(cx).settings.hotkeys.capture_screen,
                    defaults.capture_screen
                );

                // Backspace turns a shortcut off.
                window.click(("hotkey", 1_usize), cx);
                window.press("backspace", cx);
                assert!(editor.read(cx).settings.hotkeys.capture_screen.is_empty());
                window.render_frame(cx);
                window.click(("hotkey-reset", 1_usize), cx);
                assert_eq!(
                    editor.read(cx).settings.hotkeys.capture_screen,
                    defaults.capture_screen
                );
                scroll_bottom(window, Page::Shortcuts, cx);
                window.render_frame(cx);
                window.click(("hotkey-clear", 7_usize), cx);
                assert!(editor.read(cx).settings.hotkeys.show_editor.is_empty());
            })
            .unwrap();
        assert_eq!(cx.windows().len(), 2);
        cx.run_until_parked();
        let saved = store.load().unwrap().hotkeys;
        assert_eq!(saved.capture_area, "CommandOrControl+Alt+9");
        assert_eq!(saved.capture_screen, defaults.capture_screen);
        assert!(saved.show_editor.is_empty());
        handle
            .update(cx, |_, window, cx| {
                scroll_bottom(window, Page::Shortcuts, cx);
                window.render_frame(cx);
                window.click("reset-hotkeys", cx);
                assert_eq!(editor.read(cx).settings.hotkeys, defaults);
                // Closing the window while recording turns global shortcuts back on.
                window.click(("hotkey", 2_usize), cx);
                assert!(editor.read(cx).hotkeys_paused);
                window.remove_window();
            })
            .unwrap();
        drop(entity);
        cx.run_until_parked();
        assert!(!cx.read(|cx| editor.read(cx).hotkeys_paused));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn shortcut_labels_and_scroll_limit_steps() {
        let primary = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl"
        };
        assert_eq!(
            shortcut_label("CommandOrControl+Shift+2"),
            format!(
                "{primary}+{}+2",
                if cfg!(target_os = "macos") {
                    "⇧"
                } else {
                    "Shift"
                }
            )
        );
        assert_eq!(key_label("KeyA"), "A");
        assert_eq!(key_label("Digit7"), "7");
        assert_eq!(key_label("Up"), "↑");
        assert_eq!(
            hotkey_text(&Keystroke::parse(&format!("{PRIMARY}-f5")).unwrap()).unwrap(),
            "CommandOrControl+F5"
        );
        assert_eq!(hotkey_text(&Keystroke::parse("f5").unwrap()).unwrap(), "F5");
        // Native key events report Shift+1 as "!" without Shift.
        let native = Keystroke::parse(&format!("{PRIMARY}-!")).unwrap();
        assert_eq!(native.key, "!");
        assert!(!native.modifiers.shift);
        assert_eq!(hotkey_text(&native).unwrap(), "CommandOrControl+Shift+1");
        assert!(crate::runtime::parse_hotkey("CommandOrControl+Shift+\\").is_ok());
        assert!(hotkey_text(&Keystroke::parse("shift-a").unwrap()).is_err());
        assert_eq!(
            [40, 45, 2, 200, 10].map(|limit| step_limit(limit, true)),
            [50, 50, 10, 200, 20]
        );
        assert_eq!(
            [40, 45, 2, 200, 10].map(|limit| step_limit(limit, false)),
            [30, 40, 2, 190, 2]
        );
    }

    #[gpui_kit::test]
    fn settings_upload_validates_before_saving_and_supports_s3_without_network_requests(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, path) = fixture(cx, Page::Share);
        let store = SettingsStore::at(&path);
        handle.update(cx, |_, window, cx| {
            scroll_bottom(window, Page::Share, cx);
            window.render_frame(cx);
            window.click("upload-enabled", cx);
            assert!(entity.read(cx).upload_open);
            assert!(editor.read(cx).settings.cloud_upload.is_none());
            entity.read(cx).upload[0].clone().update(cx, |input, cx| input.set_value("not-a-url", window, cx));
            window.render_frame(cx);
            scroll_bottom(window, Page::Share, cx);
            window.render_frame(cx);
            window.click("save-upload", cx);
            assert!(entity.read(cx).field_error(Field::Upload).is_some());
            assert!(editor.read(cx).settings.cloud_upload.is_none());
            assert!(!path.exists());
            window.click("upload-s3", cx);
            for (index, value) in [(2, "screenshots"), (3, "us-east-1"), (4, "https://storage.example.com"), (5, "../bad")] {
                entity.read(cx).upload[index].clone().update(cx, |input, cx| input.set_value(value, window, cx));
            }
            window.render_frame(cx);
            scroll_bottom(window, Page::Share, cx);
            window.render_frame(cx);
            assert!(window.find("aws-credentials").visible());
            window.click("save-upload", cx);
            assert!(entity.read(cx).field_error(Field::Upload).is_some());
            assert!(editor.read(cx).settings.cloud_upload.is_none());
            entity.read(cx).upload[5].clone().update(cx, |input, cx| input.set_value("sniplet/", window, cx));
            window.render_frame(cx);
            window.click("save-upload", cx);
            assert!(entity.read(cx).field_error(Field::Upload).is_none());
            assert!(matches!(editor.read(cx).settings.cloud_upload.clone(), Some(CloudUploadConfig::S3 { bucket, key_prefix, .. }) if bucket == "screenshots" && key_prefix == "sniplet/"));
        }).unwrap();
        cx.run_until_parked();
        assert!(store.load().unwrap().cloud_upload.is_some());
        handle
            .update(cx, |_, window, cx| {
                window.scroll(
                    ("settings-content", Page::Share as usize),
                    ScrollDelta::Pixels(point(px(0.0), px(1600.0))),
                    cx,
                );
                window.render_frame(cx);
                window.click("upload-enabled", cx);
                assert!(!entity.read(cx).upload_open);
                assert!(editor.read(cx).settings.cloud_upload.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        assert!(store.load().unwrap().cloud_upload.is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn settings_scrolling_limit_steps_and_faster_slider_reduces_delay(cx: &mut TestAppContext) {
        let (handle, _, editor, path) = fixture(cx, Page::Capture);
        let store = SettingsStore::at(&path);
        handle
            .update(cx, |_, window, cx| {
                scroll_bottom(window, Page::Capture, cx);
                window.render_frame(cx);
                window.click("scroll-limit-up", cx);
                assert_eq!(editor.read(cx).settings.scroll_max_frames, 50);
                window.render_frame(cx);
                window.click("scroll-limit-down", cx);
                window.render_frame(cx);
                window.click("scroll-limit-down", cx);
                assert_eq!(editor.read(cx).settings.scroll_max_frames, 30);
                window.render_frame(cx);
                window.click_at("scroll-speed", point(px(190.0), px(12.0)), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(store.load().unwrap().scroll_max_frames, 30);
        assert!(store.load().unwrap().scroll_settle_ms < 250);
        std::fs::remove_file(path).unwrap();
    }
}
