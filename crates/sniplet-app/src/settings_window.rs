use gpui_kit::{
    base::Disableable,
    base::slider::{SliderEvent, SliderState},
    component::{
        ActiveTheme, Selectable, TitleBar,
        button::{Button, ButtonVariants},
        input::{Input, InputState},
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    General,
    Hotkeys,
    Uploading,
    Advanced,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Hotkeys => "Hotkeys",
            Self::Uploading => "Uploading",
            Self::Advanced => "Advanced",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::General => "settings",
            Self::Hotkeys => "keyboard",
            Self::Uploading => "cloud-upload",
            Self::Advanced => "sliders-horizontal",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::General => "Appearance, saving, and capture behavior.",
            Self::Hotkeys => "Customize your capture shortcuts.",
            Self::Uploading => "Configure a destination for sharing screenshots.",
            Self::Advanced => "Scrolling capture and editor behavior.",
        }
    }
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
    hotkeys: [Entity<InputState>; 8],
    upload: [Entity<InputState>; 7],
    upload_kind: UploadKind,
    scroll_limit: Entity<InputState>,
    scroll_speed: Entity<SliderState>,
    window_padding: Entity<SliderState>,
    background_color: Entity<InputState>,
    background_previews: [std::sync::Arc<RenderImage>; 4],
    message: String,
    error: bool,
    _theme: Subscription,
    _editor: Subscription,
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
        size(px(900.0), px(710.0)),
        cx,
    )));
    options.window_min_size = Some(size(px(760.0), px(580.0)));
    options.app_id = Some(sniplet_platform::APP_ID.into());
    gpui_kit::open_window(options, cx, move |window, cx| {
        window.set_window_title("Sniplet Settings");
        cx.new(|cx| SettingsWindow::new(editor, settings, page, window, cx))
    })
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
        let hotkeys = hotkey_values(&settings.hotkeys).map(|value| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder("Disabled")
            })
        });
        let mut upload_values = std::array::from_fn::<_, 7, _>(|_| String::new());
        let mut upload_kind = UploadKind::SignedUrl;
        match settings.cloud_upload {
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
            "https://… S3 endpoint",
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
        let scroll_limit = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.scroll_max_frames.to_string())
        });
        let scroll_speed = cx.new(|_| {
            SliderState::new()
                .min(100.0)
                .max(700.0)
                .step(50.0)
                .default_value((800 - settings.scroll_settle_ms.clamp(100, 700)) as f32)
        });
        cx.subscribe(&scroll_speed, |this, _, event, cx| {
            if let SliderEvent::Release(value) = event {
                let milliseconds = 800 - value.end() as u64;
                this.change(cx, |settings| settings.scroll_settle_ms = milliseconds);
            } else {
                cx.notify();
            }
        })
        .detach();
        let window_padding = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(120.0)
                .step(4.0)
                .default_value(settings.window_capture.padding.min(120) as f32)
        });
        cx.subscribe(&window_padding, |this, _, event, cx| {
            if let SliderEvent::Release(value) = event {
                this.change(cx, |settings| {
                    settings.window_capture.padding = value.end() as u32
                });
            } else {
                cx.notify();
            }
        })
        .detach();
        let background_color = cx.new(|cx| {
            InputState::new(window, cx).default_value(color_hex(settings.window_capture.color))
        });
        let background_previews = background_previews(settings.window_capture.color);
        let theme = cx.observe_window_appearance(window, |this, window, cx| {
            let preference = this.editor.read(cx).settings.theme;
            if preference == ThemePreference::System {
                crate::theme::apply(preference, window, cx);
            }
        });
        let editor_subscription = cx.observe(&editor, |_, _, cx| cx.notify());
        Self {
            editor,
            page,
            focus,
            hotkeys,
            upload,
            upload_kind,
            scroll_limit,
            scroll_speed,
            window_padding,
            background_color,
            background_previews,
            message: String::new(),
            error: false,
            _theme: theme,
            _editor: editor_subscription,
        }
    }

    pub(crate) fn select_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.message.clear();
        self.error = false;
        cx.notify();
    }

    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        let (saved, message) = self.editor.update(cx, |editor, cx| {
            change(&mut editor.settings);
            let saved = editor.persist_settings(cx);
            (saved, editor.status.clone())
        });
        self.message = message;
        self.error = !saved;
        self.background_previews =
            background_previews(self.editor.read(cx).settings.window_capture.color);
        cx.notify();
    }

    fn fail(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = message.into();
        self.error = true;
        cx.notify();
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
                let _ = this.update(cx, |this, cx| this.fail(error.to_string(), cx));
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
                    Err(error) => this.fail(error, cx),
                });
            }
        })
        .detach();
    }

    fn save_background_color(&mut self, cx: &mut Context<Self>) {
        let value = self.background_color.read(cx).value();
        let value = value.trim();
        let value = value.strip_prefix('#').unwrap_or(value);
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.fail("Use a six-digit hex color, such as #F2F2F7.", cx);
            return;
        }
        let color = u32::from_str_radix(value, 16).unwrap();
        self.change(cx, |settings| {
            settings.window_capture.color = [(color >> 16) as u8, (color >> 8) as u8, color as u8];
        });
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
                                        rgb(0x0a84ff).into()
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
        section("Window screenshot background", card(cx)
            .child(div().p_3().child(choices))
            .child(div().px_4().pb_3().child(note(
                "Applied to window captures. Padded modes add a soft shadow; Trim shadow keeps the window's exact size.", cx)))
            .when(style.background != WindowBackground::TrimShadow, |card| {
                card.child(divider(cx)).child(row("Padding", "Space around the window at 1× resolution.",
                    div().flex().items_center().gap_3()
                        .child(div().id("window-padding").test_support().w(px(168.0)).h(px(24.0)).flex().items_center().child(Slider::new(&self.window_padding)))
                        .child(div().w(px(44.0)).text_sm().child(format!("{} pt", self.window_padding.read(cx).value().end() as u32))), cx))
            })
            .when(matches!(style.background, WindowBackground::Solid | WindowBackground::Wallpaper), |card| {
                card.child(divider(cx)).child(row(
                    if style.background == WindowBackground::Wallpaper { "Fallback color" } else { "Background color" },
                    if style.background == WindowBackground::Wallpaper { "Used if the wallpaper image is unavailable." } else { "A solid color behind the window and its shadow." },
                    div().flex().items_center().gap_2()
                        .child(div().size(px(22.0)).rounded(px(5.0)).border_1().border_color(cx.theme().border)
                            .bg(rgb((u32::from(style.color[0]) << 16) | (u32::from(style.color[1]) << 8) | u32::from(style.color[2]))))
                        .child(Input::new(&self.background_color).id("window-background-color").w(px(100.0)))
                        .child(Button::new("save-background-color").label("Apply")
                            .on_click(cx.listener(|this, _, _, cx| this.save_background_color(cx)))), cx))
            })
            .when(style.background == WindowBackground::Wallpaper, |card| {
                card.child(divider(cx))
                    .child(row("Wallpaper image", "Use your desktop wallpaper or choose an image.",
                        div().flex().gap_2()
                            .child(Button::new("choose-wallpaper").label("Choose…")
                                .on_click(cx.listener(|this, _, window, cx| this.choose_wallpaper(window, cx))))
                            .child(Button::new("desktop-wallpaper").ghost().label("Use desktop")
                                .disabled(style.wallpaper.is_none())
                                .on_click(cx.listener(|this, _, _, cx| this.change(cx, |settings| settings.window_capture.wallpaper = None)))), cx))
                    .child(div().px_4().pb_3().text_xs().text_color(cx.theme().muted_foreground).truncate()
                        .child(style.wallpaper.as_ref().map_or_else(|| "Desktop wallpaper".into(), |path| path.display().to_string())))
            }), cx).into_any_element()
    }

    fn general(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.editor.read(cx).settings.clone();
        let directory = settings
            .screenshot_directory
            .clone()
            .or_else(|| sniplet_platform::default_export_directory().ok());
        let appearance = div().flex().gap_3().children(
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
            .child(section("Appearance", card(cx).p_3().child(appearance), cx))
            .child(self.window_background(cx))
            .child(section(
                "Saving",
                card(cx)
                    .child(row(
                        "Screenshots folder",
                        "The starting folder when saving a screenshot.",
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
                    .child(row(
                        "Save format",
                        "Choose the default image format.",
                        format,
                        cx,
                    ))
                    .child(divider(cx))
                    .child(self.toggle(
                        "downscale-on-save",
                        "Save Retina screenshots at 1×",
                        "Keep the editor and clipboard at full resolution.",
                        settings.downscale_on_save,
                        |s, v| s.downscale_on_save = v,
                        cx,
                    )),
                cx,
            ))
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
            .into_any_element()
    }

    fn hotkeys(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut captures = card(cx);
        for (index, label) in [
            "Capture area",
            "Capture screen",
            "Capture window",
            "Scrolling capture",
            "Repeat area",
            "Active window",
            "Capture text / QR",
            "Reopen editor",
        ]
        .into_iter()
        .enumerate()
        {
            if index > 0 {
                captures = captures.child(divider(cx));
            }
            captures = captures.child(row(
                label,
                "",
                Input::new(&self.hotkeys[index])
                    .id(("hotkey-input", index))
                    .w(px(250.0)),
                cx,
            ));
        }
        let controls = div().flex().justify_between().items_center().child(
            Button::new("reset-hotkeys")
                .ghost()
                .label("Restore defaults")
                .on_click(cx.listener(|this, _, window, cx| {
                    for (input, value) in this
                        .hotkeys
                        .iter()
                        .zip(hotkey_values(&HotkeySettings::default()))
                    {
                        input.update(cx, |input, cx| input.set_value(value, window, cx));
                    }
                    this.message = "Default shortcuts restored. Save to apply.".into();
                    this.error = false;
                    cx.notify();
                })),
        );
        section("Capture shortcuts", div().flex().flex_col().gap_3().child(captures).child(controls)
            .child(note("Use Ctrl+Shift+2 on Windows/Linux or Cmd+Shift+2 on Mac. Leave a field empty to disable it. Changes apply immediately after saving.", cx)), cx).into_any_element()
    }

    fn save_hotkeys(&mut self, cx: &mut Context<Self>) {
        let values = self
            .hotkeys
            .each_ref()
            .map(|input| input.read(cx).value().trim().to_owned());
        if let Err(error) = crate::runtime::validate_hotkeys(&values) {
            self.fail(error, cx);
            return;
        }
        let [
            capture_area,
            capture_screen,
            capture_window,
            scrolling_capture,
            repeat_area,
            active_window,
            capture_ocr,
            show_editor,
        ] = values;
        self.change(cx, |settings| {
            settings.hotkeys = HotkeySettings {
                capture_area,
                capture_screen,
                capture_window,
                scrolling_capture,
                repeat_area,
                active_window,
                capture_ocr,
                show_editor,
            }
        });
    }

    fn uploading(&self, cx: &mut Context<Self>) -> AnyElement {
        let configured = self.editor.read(cx).settings.cloud_upload.is_some();
        let mut fields = card(cx);
        let descriptors: &[(usize, &str)] = match self.upload_kind {
            UploadKind::SignedUrl => &[(0, "Signed PUT URL"), (1, "Image link (optional)")],
            UploadKind::S3 => &[
                (2, "Bucket"),
                (3, "Region"),
                (4, "Endpoint (optional)"),
                (5, "Folder prefix (optional)"),
                (6, "Public base URL (optional)"),
            ],
        };
        for (index, label) in descriptors {
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .child(div().text_sm().child(*label))
                    .child(Input::new(&self.upload[*index]).id(("upload-input", *index))),
            );
        }
        div().flex().flex_col().gap_5()
            .child(section("Destination", card(cx).p_4().child(div().flex().justify_between().items_center()
                .child(div().flex().flex_col().gap_1().child(if configured { "Uploading enabled" } else { "Uploading is off" })
                    .child(note("Screenshots upload only when you choose Upload.", cx)))
                .when(configured, |div| div.child(Button::new("disable-upload").ghost().label("Disable")
                    .on_click(cx.listener(|this, _, _, cx| this.change(cx, |s| s.cloud_upload = None)))))), cx))
            .child(div().flex().gap_2().children([
                ("upload-signed-url", "Signed URL", UploadKind::SignedUrl), ("upload-s3", "S3 compatible", UploadKind::S3),
            ].map(|(id, label, kind)| Button::new(id).ghost().label(label).selected(self.upload_kind == kind)
                .on_click(cx.listener(move |this, _, _, cx| { this.upload_kind = kind; this.message.clear(); this.error = false; cx.notify(); })))))
            .child(fields)
            .child(note(match self.upload_kind {
                UploadKind::SignedUrl => "Paste a signed upload URL from your provider. A new URL may be needed when it expires.",
                UploadKind::S3 => "Uses AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY from your environment. Optional AWS_SESSION_TOKEN is supported. Leave Endpoint empty for Amazon S3.",
            }, cx))
            .into_any_element()
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
            self.fail(error.to_string(), cx);
            return;
        }
        self.change(cx, |settings| settings.cloud_upload = Some(config));
    }

    fn advanced(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.editor.read(cx).settings.clone();
        let content = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section(
                "Scrolling capture",
                card(cx)
                    .child(row(
                        "Capture limit",
                        "Maximum frames captured in one session (2–200).",
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Input::new(&self.scroll_limit)
                                    .id("scroll-limit")
                                    .w(px(75.0)),
                            )
                            .child(Button::new("save-scroll-limit").label("Apply").on_click(
                                cx.listener(|this, _, _, cx| {
                                    let value = this.scroll_limit.read(cx).value().parse::<usize>();
                                    match value {
                                        Ok(value) if (2..=200).contains(&value) => {
                                            this.change(cx, |s| s.scroll_max_frames = value)
                                        }
                                        _ => this.fail(
                                            "Enter a capture limit between 2 and 200 frames.",
                                            cx,
                                        ),
                                    }
                                }),
                            )),
                        cx,
                    ))
                    .child(divider(cx))
                    .child(row(
                        "Scrolling speed",
                        "Slow down for pages that need time to load.",
                        div()
                            .w(px(210.0))
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
            .child(section(
                "Editor",
                card(cx).child(self.toggle(
                    "always-on-top",
                    "Keep editor on top",
                    "Takes effect after restarting Sniplet.",
                    settings.always_on_top,
                    |s, v| s.always_on_top = v,
                    cx,
                )),
                cx,
            ));
        #[cfg(target_os = "macos")]
        let content = content.child(section(
            "Startup",
            card(cx).child(row(
                "Launch at startup",
                "Open Sniplet when you sign in.",
                Switch::new("launch-at-startup")
                    .checked(crate::macos::launch_at_startup())
                    .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                        match crate::macos::set_launch_at_startup(*enabled) {
                            Ok(true) => {
                                this.message = "Startup preference saved".into();
                                this.error = false;
                                cx.notify();
                            }
                            Ok(false) => this.fail(
                                "Enable Sniplet in System Settings → General → Login Items.",
                                cx,
                            ),
                            Err(error) => this.fail(error, cx),
                        }
                    })),
                cx,
            )),
            cx,
        ));
        content
            .child(section(
                "Preferences",
                card(cx).child(row(
                    "Settings file",
                    "Open the saved preferences in your file manager.",
                    Button::new("reveal-settings")
                        .label("Show in folder")
                        .on_click(|_, _, cx| {
                            if let Ok(store) = sniplet_platform::SettingsStore::for_app() {
                                cx.reveal_path(store.path());
                            }
                        }),
                    cx,
                )),
                cx,
            ))
            .into_any_element()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page {
            Page::General => self.general(cx),
            Page::Hotkeys => self.hotkeys(cx),
            Page::Uploading => self.uploading(cx),
            Page::Advanced => self.advanced(cx),
        };
        let sidebar = div()
            .w(px(188.0))
            .flex_shrink_0()
            .h_full()
            .bg(cx.theme().muted)
            .border_r_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .p_3()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .py_3()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child("SNIPLET"),
            )
            .children(
                [
                    Page::General,
                    Page::Hotkeys,
                    Page::Uploading,
                    Page::Advanced,
                ]
                .into_iter()
                .enumerate()
                .map(|(index, page)| {
                    Button::new(("settings-nav", index))
                        .ghost()
                        .label(page.title())
                        .icon(tool_icon(page.icon()))
                        .selected(self.page == page)
                        .when(self.page == page, |button| {
                            button.bg(rgb(0x0a84ff)).text_color(rgb(0xffffff))
                        })
                        .w_full()
                        .h(px(40.0))
                        .justify_start()
                        .on_click(cx.listener(move |this, _, _, cx| this.select_page(page, cx)))
                }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(concat!("Sniplet ", env!("CARGO_PKG_VERSION"))),
            );
        let registration_error = if self.page == Page::Hotkeys && !self.error {
            self.editor.read(cx).hotkey_error.clone()
        } else {
            None
        };
        let error = self.error || registration_error.is_some();
        let footer = if let Some(error) = registration_error {
            error
        } else if self.message.is_empty() {
            match self.page {
                Page::Hotkeys | Page::Uploading => "Save to apply your changes.",
                _ => "Changes save automatically.",
            }
            .to_owned()
        } else {
            self.message.clone()
        };
        div()
            .id("settings-window")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
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
            .child(
                div().flex().flex_1().min_h_0().child(sidebar).child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .id(("settings-content", self.page as usize))
                                .test_support()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .p_6()
                                .child(
                                    div()
                                        .mb_5()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_2xl()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(self.page.title()),
                                        )
                                        .child(note(self.page.description(), cx)),
                                )
                                .child(page),
                        )
                        .child(
                            div()
                                .id("settings-status")
                                .test_support()
                                .px_6()
                                .py_3()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .text_xs()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(if error {
                                            cx.theme().danger
                                        } else {
                                            cx.theme().muted_foreground
                                        })
                                        .child(footer),
                                )
                                .when(self.page == Page::Hotkeys, |footer| {
                                    footer.child(
                                        Button::new("save-hotkeys")
                                            .primary()
                                            .label("Save shortcuts")
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.save_hotkeys(cx)),
                                            ),
                                    )
                                })
                                .when(self.page == Page::Uploading, |footer| {
                                    footer.child(
                                        Button::new("save-upload")
                                            .primary()
                                            .label("Save destination")
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.save_upload(cx)),
                                            ),
                                    )
                                }),
                        ),
                ),
            )
    }
}

fn optional(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn hotkey_values(keys: &HotkeySettings) -> [String; 8] {
    [
        &keys.capture_area,
        &keys.capture_screen,
        &keys.capture_window,
        &keys.scrolling_capture,
        &keys.repeat_area,
        &keys.active_window,
        &keys.capture_ocr,
        &keys.show_editor,
    ]
    .map(Clone::clone)
}

fn note(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

fn card(cx: &App) -> Div {
    div()
        .rounded(px(12.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .overflow_hidden()
}

fn section(title: &'static str, content: impl IntoElement, cx: &App) -> Div {
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

fn divider(cx: &App) -> Div {
    div().mx_4().h(px(1.0)).bg(cx.theme().border)
}

fn row(title: &'static str, description: &'static str, control: impl IntoElement, cx: &App) -> Div {
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
                .when(!description.is_empty(), |div| {
                    div.child(note(description, cx))
                }),
        )
        .child(div().flex_shrink_0().child(control))
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
            ScrollDelta::Pixels(point(px(0.0), px(-1200.0))),
            cx,
        );
    }

    #[gpui_kit::test]
    fn window_background_choices_padding_and_color_persist_without_changing_open_capture(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, path) = fixture(cx, Page::General);
        let original = cx.read(|cx| editor.read(cx).export_pixels().unwrap());
        let store = SettingsStore::at(&path);
        handle
            .update(cx, |_, window, cx| {
                window.resize(size(px(760.0), px(580.0)));
                window.scroll(
                    ("settings-content", Page::General as usize),
                    ScrollDelta::Pixels(point(px(0.0), px(-180.0))),
                    cx,
                );
                for (id, mode) in [
                    ("window-wallpaper", WindowBackground::Wallpaper),
                    ("window-trim", WindowBackground::TrimShadow),
                    ("window-transparent", WindowBackground::Transparent),
                    ("window-solid", WindowBackground::Solid),
                ] {
                    window.render_frame(cx);
                    window.click(id, cx);
                    assert_eq!(store.load().unwrap().window_capture.background, mode);
                    assert!(window.find(id).bounds().right() <= window.viewport_size().width);
                }
                entity
                    .read(cx)
                    .background_color
                    .clone()
                    .update(cx, |input, cx| input.set_value("invalid", window, cx));
                window.render_frame(cx);
                window.click("save-background-color", cx);
                assert!(entity.read(cx).error);
                assert_eq!(store.load().unwrap().window_capture.color, [242, 242, 247]);
                entity
                    .read(cx)
                    .background_color
                    .clone()
                    .update(cx, |input, cx| input.set_value("#123456", window, cx));
                window.render_frame(cx);
                window.click("save-background-color", cx);
                assert!(!entity.read(cx).error);
                assert_eq!(store.load().unwrap().window_capture.color, [18, 52, 86]);
                window.click_at("window-padding", point(px(140.0), px(12.0)), cx);
            })
            .unwrap();
        assert!(store.load().unwrap().window_capture.padding > 32);
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
        let (handle, entity, editor, path) = fixture(cx, Page::General);
        let wallpaper_path = path.with_extension("wallpaper.png");
        image::RgbaImage::from_pixel(40, 30, image::Rgba([40, 100, 180, 255]))
            .save(&wallpaper_path)
            .unwrap();
        handle
            .update(cx, |_, window, cx| {
                window.scroll(
                    ("settings-content", Page::General as usize),
                    ScrollDelta::Pixels(point(px(0.0), px(-180.0))),
                    cx,
                );
                window.render_frame(cx);
                window.click("window-wallpaper", cx);
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
            assert_eq!(cx.read(|cx| entity.read(cx).error), !candidate.exists());
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
        let (handle, _, editor, path) = fixture(cx, Page::General);
        let store = SettingsStore::at(&path);
        let directory = std::env::temp_dir();
        handle
            .update(cx, |_, window, cx| {
                window.scroll(
                    ("settings-content", Page::General as usize),
                    ScrollDelta::Pixels(point(px(0.0), px(-520.0))),
                    cx,
                );
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
        let (handle, _, editor, _) = fixture(cx, Page::General);
        let parent = cx
            .windows()
            .into_iter()
            .find(|window| *window != handle)
            .unwrap();
        parent
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.open_settings(Page::Uploading, window, cx)
                })
            })
            .unwrap();
        assert_eq!(cx.windows().len(), 2);
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
    fn settings_navigation_fits_small_windows_in_both_themes_and_escape_preserves_editor(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, _) = fixture(cx, Page::General);
        let original = cx.read(|cx| editor.read(cx).export_pixels().unwrap());
        handle
            .update(cx, |_, window, _| window.resize(size(px(760.0), px(580.0))))
            .unwrap();
        handle
            .update(cx, |_, window, cx| {
                for theme in [ThemePreference::Light, ThemePreference::Dark] {
                    crate::theme::apply(theme, window, cx);
                    for (index, page) in [
                        Page::General,
                        Page::Hotkeys,
                        Page::Uploading,
                        Page::Advanced,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        window.render_frame(cx);
                        window.click(("settings-nav", index), cx);
                        assert_eq!(entity.read(cx).page as usize, page as usize);
                        let nav = window.find(("settings-nav", index));
                        assert!(nav.visible());
                        assert!(nav.bounds().right() < px(188.0));
                        let content = window.find(("settings-content", index));
                        assert!(content.bounds().right() <= window.viewport_size().width);
                        assert!(window.find("settings-status").visible());
                        if page == Page::Hotkeys {
                            assert!(window.find("save-hotkeys").visible());
                            for i in 0_usize..8 {
                                assert!(
                                    window.find(("hotkey-input", i)).bounds().right()
                                        <= window.viewport_size().width
                                );
                            }
                            scroll_bottom(window, page, cx);
                            assert!(window.find("save-hotkeys").visible());
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
    fn settings_hotkey_validation_retains_drafts_and_saves_without_restarting(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, path) = fixture(cx, Page::Hotkeys);
        let store = SettingsStore::at(&path);
        handle
            .update(cx, |_, window, cx| {
                let duplicate = editor.read(cx).settings.hotkeys.capture_screen.clone();
                entity.read(cx).hotkeys[0]
                    .clone()
                    .update(cx, |input, cx| input.set_value(duplicate, window, cx));
                window.render_frame(cx);
                scroll_bottom(window, Page::Hotkeys, cx);
                window.click("save-hotkeys", cx);
                assert!(entity.read(cx).error);
                assert_eq!(editor.read(cx).settings.hotkeys, HotkeySettings::default());
                window.click(("settings-nav", Page::General as usize), cx);
                window.click(("settings-nav", Page::Hotkeys as usize), cx);
                assert_eq!(
                    entity.read(cx).hotkeys[0].read(cx).value().as_str(),
                    editor.read(cx).settings.hotkeys.capture_screen
                );
                for (index, value) in [(0, "Control+Alt+9"), (1, "")] {
                    entity.read(cx).hotkeys[index]
                        .clone()
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                }
                window.render_frame(cx);
                scroll_bottom(window, Page::Hotkeys, cx);
                window.click("save-hotkeys", cx);
                assert!(!entity.read(cx).error);
                assert_eq!(store.load().unwrap().hotkeys.capture_area, "Control+Alt+9");
                assert!(store.load().unwrap().hotkeys.capture_screen.is_empty());
                window.click("reset-hotkeys", cx);
                assert_eq!(
                    entity.read(cx).hotkeys[0].read(cx).value().as_str(),
                    HotkeySettings::default().capture_area
                );
                assert_eq!(store.load().unwrap().hotkeys.capture_area, "Control+Alt+9");
            })
            .unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn settings_upload_validates_before_saving_and_supports_s3_without_network_requests(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, editor, path) = fixture(cx, Page::Uploading);
        let store = SettingsStore::at(&path);
        handle.update(cx, |_, window, cx| {
            entity.read(cx).upload[0].clone().update(cx, |input, cx| input.set_value("not-a-url", window, cx));
            window.render_frame(cx);
            window.click("save-upload", cx);
            assert!(entity.read(cx).error);
            assert!(editor.read(cx).settings.cloud_upload.is_none());
            assert!(!path.exists());
            window.click("upload-s3", cx);
            for (index, value) in [(2, "screenshots"), (3, "us-east-1"), (4, "https://storage.example.com"), (5, "../bad")] {
                entity.read(cx).upload[index].clone().update(cx, |input, cx| input.set_value(value, window, cx));
            }
            window.render_frame(cx);
            scroll_bottom(window, Page::Uploading, cx);
            window.click("save-upload", cx);
            assert!(entity.read(cx).error);
            assert!(editor.read(cx).settings.cloud_upload.is_none());
            entity.read(cx).upload[5].clone().update(cx, |input, cx| input.set_value("sniplet/", window, cx));
            window.render_frame(cx);
            window.click("save-upload", cx);
            assert!(!entity.read(cx).error);
            assert!(matches!(store.load().unwrap().cloud_upload, Some(CloudUploadConfig::S3 { bucket, key_prefix, .. }) if bucket == "screenshots" && key_prefix == "sniplet/"));
            window.scroll(("settings-content", Page::Uploading as usize), ScrollDelta::Pixels(point(px(0.0), px(1200.0))), cx);
            window.click("disable-upload", cx);
            assert!(store.load().unwrap().cloud_upload.is_none());
        }).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn settings_scrolling_preferences_validate_limits_and_faster_slider_reduces_delay(
        cx: &mut TestAppContext,
    ) {
        let (handle, entity, _, path) = fixture(cx, Page::Advanced);
        let store = SettingsStore::at(&path);
        handle
            .update(cx, |_, window, cx| {
                for invalid in ["0", "201", "NaN"] {
                    entity
                        .read(cx)
                        .scroll_limit
                        .clone()
                        .update(cx, |input, cx| input.set_value(invalid, window, cx));
                    window.render_frame(cx);
                    window.click("save-scroll-limit", cx);
                    assert!(entity.read(cx).error);
                    assert!(!path.exists());
                }
                entity
                    .read(cx)
                    .scroll_limit
                    .clone()
                    .update(cx, |input, cx| input.set_value("80", window, cx));
                window.render_frame(cx);
                window.click("save-scroll-limit", cx);
                assert_eq!(store.load().unwrap().scroll_max_frames, 80);
                window.click_at("scroll-speed", point(px(200.0), px(12.0)), cx);
            })
            .unwrap();
        assert!(store.load().unwrap().scroll_settle_ms < 250);
        std::fs::remove_file(path).unwrap();
    }
}
