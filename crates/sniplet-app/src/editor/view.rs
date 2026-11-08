//! The editor window: the toolbar in the title bar, and the canvas with its dock.
use rune::{Color, Cursor, Cx, El, KeyEvent, PointerButton, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Divider, IconButton, IconButtonVariant, Keybind, Kit,
    WindowTitlebar,
};
use sniplet_core::{ColorFormat, Point as ImagePoint};

use super::{BAR, DOCK_BOTTOM, DOCK_HEIGHT, Editor, Keystroke, Panel};
use crate::{
    tools::Tool,
    ui::{self, core_color, icons, rgb},
};

/// The editor window's view.
pub(crate) fn view(s: &Editor, cx: &mut Cx<Editor>) -> El {
    let k = ui::window_kit(cx, s.settings.theme);
    if s.updates.installing() {
        return crate::updates::installing_screen(&k);
    }
    let insets = cx.chrome_insets();
    let mut root = col()
        .key("editor")
        .w_full()
        .h_full()
        .fill(k.c.page)
        .on_key_up(|s: &mut Editor, event: &KeyEvent, tx: &mut Tx| {
            Keystroke::new(event, tx.key_base(), false).is_some_and(|key| s.key_up(&key))
        })
        .on_window_focus(|s: &mut Editor, focused: bool, _: &mut Tx| {
            if !focused {
                s.deactivated();
            }
        })
        .on_drop_files(|s: &mut Editor, paths: &[std::path::PathBuf], _: &mut Tx| {
            if let Some(path) = paths.first() {
                s.load_path(path.clone());
            }
        })
        .child(toolbar(s, &k, insets))
        .child(Divider::horizontal().build(&k))
        .child(canvas(s, &k));
    if s.panel == Some(Panel::Menu) {
        root = root.child(super::panels::app_menu(s, &k, insets.leading));
    }
    let mut layer = Vec::new();
    super::panels::recognized_text(s, &k, &mut layer);
    rune_kit::root(root, layer)
}

/// Keys nothing focused took, or every key while nothing has focus.
pub(crate) fn on_key(s: &mut Editor, event: &KeyEvent, tx: &mut Tx) -> bool {
    Keystroke::new(event, tx.key_base(), tx.key_repeat()).is_some_and(|key| s.key(&key, tx))
}

/// A ghost icon button in the toolbar's size.
fn toolbar_icon(glyph: &'static rune::Icon, help: &str) -> IconButton {
    IconButton::new(glyph)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Large)
        .help(help)
}

fn toolbar(s: &Editor, k: &Kit, insets: rune::ChromeInsets) -> El {
    let zoom = s.transform().zoom();
    let mut menu = toolbar_icon(&icons::SCISSORS, "Sniplet · capture and settings")
        .expanded(s.panel == Some(Panel::Menu))
        .on_press(|s: &mut Editor| {
            s.panel = if s.panel == Some(Panel::Menu) {
                None
            } else {
                Some(Panel::Menu)
            };
        })
        .build(k)
        .key("app-menu");
    if s.waiting_update_label().is_some() {
        menu = el().child(menu).child(
            el().key("update-dot")
                .size(7.0, 7.0)
                .radius(3.5)
                .fill(k.c.bg_accent)
                .pin(None, Some(3.0), Some(3.0), None),
        );
    }
    let ghost = |label: String| {
        Button::new(label)
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Large)
    };
    WindowTitlebar::from_insets(insets, 1.0)
        .key("editor-toolbar")
        .height(BAR)
        .leading_padding(8.0)
        .trailing_padding(8.0)
        .gap(4.0)
        .fill(k.c.bg_base)
        .leading(menu)
        .trailing(
            ghost(format!("{:.0}%", zoom * 100.0))
                .help(format!("{} · Fit", s.image_size_label()))
                .shortcut("mod+1")
                .on_press_tx(|s: &mut Editor, tx| s.command(super::Command::Fit, tx))
                .build(k)
                .key("zoom-fit"),
        )
        .trailing(Divider::vertical().build(k).h(20.0).mx(4.0))
        .trailing(
            ghost("Backdrop".into())
                .icon(&icons::LAYERS)
                .expanded(s.panel == Some(Panel::Backdrop))
                .on_press(|s: &mut Editor| s.panel = Some(Panel::Backdrop))
                .build(k)
                .key("backdrop"),
        )
        .trailing(drag_file(s, k))
        .trailing(
            toolbar_icon(&icons::PIN, "Pin image on screen")
                .on_press_tx(|s: &mut Editor, tx| s.command(super::Command::Pin, tx))
                .build(k)
                .key("pin"),
        )
        .trailing(
            toolbar_icon(&icons::SAVE, "Save image")
                .shortcut("mod+s")
                .on_press_tx(|s: &mut Editor, tx| s.command(super::Command::Save, tx))
                .build(k)
                .key("save"),
        )
        .trailing(
            Button::new("Copy")
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Large)
                .icon(&icons::COPY)
                .help("Copy image")
                .shortcut("mod+c")
                .on_press_tx(|s: &mut Editor, tx| s.command(super::Command::Copy, tx))
                .build(&ui::accent(k))
                .key("copy")
                .ml(4.0),
        )
        .build(k)
}

/// The pill that drags the exported image into another app as a PNG file.
fn drag_file(s: &Editor, k: &Kit) -> El {
    let grip = || {
        row().gap(2.5).children((0..2).map(|_| {
            col()
                .gap(2.5)
                .children((0..3).map(|_| el().size(1.5, 1.5).radius(0.75).fill(k.c.icon_faint)))
        }))
    };
    let control = row()
        .key("drag-file")
        .role("button")
        .label("Drag'n'Drop Image")
        .help("Drag'n'Drop Image")
        .w(44.0)
        .h(26.0)
        .gap(3.0)
        .center()
        .radius(13.0)
        .fill(k.c.layer_02)
        .box_shadow(rune::Shadow {
            spread: 0.5,
            color: k.c.border_base,
            ..Default::default()
        })
        .child(grip())
        .child(icon(&icons::FILE, 16.0).color(k.c.icon_base))
        .child(grip());
    if s.document.is_none() {
        return control.layer_opacity(0.5);
    }
    crate::drag_out::source(
        k.interactive(control).cursor(Cursor::Grab),
        |s: &Editor| match s.export_drag_file() {
            Ok(path) => Ok(path),
            Err(error) => Err(format!("Could not drag image: {error}")),
        },
        |s: &mut Editor, error: String| s.status = error,
    )
}

fn canvas(s: &Editor, k: &Kit) -> El {
    let (width, height) = s.canvas;
    let mut canvas = el()
        .key("editor-canvas")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(k.c.layer_02)
        .clip_rounded(0.0)
        .pointer_buttons([PointerButton::Middle])
        .on_layout(|s: &mut Editor, rect: [f32; 4]| s.canvas = (rect[2], rect[3]))
        .on_drag(|s: &mut Editor, drag: &mut rune::Drag, tx: &mut Tx| s.canvas_drag(drag, tx))
        .on_pointer_move_local(|s: &mut Editor, event: rune::PointerEvent, tx: &mut Tx| {
            s.canvas_hover(event.x, event.y, tx.mods().shift)
        })
        .on_wheel(|s: &mut Editor, event: rune::WheelEvent, _: &mut Tx| s.wheel(event));
    if s.document.is_some() && s.image_size.0 > 0 {
        canvas = image_layers(s, k, canvas, height);
        if let Some(properties) = super::panels::properties(s, k) {
            // Keyed, so overlays added before it (a measurement, handles) do not remount it.
            canvas = canvas.child(
                row()
                    .key("properties-bar")
                    .pin(
                        Some(0.0),
                        None,
                        Some(0.0),
                        Some(DOCK_BOTTOM + DOCK_HEIGHT + 10.0),
                    )
                    .justify_center()
                    .child(properties),
            );
        }
        canvas = canvas
            .child(dock(s, k))
            .children(color_peek(s, k, width, height));
    } else if s.document.is_none() {
        canvas = canvas.child(welcome(s, k));
    }
    canvas = canvas.child(
        ui::chip(
            k,
            text(s.status.clone())
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted),
        )
        .key("status")
        .absolute(12.0, 12.0)
        .max_w((width - 24.0).max(0.0)),
    );
    canvas
        .children(s.render_update_notice(k))
        .children(super::panels::floating_panel(s, k, height))
}

/// The image, the measurement overlay, and the selection or edit handles.
fn image_layers(s: &Editor, k: &Kit, mut canvas: El, height: f32) -> El {
    let transform = s.transform();
    let (pan, zoom) = (transform.pan(), transform.zoom());
    canvas = canvas.child(
        el().key("preview")
            .absolute(pan.x, pan.y)
            .size(s.image_size.0 as f32 * zoom, s.image_size.1 as f32 * zoom)
            .pixels(&s.pixels),
    );
    if s.measure.as_ref().is_some_and(|m| m.measurement.is_some())
        && let Some(bounds) = s.measure_overlay
    {
        let origin = s.canvas_point(ImagePoint::new(bounds.x, bounds.y));
        canvas = canvas.child(
            el().key("measure-preview")
                .absolute(origin.x, origin.y)
                .size(bounds.width * zoom, bounds.height * zoom)
                .pixels(&s.measure_pixels),
        );
    }
    // Handles sit on the image, so they keep one look in both appearances.
    let handle = |key: String, point: ImagePoint, size: f32| {
        el().key(key)
            .absolute(point.x - size / 2.0, point.y - size / 2.0)
            .size(size, size)
            .fill(Color::WHITE)
            .border(1.0, rgb(0x777777))
            .radius(2.0)
            .cursor(Cursor::Crosshair)
    };
    let selected = s.selected_annotation();
    let arrow_points = selected.and_then(|annotation| annotation.kind.arrow_points());
    let magnifier_circles = selected.and_then(|annotation| annotation.kind.magnifier_circles());
    let selection = s.selection.or_else(|| selected.map(|a| a.kind.bounds()));
    if s.selection.is_none()
        && let Some(points) = arrow_points
    {
        for (index, point) in points.into_iter().enumerate() {
            canvas = canvas.child(handle(
                format!("arrow-handle-{index}"),
                s.canvas_point(point),
                8.0,
            ));
        }
    } else if s.selection.is_none()
        && let Some(circles) = magnifier_circles
    {
        for (index, circle) in circles.into_iter().enumerate() {
            let center = ImagePoint::new(
                circle.x + circle.width * 0.5,
                circle.y + circle.height * 0.5,
            );
            let edge = ImagePoint::new(circle.x + circle.width, center.y);
            for (control, point) in [center, edge].into_iter().enumerate() {
                canvas = canvas.child(handle(
                    format!("magnifier-handle-{}", index * 2 + control),
                    s.canvas_point(point),
                    8.0,
                ));
            }
        }
    } else if let Some(rect) = selection {
        let origin = s.canvas_point(ImagePoint::new(rect.x, rect.y));
        let (w, h) = (rect.width * zoom, rect.height * zoom);
        let mut outline = el()
            .key("selection")
            .absolute(origin.x, origin.y)
            .size(w.max(1.0), h.max(1.0))
            .border(1.0, k.c.bg_accent);
        for (index, (x, y)) in [
            (0.0, 0.0),
            (w / 2.0, 0.0),
            (w, 0.0),
            (0.0, h / 2.0),
            (w, h / 2.0),
            (0.0, h),
            (w / 2.0, h),
            (w, h),
        ]
        .into_iter()
        .enumerate()
        {
            outline = outline.child(handle(
                format!("selection-handle-{index}"),
                ImagePoint::new(x, y),
                6.0,
            ));
        }
        let hint = if s.tool == Tool::Ocr && s.drag.is_some() {
            " · Release to recognize"
        } else if s.tool == Tool::Ocr {
            " · Ctrl/Cmd O to recognize again"
        } else if s.selection.is_some() {
            " · Enter to crop"
        } else {
            ""
        };
        canvas = canvas.child(outline).child(
            ui::chip(
                k,
                text(format!("{:.0} × {:.0} px{hint}", rect.width, rect.height))
                    .font(k.ty.text_xsmall)
                    .color(k.c.text_base)
                    .single_line(),
            )
            .key("selection-size")
            .absolute(origin.x, (origin.y + h + 10.0).min(height - 30.0)),
        );
    }
    canvas
}

/// Every tool, in a dock at the bottom center of the canvas.
fn dock(s: &Editor, k: &Kit) -> El {
    let tools = Tool::ALL.map(|tool| {
        let mut button = IconButton::new(tool.icon())
            .variant(IconButtonVariant::Ghost)
            .control_size(ControlSize::ExtraLarge)
            .help(tool.label());
        if let Some(key) = tool.shortcut() {
            button = button.shortcut(key);
        }
        let button = button
            .on_press(move |s: &mut Editor| s.select_tool(tool))
            .build(k);
        ui::selected(button, k, s.tool == tool).key(format!("tool-{}", tool as usize))
    });
    row()
        .key("dock-bar")
        .pin(Some(0.0), None, Some(0.0), Some(DOCK_BOTTOM))
        .justify_center()
        .child(
            ui::surface::<Editor>(k, row().gap(2.0).padding(5.0))
                .key("tool-dock")
                .radius(14.0)
                .children(tools),
        )
}

/// The sampled color next to the pointer, shown while Tab or C is held.
fn color_peek(s: &Editor, k: &Kit, width: f32, height: f32) -> Option<El> {
    if !s.color_peek {
        return None;
    }
    // The label opens below and right of the pointer, and flips to the other side near the
    // canvas edges so it is never clipped.
    let point = s.canvas_point(s.cursor);
    let (label_width, label_height) = (110.0, 30.0);
    let x = if point.x + 18.0 + label_width > width {
        point.x - 12.0 - label_width
    } else {
        point.x + 18.0
    };
    let y = if point.y + 18.0 + label_height > height {
        point.y - 12.0 - label_height
    } else {
        point.y + 18.0
    };
    Some(
        ui::chip(
            k,
            row()
                .gap(8.0)
                .center_y()
                .child(
                    el().size(14.0, 14.0)
                        .radius(7.0)
                        .fill(core_color(s.sampled))
                        .border(1.0, k.c.border_base),
                )
                .child(
                    text(s.sampled.format(ColorFormat::HexRgb))
                        .font(k.ty.mono)
                        .color(k.c.text_base),
                ),
        )
        .key("color-peek")
        .absolute(x, y),
    )
}

fn welcome(s: &Editor, k: &Kit) -> El {
    let mut welcome = col()
        .key("welcome")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .gap(12.0)
        .child(
            text("Sniplet")
                .font(rune::TextStyle::new(30.0, 600.0))
                .color(k.c.text_base),
        )
        .child(
            text("Capture. Annotate. Copy.")
                .font(k.ty.body)
                .color(k.c.text_muted),
        )
        .child(
            Button::new("Capture an area")
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::ExtraLarge)
                .icon(&icons::SCAN)
                .on_press_tx(|s: &mut Editor, tx| s.command(super::Command::Area, tx))
                .build(k)
                .key("welcome-capture"),
        )
        .child(
            Button::new("Open an image…")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Large)
                .on_press_tx(|s: &mut Editor, tx| s.command(super::Command::Open, tx))
                .build(k)
                .key("welcome-open"),
        );
    let keys = crate::quick_menu::keycaps(&s.settings.hotkeys.capture_area);
    if !keys.is_empty() {
        welcome = welcome.child(
            row()
                .gap(6.0)
                .center_y()
                .child(Keybind::new(&keys).build(k))
                .child(
                    text("available anywhere")
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_muted),
                ),
        );
    }
    welcome
}
