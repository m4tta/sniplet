//! The editor window: the toolbar in the title bar, and the canvas with its dock.
use rune::{Color, Cursor, Cx, El, KeyEvent, PointerButton, Tx, col, el, icon, row, text};
use sniplet_core::{ColorFormat, Point as ImagePoint};

use super::{BAR, DOCK_BOTTOM, DOCK_HEIGHT, Editor, Keystroke, Panel};
use crate::{
    tools::Tool,
    ui::{BODY, Button, Palette, SMALL, core_color, divider, icons, rgb},
};

/// The editor window's view.
pub(crate) fn view(s: &Editor, cx: &mut Cx<Editor>) -> El {
    let p = Palette::for_preference(s.settings.theme, cx.system_dark());
    cx.caption_color(p.foreground);
    if s.updates.installing() {
        return crate::updates::installing_screen(&p);
    }
    let insets = cx.chrome_insets();
    let mut root = col()
        .key("editor")
        .w_full()
        .h_full()
        .fill(p.background)
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
        .child(toolbar(s, &p, insets.leading, insets.trailing))
        .child(canvas(s, &p));
    if s.panel == Some(Panel::Menu) {
        root = root.child(super::panels::app_menu(s, &p, insets.leading));
    }
    root.children(super::panels::recognized_text(s, &p))
}

/// Keys nothing focused took, or every key while nothing has focus.
pub(crate) fn on_key(s: &mut Editor, event: &KeyEvent, tx: &mut Tx) -> bool {
    Keystroke::new(event, tx.key_base(), tx.key_repeat()).is_some_and(|key| s.key(&key, tx))
}

fn toolbar(s: &Editor, p: &Palette, leading: f32, trailing: f32) -> El {
    let zoom = s.transform().zoom();
    let mut menu = Button::new("app-menu")
        .icon(&icons::SCISSORS)
        .help("Sniplet · capture and settings")
        .build(p, |s: &mut Editor, _| {
            s.panel = if s.panel == Some(Panel::Menu) {
                None
            } else {
                Some(Panel::Menu)
            };
        });
    if s.waiting_update_label().is_some() {
        menu = el().child(menu).child(
            el().key("update-dot")
                .size(7.0, 7.0)
                .radius(3.5)
                .fill(p.accent)
                .pin(None, Some(5.0), Some(5.0), None),
        );
    }
    row()
        .key("editor-toolbar")
        .w_full()
        .h(BAR)
        .pl(leading + 8.0)
        .pr(trailing + 8.0)
        .gap(2.0)
        .center_y()
        .fill(p.title_bar)
        .child(menu)
        .child(el().grow(1.0).h_full().window_drag())
        .child(
            Button::new("zoom-fit")
                .label(format!("{:.0}%", zoom * 100.0))
                .help(format!("{} · Fit · Ctrl/Cmd 1", s.image_size_label()))
                .build(p, |s: &mut Editor, tx| s.command(super::Command::Fit, tx)),
        )
        .child(divider(p, 24.0).mx(8.0))
        .child(
            Button::new("backdrop")
                .icon(&icons::LAYERS)
                .label("Backdrop")
                .build(p, |s: &mut Editor, _| s.panel = Some(Panel::Backdrop)),
        )
        .child(drag_file(s, p))
        .child(
            Button::new("pin")
                .icon(&icons::PIN)
                .help("Pin image on screen")
                .size(34.0, 36.0)
                .build(p, |s: &mut Editor, tx| s.command(super::Command::Pin, tx)),
        )
        .child(
            Button::new("save")
                .icon(&icons::SAVE)
                .help("Save image · Ctrl/Cmd S")
                .size(34.0, 36.0)
                .build(p, |s: &mut Editor, tx| s.command(super::Command::Save, tx)),
        )
        .child(
            Button::new("copy")
                .accent()
                .icon(&icons::COPY)
                .label("Copy")
                .help("Copy image · Ctrl/Cmd C")
                .build(p, |s: &mut Editor, tx| s.command(super::Command::Copy, tx))
                .ml(4.0),
        )
}

/// The pill that drags the exported image into another app as a PNG file.
fn drag_file(s: &Editor, p: &Palette) -> El {
    let grip = || {
        row().gap(2.5).children((0..2).map(|_| {
            col()
                .gap(2.5)
                .children((0..3).map(|_| el().size(1.5, 1.5).radius(0.75).fill(p.muted_foreground)))
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
        .fill(p.foreground.alpha(0.08))
        .child(grip())
        .child(icon(&icons::FILE, 17.0).color(p.foreground))
        .child(grip());
    if s.document.is_none() {
        return control.opacity(0.4);
    }
    crate::drag_out::source(
        control.hover(p.foreground.alpha(0.12)).cursor(Cursor::Grab),
        |s: &Editor| match s.export_drag_file() {
            Ok(path) => Ok(path),
            Err(error) => Err(format!("Could not drag image: {error}")),
        },
        |s: &mut Editor, error: String| s.status = error,
    )
}

fn canvas(s: &Editor, p: &Palette) -> El {
    let (width, height) = s.canvas;
    let mut canvas = el()
        .key("editor-canvas")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(p.muted)
        .clip_rounded(0.0)
        .pointer_buttons([PointerButton::Middle])
        .on_layout(|s: &mut Editor, rect: [f32; 4]| s.canvas = (rect[2], rect[3]))
        .on_drag(|s: &mut Editor, drag: &mut rune::Drag, tx: &mut Tx| s.canvas_drag(drag, tx))
        .on_pointer_move_local(|s: &mut Editor, event: rune::PointerEvent, tx: &mut Tx| {
            s.canvas_hover(event.x, event.y, tx.mods().shift)
        })
        .on_wheel(|s: &mut Editor, event: rune::WheelEvent, _: &mut Tx| s.wheel(event));
    if s.document.is_some() && s.image_size.0 > 0 {
        canvas = image_layers(s, p, canvas, height);
        if let Some(properties) = super::panels::properties(s, p) {
            canvas = canvas.child(
                row()
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
            .child(dock(s, p))
            .children(color_peek(s, p, width, height));
    } else if s.document.is_none() {
        canvas = canvas.child(welcome(p));
    }
    canvas = canvas.child(
        el().key("status")
            .absolute(12.0, 12.0)
            .max_w((width - 24.0).max(0.0))
            .px(12.0)
            .py(4.0)
            .radius(6.0)
            .fill(p.popover.alpha(0.95))
            .child(text(s.status.clone()).font(SMALL).color(p.muted_foreground)),
    );
    canvas
        .children(s.render_update_notice(p))
        .children(super::panels::floating_panel(s, p, height))
}

/// The image, the measurement overlay, and the selection or edit handles.
fn image_layers(s: &Editor, p: &Palette, mut canvas: El, height: f32) -> El {
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
            .border(1.0, p.accent);
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
            el().key("selection-size")
                .absolute(origin.x, (origin.y + h + 10.0).min(height - 30.0))
                .px(8.0)
                .py(4.0)
                .radius(6.0)
                .fill(p.popover.alpha(0.95))
                .child(
                    text(format!("{:.0} × {:.0} px{hint}", rect.width, rect.height))
                        .font(SMALL)
                        .color(p.foreground)
                        .single_line(),
                ),
        );
    }
    canvas
}

/// Every tool, in a dock at the bottom center of the canvas.
fn dock(s: &Editor, p: &Palette) -> El {
    let tools = Tool::ALL.map(|tool| {
        Button::new(format!("tool-{}", tool as usize))
            .icon(tool.icon())
            .size(36.0, 36.0)
            .selected(s.tool == tool)
            .help(tool.shortcut().map_or_else(
                || tool.label().to_owned(),
                |key| format!("{} · {key}", tool.label()),
            ))
            .build(p, move |s: &mut Editor, _| s.select_tool(tool))
    });
    row()
        .pin(Some(0.0), None, Some(0.0), Some(DOCK_BOTTOM))
        .justify_center()
        .child(
            row()
                .key("tool-dock")
                .gap(2.0)
                .padding(5.0)
                .fill(p.popover)
                .border(1.0, p.border)
                .radius(14.0)
                .shadow(8.0, 24.0, Color::BLACK.alpha(0.18))
                .on_press(|_: &mut Editor| {})
                .children(tools),
        )
}

/// The sampled color next to the pointer, shown while Tab or C is held.
fn color_peek(s: &Editor, p: &Palette, width: f32, height: f32) -> Option<El> {
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
        row()
            .key("color-peek")
            .absolute(x, y)
            .gap(8.0)
            .center_y()
            .px(8.0)
            .py(4.0)
            .radius(6.0)
            .fill(p.popover)
            .border(1.0, p.border)
            .child(
                el().size(14.0, 14.0)
                    .radius(7.0)
                    .fill(core_color(s.sampled))
                    .border(1.0, p.border),
            )
            .child(
                text(s.sampled.format(ColorFormat::HexRgb))
                    .font(SMALL)
                    .color(p.foreground),
            ),
    )
}

fn welcome(p: &Palette) -> El {
    col()
        .key("welcome")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .gap(12.0)
        .child(
            text("Sniplet")
                .font(rune::TextStyle::new(30.0, 600.0))
                .color(p.foreground),
        )
        .child(
            text("Capture. Annotate. Copy.")
                .font(BODY)
                .color(p.muted_foreground),
        )
        .child(
            Button::new("welcome-capture")
                .primary()
                .icon(&icons::SCAN)
                .label("Capture an area")
                .build(p, |s: &mut Editor, tx| s.command(super::Command::Area, tx)),
        )
        .child(
            Button::new("welcome-open")
                .label("Open an image…")
                .build(p, |s: &mut Editor, tx| s.command(super::Command::Open, tx)),
        )
        .child(
            text("Ctrl/Cmd + Shift + 2 · available anywhere")
                .font(SMALL)
                .color(p.muted_foreground),
        )
}
