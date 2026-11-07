//! Floating parts of the editor: the app menu, the properties strip above the dock, the
//! side panels, and the recognized text dialog.
use rune::{Color, El, TextStyle, Tx, col, el, row, text, text_input};
use sniplet_core::{
    AnnotationKind, ArrowVariant, Backdrop, Background, Color as ImageColor, Point as ImagePoint,
    Shadow,
};

use super::{BAR, Editor, Panel, hex_color};
use crate::{
    tools::{HideStyle, Tool},
    ui::{BODY, Button, Palette, SEMIBOLD, SMALL, core_color, divider, icons},
};

/// A floating surface: popover fill, border, rounded corners and a soft shadow. Presses on it
/// stay on it rather than reaching the canvas below.
fn surface(p: &Palette) -> El {
    el().fill(p.popover)
        .border(1.0, p.border)
        .radius(8.0)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.18))
        .on_press(|_: &mut Editor| {})
}

/// The quick menu under the app menu button.
pub(super) fn app_menu(s: &Editor, p: &Palette, leading: f32) -> El {
    let mut menu = crate::quick_menu::view(
        p,
        &crate::quick_menu::EDITOR,
        &s.settings.hotkeys,
        |s: &mut Editor, action, tx: &mut Tx| s.choose(action, tx),
    );
    if let Some(label) = s.waiting_update_label() {
        menu = menu.child(
            Button::new("menu-update")
                .primary()
                .icon(&icons::DOWNLOAD)
                .label(label)
                .build(p, |s: &mut Editor, _| s.show_update_notice()),
        );
    }
    el().key("app-menu-popover")
        .absolute(leading + 8.0, BAR - 4.0)
        .radius(8.0)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.18))
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

fn color_swatches(s: &Editor, p: &Palette) -> El {
    row()
        .gap(4.0)
        .center_y()
        .children(COLORS.into_iter().enumerate().map(|(index, color)| {
            // The selected color gets a ring with a gap, so it stays visible on swatches that
            // match the panel or the accent.
            let ring = if s.style.stroke == color {
                p.foreground
            } else {
                Color::TRANSPARENT
            };
            el().key(format!("color-{index}"))
                .role("button")
                .label(format!("Color {}", index + 1))
                .size(27.0, 27.0)
                .padding(2.0)
                .radius(8.0)
                .border(2.0, ring)
                .cursor(rune::Cursor::Pointer)
                .child(
                    el().w_full()
                        .h_full()
                        .radius(6.0)
                        .fill(core_color(color))
                        .border(1.0, p.border),
                )
                .on_press(move |s: &mut Editor| s.change_style(Some(color), None))
        }))
}

/// A row of buttons where the selected one is filled with the accent color.
fn segment(p: &Palette) -> El {
    row()
        .gap(1.0)
        .padding(2.0)
        .radius(6.0)
        .fill(p.foreground.alpha(0.06))
}

/// The properties strip above the dock, for the selected annotation or the current tool.
/// `None` when there is nothing to edit.
pub(super) fn properties(s: &Editor, p: &Palette) -> Option<El> {
    let selected = s.selected_annotation().cloned();
    let subject = selected
        .as_ref()
        .map_or(s.tool, |annotation| Tool::of_kind(&annotation.kind));
    let mut sections: Vec<El> = vec![];
    let label = |value: &str| text(value.to_owned()).font(SMALL).color(p.muted_foreground);

    if !matches!(
        subject,
        Tool::Select | Tool::Hide | Tool::Spotlight | Tool::Ruler | Tool::Ocr
    ) {
        sections.push(color_swatches(s, p));
    }

    if subject == Tool::Hide {
        let active = selected
            .as_ref()
            .and_then(|annotation| HideStyle::of(&annotation.kind))
            .map_or(s.hide, |(style, _)| style);
        sections.push(segment(p).children(HideStyle::ALL.map(|style| {
            Button::new(format!("hide-style-{}", style as usize))
                .compact()
                .icon(style.icon())
                .label(style.label())
                .chosen(active == style)
                .build(p, move |s: &mut Editor, _| s.set_hide(style))
        })));
    }

    if let Some(size) = s.size_target() {
        let value = s.size_value(size);
        sections.push(
            row()
                .gap(8.0)
                .center_y()
                .child(label(size.label()))
                .child(crate::arrow_palette::size_slider(
                    p,
                    size.label(),
                    size.fraction(value),
                    |s: &mut Editor, fraction, phase, _| s.slide_size(fraction, phase),
                ))
                .child(
                    el().key("size-value")
                        .min_w(42.0)
                        .child(text(size.format(value)).font(SMALL).color(p.foreground)),
                ),
        );
    }

    if subject == Tool::Arrow {
        let active = match selected.as_ref().map(|annotation| &annotation.kind) {
            Some(AnnotationKind::Arrow { variant, .. }) => *variant,
            _ => s.arrow_variant,
        };
        sections.push(
            segment(p).children(
                [
                    (ArrowVariant::Solid, "Solid arrow"),
                    (ArrowVariant::HandDrawn, "Hand drawn arrow"),
                    (ArrowVariant::Thin, "Thin arrow"),
                    (ArrowVariant::DoubleEnded, "Double ended arrow"),
                ]
                .map(|(variant, name)| {
                    let on = active == variant;
                    let ink = if on { Color::WHITE } else { p.foreground };
                    row()
                        .key(format!("arrow-variant-{}", variant as usize))
                        .role("button")
                        .label(name)
                        .help(name)
                        .checked(on)
                        .size(34.0, 30.0)
                        .center()
                        .radius(6.0)
                        .fill(if on { p.accent } else { Color::TRANSPARENT })
                        .hover(if on { p.accent } else { p.hover })
                        .cursor(rune::Cursor::Pointer)
                        .child(crate::arrow_palette::variant_icon(variant, ink))
                        .on_press(move |s: &mut Editor| s.change_arrow_variant(variant))
                }),
            ),
        );
    }

    if matches!(subject, Tool::Rectangle | Tool::Ellipse) {
        sections.push(
            Button::new("fill")
                .icon(&icons::PAINT_BUCKET)
                .help("Toggle fill")
                .selected(s.style.fill.is_some())
                .build(p, |s: &mut Editor, _| {
                    s.change_style(None, Some(s.style.fill.is_none()))
                }),
        );
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
                    Button::new("counter-value-down")
                        .compact()
                        .icon(&icons::MINUS)
                        .help("Lower number")
                        .build(p, |s: &mut Editor, _| s.adjust_counter_value(-1)),
                )
                .child(
                    el().min_w(18.0)
                        .child(text(value.to_string()).font(BODY).color(p.foreground)),
                )
                .child(
                    Button::new("counter-value-up")
                        .compact()
                        .icon(&icons::PLUS)
                        .help("Raise number")
                        .build(p, |s: &mut Editor, _| s.adjust_counter_value(1)),
                ),
        );
    }

    if subject == Tool::Ruler {
        sections.push(label(
            "Hold 1 for width or 2 for height. Shift includes outer edges.",
        ));
        sections.push(
            Button::new("measure-units")
                .compact()
                .label(if s.physical_units {
                    "Physical pixels"
                } else {
                    "Points"
                })
                .help("Switch between points and physical pixels")
                .build(p, |s: &mut Editor, _| s.toggle_units()),
        );
    }

    if sections.is_empty() {
        return None;
    }
    let mut panel = surface(p).key("properties").padding(8.0);
    panel = panel.child({
        let mut strip = row().gap(8.0).center_y();
        for (index, section) in sections.into_iter().enumerate() {
            if index > 0 {
                strip = strip.child(divider(p, 22.0));
            }
            strip = strip.child(section);
        }
        strip
    });
    Some(panel)
}

/// The side panel at the canvas's top right: text entry, backdrops or shortcuts.
pub(super) fn floating_panel(s: &Editor, p: &Palette, canvas_height: f32) -> Option<El> {
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
        .child(text(title).font(SEMIBOLD).color(p.foreground))
        .child(
            Button::new("close-panel")
                .icon(&icons::X)
                .help("Close")
                .build(p, |s: &mut Editor, tx| {
                    s.panel = None;
                    tx.blur();
                }),
        );
    let mut body = col().gap(8.0).child(header);
    let option = |key: String, name: &str| Button::new(key).label(name);
    match panel {
        Panel::Menu => {}
        Panel::Text => {
            body = body
                .child(
                    text_input(&s.text_input, BODY)
                        .key("annotation-text")
                        .single_line()
                        .w_full()
                        .h(32.0)
                        .px(10.0)
                        .radius(6.0)
                        .border(1.0, p.border)
                        .fill(p.background)
                        .color(p.foreground)
                        .placeholder("Type your annotation…", p.muted_foreground)
                        .label("Annotation text")
                        .on_edit(|s: &mut Editor| &mut s.text_input),
                )
                .child(
                    Button::new("add-text")
                        .primary()
                        .label(if s.text_edit.is_some() {
                            "Update text"
                        } else {
                            "Add text"
                        })
                        .build(p, |s: &mut Editor, tx| s.commit_text(tx)),
                );
        }
        Panel::Backdrop => {
            body = body.child(
                text("Choose a background. Export includes padding, corners, and shadow.")
                    .font(BODY)
                    .color(p.muted_foreground)
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
                body = body.child(option(format!("backdrop-{index}"), name).build(
                    p,
                    move |s: &mut Editor, _| {
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
            body = body.child(option("no-backdrop".into(), "Remove backdrop").build(
                p,
                |s: &mut Editor, _| {
                    s.edit_backdrop(|backdrop| *backdrop = Backdrop::default());
                    s.fit = true;
                },
            ));
            for (name, padding) in [
                ("Compact · 24 px", 24),
                ("Comfortable · 56 px", 56),
                ("Wide · 100 px", 100),
            ] {
                body = body.child(option(format!("padding-{padding}"), name).build(
                    p,
                    move |s: &mut Editor, _| {
                        s.edit_backdrop(|backdrop| backdrop.padding = padding);
                        s.fit = true;
                    },
                ));
            }
            body = body
                .child(
                    option("toggle-corners".into(), "Toggle rounded corners").build(
                        p,
                        |s: &mut Editor, _| {
                            s.edit_backdrop(|b| {
                                b.corner_radius = if b.corner_radius == 0 { 12 } else { 0 }
                            })
                        },
                    ),
                )
                .child(option("toggle-shadow".into(), "Toggle shadow").build(
                    p,
                    |s: &mut Editor, _| {
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
                body = body.child(text(line).font(BODY).color(p.foreground));
            }
        }
    }
    Some(
        surface(p)
            .key("floating-panel")
            .pin(None, Some(10.0), Some(14.0), None)
            .w(340.0)
            .max_h((canvas_height - 32.0).max(120.0))
            .padding(16.0)
            .clip_rounded(8.0)
            .child(body),
    )
}

fn backdrop_shadow() -> Shadow {
    Shadow {
        offset: ImagePoint::new(0.0, 8.0),
        blur_radius: 16.0,
        color: ImageColor::new(0, 0, 0, 75),
    }
}

/// The dialog that shows text read from the image, editable and ready to copy.
pub(super) fn recognized_text(s: &Editor, p: &Palette) -> Option<El> {
    let edit = s.recognized_text.as_ref()?;
    let (width, height) = s.canvas;
    let dialog = col()
        .key("recognized-text-dialog")
        .role("dialog")
        .label("Recognized text")
        .gap(12.0)
        .w((width - 48.0).clamp(240.0, 680.0))
        .padding(16.0)
        .fill(p.popover)
        .border(1.0, p.border)
        .radius(8.0)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.25))
        .modal(false, |s: &mut Editor| s.recognized_text = None)
        .child(
            row()
                .justify_between()
                .center_y()
                .child(text("Recognized text").font(SEMIBOLD).color(p.foreground))
                .child(
                    Button::new("close-recognized-text")
                        .icon(&icons::X)
                        .help("Close · Escape")
                        .build(p, |s: &mut Editor, tx| {
                            s.recognized_text = None;
                            tx.blur();
                        }),
                ),
        )
        .child(
            text("Edit or select the text below.")
                .font(BODY)
                .color(p.muted_foreground),
        )
        .child(
            text_input(edit, TextStyle::new(14.0, 400.0))
                .key("recognized-text")
                .w_full()
                .h((height + BAR - 240.0).clamp(100.0, 360.0))
                .padding(10.0)
                .radius(6.0)
                .border(1.0, p.border)
                .fill(p.background)
                .color(p.foreground)
                .label("Recognized text")
                .on_edit(|s: &mut Editor| s.recognized_text.as_mut().expect("the dialog is open")),
        )
        .child(
            row()
                .justify_between()
                .center_y()
                .gap(12.0)
                .child(
                    // A long clipboard error wraps instead of pushing the button out.
                    el().grow(1.0).min_w(0.0).child(
                        text(s.status.clone())
                            .font(SMALL)
                            .color(p.muted_foreground)
                            .wrap(),
                    ),
                )
                .child(
                    Button::new("copy-recognized-text")
                        .primary()
                        .icon(&icons::COPY)
                        .label("Copy text")
                        .build(p, |s: &mut Editor, _| s.copy_recognized_text()),
                ),
        );
    Some(
        el().key("recognized-text-scrim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center()
            .fill(Color::BLACK.alpha(0.35))
            .child(dialog),
    )
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
