//! Floating parts of the editor: the app menu, the properties strip above the dock, the
//! side panels, and the recognized text dialog.
use rune::{Color, El, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Dialog, Divider, IconButton, IconButtonVariant, Kit,
    TextArea, TextInput,
};
use sniplet_core::{
    AnnotationKind, ArrowVariant, Backdrop, Background, Color as ImageColor, Point as ImagePoint,
    Shadow,
};

use super::{BAR, Editor, Panel, hex_color};
use crate::{
    tools::{HideStyle, Tool},
    ui::{self, core_color, icons},
};

/// The quick menu under the app menu button.
pub(super) fn app_menu(s: &Editor, k: &Kit, leading: f32) -> El {
    let mut menu = crate::quick_menu::view(
        k,
        &crate::quick_menu::EDITOR,
        &s.settings.hotkeys,
        |s: &mut Editor, action, tx: &mut Tx| s.choose(action, tx),
    );
    if let Some(label) = s.waiting_update_label() {
        menu = menu.child(
            Button::new(label)
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Large)
                .icon(&icons::DOWNLOAD)
                .on_press(|s: &mut Editor| s.show_update_notice())
                .build(&ui::accent(k))
                .key("menu-update"),
        );
    }
    k.elevate(
        el().key("app-menu-popover")
            .absolute(leading + 8.0, BAR - 4.0)
            .radius(k.radius.lg),
        &k.elevation.floating,
    )
    .modal(true, |s: &mut Editor| s.panel = None)
    .child(menu)
}

const COLORS: [ImageColor; 8] = [
    ImageColor::new(255, 0, 0, 255),
    ImageColor::new(255, 153, 0, 255),
    ImageColor::new(255, 218, 48, 255),
    ImageColor::new(39, 174, 96, 255),
    ImageColor::new(0, 122, 255, 255),
    ImageColor::new(158, 71, 238, 255),
    ImageColor::WHITE,
    ImageColor::BLACK,
];

fn color_swatches(s: &Editor, k: &Kit) -> El {
    row()
        .role("radiogroup")
        .label("Color")
        .gap(4.0)
        .center_y()
        .children(COLORS.into_iter().enumerate().map(|(index, color)| {
            // The selected color gets a ring with a gap, so it stays visible on swatches that
            // match the panel or the accent.
            let on = s.style.stroke == color;
            el().key(format!("color-{index}"))
                .role("radio")
                .label(format!("Color {}", index + 1))
                .checked(on)
                .size(26.0, 26.0)
                .padding(2.0)
                .radius(8.0)
                .border(
                    2.0,
                    if on {
                        k.c.text_base
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .cursor(rune::Cursor::Pointer)
                .child(
                    el().w_full()
                        .h_full()
                        .radius(6.0)
                        .fill(core_color(color))
                        .border(1.0, k.c.border_base),
                )
                .on_press(move |s: &mut Editor| s.change_style(Some(color), None))
        }))
}

/// A ghost icon button in the properties strip.
fn strip_icon(glyph: &'static rune::Icon, help: &str) -> IconButton {
    IconButton::new(glyph)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Large)
        .help(help)
}

/// The properties strip above the dock, for the selected annotation or the current tool.
/// `None` when there is nothing to edit.
pub(super) fn properties(s: &Editor, k: &Kit) -> Option<El> {
    let selected = s.selected_annotation().cloned();
    let subject = selected
        .as_ref()
        .map_or(s.tool, |annotation| Tool::of_kind(&annotation.kind));
    let mut sections: Vec<El> = vec![];
    let label = |value: &str| {
        text(value.to_owned())
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted)
    };

    if !matches!(
        subject,
        Tool::Select | Tool::Hide | Tool::Spotlight | Tool::Ruler | Tool::Ocr
    ) {
        sections.push(color_swatches(s, k));
    }

    if subject == Tool::Hide {
        let active = selected
            .as_ref()
            .and_then(|annotation| HideStyle::of(&annotation.kind))
            .map_or(s.hide, |(style, _)| style);
        sections.push(
            ui::segments(k, "Hide style").children(HideStyle::ALL.map(|style| {
                let on = active == style;
                ui::segment(
                    k,
                    format!("hide-style-{}", style as usize),
                    style.label(),
                    on,
                )
                .child(icon(style.icon(), 16.0).color(k.c.icon_base))
                .child(text(style.label()).font(k.ty.ui).color(if on {
                    k.c.text_base
                } else {
                    k.c.text_muted
                }))
                .on_press(move |s: &mut Editor| s.set_hide(style))
            })),
        );
    }

    if let Some(size) = s.size_target() {
        let value = s.size_value(size);
        sections.push(
            row()
                .gap(8.0)
                .center_y()
                .child(label(size.label()))
                .child(crate::arrow_palette::size_slider(
                    k,
                    size.label(),
                    size.fraction(value),
                    |s: &mut Editor, fraction, phase, _| s.slide_size(fraction, phase),
                    Editor::cancel_size_edit,
                ))
                .child(
                    el().key("size-value").min_w(42.0).child(
                        text(size.format(value))
                            .font(k.ty.text_xsmall)
                            .color(k.c.text_base),
                    ),
                ),
        );
    }

    if subject == Tool::Arrow {
        let active = match selected.as_ref().map(|annotation| &annotation.kind) {
            Some(AnnotationKind::Arrow { variant, .. }) => *variant,
            _ => s.arrow_variant,
        };
        sections.push(
            ui::segments(k, "Arrow style").children(
                [
                    (ArrowVariant::Solid, "Solid arrow"),
                    (ArrowVariant::HandDrawn, "Hand drawn arrow"),
                    (ArrowVariant::Thin, "Thin arrow"),
                    (ArrowVariant::DoubleEnded, "Double ended arrow"),
                ]
                .map(|(variant, name)| {
                    let on = active == variant;
                    ui::segment(k, format!("arrow-variant-{}", variant as usize), name, on)
                        .help(name)
                        .child(crate::arrow_palette::variant_icon(
                            variant,
                            if on { k.c.icon_base } else { k.c.icon_muted },
                        ))
                        .on_press(move |s: &mut Editor| s.change_arrow_variant(variant))
                }),
            ),
        );
    }

    if matches!(subject, Tool::Rectangle | Tool::Ellipse) {
        let filled = s.style.fill.is_some();
        let button = strip_icon(&icons::PAINT_BUCKET, "Toggle fill")
            .on_press(|s: &mut Editor| s.change_style(None, Some(s.style.fill.is_none())))
            .build(k);
        sections.push(ui::selected(button, k, filled).key("fill").focusable(false));
    }

    if let Some(AnnotationKind::Counter { value, .. }) =
        selected.as_ref().map(|annotation| &annotation.kind)
    {
        sections.push(
            row()
                .gap(4.0)
                .center_y()
                .child(label("Number"))
                .child(
                    strip_icon(&icons::MINUS, "Lower number")
                        .control_size(ControlSize::Regular)
                        .on_press(|s: &mut Editor| s.adjust_counter_value(-1))
                        .build(k)
                        .key("counter-value-down")
                        .focusable(false),
                )
                .child(
                    el().min_w(18.0).center().child(
                        text(value.to_string())
                            .font(k.ty.ui_medium)
                            .color(k.c.text_base),
                    ),
                )
                .child(
                    strip_icon(&icons::PLUS, "Raise number")
                        .control_size(ControlSize::Regular)
                        .on_press(|s: &mut Editor| s.adjust_counter_value(1))
                        .build(k)
                        .key("counter-value-up")
                        .focusable(false),
                ),
        );
    }

    if subject == Tool::Ruler {
        sections.push(label(
            "Hold 1 for width or 2 for height. Shift includes outer edges.",
        ));
        sections.push(
            Button::new(if s.physical_units {
                "Physical pixels"
            } else {
                "Points"
            })
            .variant(ButtonVariant::Outline)
            .help("Switch between points and physical pixels")
            .on_press(|s: &mut Editor| s.toggle_units())
            .build(k)
            .key("measure-units")
            .focusable(false),
        );
    }

    if sections.is_empty() {
        return None;
    }
    let mut strip = row().gap(8.0).center_y().padding(8.0);
    for (index, section) in sections.into_iter().enumerate() {
        if index > 0 {
            strip = strip.child(Divider::vertical().build(k).h(22.0));
        }
        strip = strip.child(section);
    }
    Some(ui::surface::<Editor>(k, strip).key("properties"))
}

/// The side panel at the canvas's top right: text entry, backdrops or shortcuts.
pub(super) fn floating_panel(s: &Editor, k: &Kit, canvas_height: f32) -> Option<El> {
    let panel = s.panel?;
    let title = match panel {
        Panel::Menu => return None,
        Panel::Text => "Text annotation",
        Panel::Backdrop => "Backdrop",
        Panel::Help => "Keyboard shortcuts",
    };
    let header = row()
        .justify_between()
        .center_y()
        .child(text(title).font(k.ty.body_medium).color(k.c.text_base))
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::Ghost)
                .label("Close")
                .on_press_tx(|s: &mut Editor, tx| {
                    s.panel = None;
                    tx.blur();
                })
                .build(k)
                .key("close-panel")
                .focusable(false),
        );
    let mut body = col().gap(8.0).child(header);
    // A full-width ghost row for each choice in a panel.
    let option = |key: String, name: &str| {
        Button::new(name)
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Large)
            .build(k)
            .key(key)
            .focusable(false)
            .w_full()
            .justify_start()
    };
    match panel {
        Panel::Menu => {}
        Panel::Text => {
            body = body
                .child(
                    TextInput::new(&s.text_input)
                        .label("Annotation text")
                        .placeholder("Type your annotation…")
                        .large()
                        .width(None)
                        .on_edit(|s: &mut Editor| &mut s.text_input)
                        .on_submit(|s: &mut Editor, tx| s.commit_text(tx))
                        .build(k)
                        .key("annotation-text"),
                )
                .child(
                    Button::new(if s.text_edit.is_some() {
                        "Update text"
                    } else {
                        "Add text"
                    })
                    .variant(ButtonVariant::Contrast)
                    .control_size(ControlSize::Large)
                    .on_press_tx(|s: &mut Editor, tx| s.commit_text(tx))
                    .build(k)
                    .key("add-text")
                    .focusable(false),
                );
        }
        Panel::Backdrop => {
            body = body.child(
                text("Choose a background. Export includes padding, corners, and shadow.")
                    .font(k.ty.ui)
                    .color(k.c.text_muted)
                    .wrap(),
            );
            for (index, (name, a, b)) in [
                ("Aurora", 0x667eea, 0xc7a4ef),
                ("Sunset", 0xffa57b, 0xffd5a5),
                ("Ocean", 0x58c6e7, 0xccf4ef),
                ("Paper", 0xf1f1f1, 0xffffff),
                ("Midnight", 0x202737, 0x53627a),
            ]
            .into_iter()
            .enumerate()
            {
                body = body.child(option(format!("backdrop-{index}"), name).on_press(
                    move |s: &mut Editor| {
                        s.edit_backdrop(|backdrop| {
                            *backdrop = Backdrop {
                                padding: 56,
                                corner_radius: 12,
                                background: Background::LinearGradient {
                                    start: hex_color(a),
                                    end: hex_color(b),
                                    angle_degrees: 135.0,
                                },
                                shadow: Some(backdrop_shadow()),
                            }
                        });
                        s.fit = true;
                    },
                ));
            }
            body = body.child(option("no-backdrop".into(), "Remove backdrop").on_press(
                |s: &mut Editor| {
                    s.edit_backdrop(|backdrop| *backdrop = Backdrop::default());
                    s.fit = true;
                },
            ));
            body = body.child(Divider::horizontal().build(k));
            for (name, padding) in [
                ("Compact · 24 px", 24),
                ("Comfortable · 56 px", 56),
                ("Wide · 100 px", 100),
            ] {
                body = body.child(option(format!("padding-{padding}"), name).on_press(
                    move |s: &mut Editor| {
                        s.edit_backdrop(|backdrop| backdrop.padding = padding);
                        s.fit = true;
                    },
                ));
            }
            body = body
                .child(Divider::horizontal().build(k))
                .child(
                    option("toggle-corners".into(), "Toggle rounded corners").on_press(
                        |s: &mut Editor| {
                            s.edit_backdrop(|b| {
                                b.corner_radius = if b.corner_radius == 0 { 12 } else { 0 }
                            })
                        },
                    ),
                )
                .child(option("toggle-shadow".into(), "Toggle shadow").on_press(
                    |s: &mut Editor| {
                        s.edit_backdrop(|b| {
                            b.shadow = if b.shadow.is_some() {
                                None
                            } else {
                                Some(backdrop_shadow())
                            }
                        })
                    },
                ));
        }
        Panel::Help => {
            for line in [
                "V Select / Crop    Enter Apply crop",
                "A Arrow · drag its middle handle to bend",
                "T Text    R Rectangle    O Oval    L Line",
                "C Counter / average color",
                "B Hide · press again for Blur, Pixelate, Erase, Redact",
                "H Highlighter    S Spotlight",
                "Z + click Quick zoom    Alt Zoom out",
                "Shift Constrain shapes / angles",
                "Space + drag or middle-drag Pan",
                "Ctrl/Cmd + wheel Zoom at cursor",
                "Tab Copy sampled color    Shift Tab Copy darkest nearby",
                "Ctrl/Cmd + click Select a monotone region",
                "Delete Remove selection / annotation",
                "Ctrl/Cmd Z Undo    Ctrl/Cmd Shift Z Redo",
                "Ctrl/Cmd C Copy    Ctrl/Cmd S Save    Ctrl/Cmd O Open/OCR",
                "Ctrl/Cmd 1 Fit    Ctrl/Cmd 0 Actual size",
                "Arrow keys Nudge    Shift 10 pixels",
                "Escape Cancel / deselect",
            ] {
                body = body.child(text(line).font(k.ty.ui).color(k.c.text_base));
            }
        }
    }
    // A short window scrolls the panel rather than cutting off its last controls.
    let height = (canvas_height - 32.0).max(120.0);
    let viewport = col()
        .key("floating-panel-scroll")
        .w_full()
        .max_h(height)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(body.padding(16.0));
    Some(
        ui::surface::<Editor>(k, el())
            .key("floating-panel")
            .pin(None, Some(10.0), Some(14.0), None)
            .w(340.0)
            .max_h(height)
            .clip_rounded(k.radius.lg)
            .child(k.scroll_bar(viewport)),
    )
}

fn backdrop_shadow() -> Shadow {
    Shadow {
        offset: ImagePoint::new(0.0, 8.0),
        blur_radius: 16.0,
        color: ImageColor::new(0, 0, 0, 75),
    }
}

/// The dialog that shows text read from the image, editable and ready to copy. It mounts in
/// the window's overlay `layer`.
pub(super) fn recognized_text(s: &Editor, k: &Kit, layer: &mut Vec<El>) {
    let Some(edit) = s.recognized_text.as_ref() else {
        return;
    };
    let (width, height) = s.canvas;
    let field = TextArea::new(edit)
        .label("Recognized text")
        .width(None)
        .rows(40)
        .max_height((height + BAR - 240.0).clamp(100.0, 360.0))
        .on_edit(|s: &mut Editor| s.recognized_text.as_mut().expect("the dialog is open"))
        // Return breaks the line, as in any text editor; the dialog has no submit.
        .on_submit(|s: &mut Editor, _| {
            if let Some(edit) = &mut s.recognized_text {
                edit.insert("\n");
            }
        })
        .build(k)
        .key("recognized-text");
    let footer = row()
        .justify_between()
        .center_y()
        .gap(12.0)
        .child(
            // A long clipboard error wraps instead of pushing the button out.
            el().grow(1.0).min_w(0.0).child(
                text(s.status.clone())
                    .font(k.ty.text_xsmall)
                    .color(k.c.text_muted)
                    .wrap(),
            ),
        )
        .child(
            Button::new("Copy text")
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Large)
                .icon(&icons::COPY)
                .on_press(|s: &mut Editor| s.copy_recognized_text())
                .build(k)
                .key("copy-recognized-text"),
        );
    Dialog::new("recognized-text", "Recognized text")
        .description("Edit or select the text below.")
        .open(true)
        .fit()
        .width((width - 48.0).clamp(240.0, 680.0))
        .click_outside(false)
        .on_close(|s: &mut Editor| s.recognized_text = None)
        .child(col().gap(12.0).px(16.0).pb(16.0).child(field).child(footer))
        .build(k, layer);
}

impl Editor {
    fn edit_backdrop(&mut self, change: impl FnOnce(&mut Backdrop)) {
        if let Some(doc) = &mut self.document {
            let mut backdrop = doc.backdrop();
            change(&mut backdrop);
            doc.set_backdrop(backdrop);
        }
        self.refresh();
    }
}
