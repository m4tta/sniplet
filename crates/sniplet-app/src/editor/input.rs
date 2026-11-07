//! Pointer, wheel and key input on the editor canvas.
use rune::{Drag, Key, KeyEvent, Modifier, Phase, PointerButton, Tx, WheelEvent};
use sniplet_core::{
    Annotation, AnnotationId, AnnotationKind, AnnotationStyle, ImageRect, MagnifierPart,
    MeasurementAxis, Point as ImagePoint, measure_at,
};

use super::{
    Command, Drag as EditDrag, EditHandle, Editor, INLINE_PIXELS, MeasureSession, Panel, handle_at,
    render_options, resize_from_handle,
};
use crate::tools::{Size, Tool};

/// A key as the editor's shortcuts name it: lowercase names such as `"escape"`, `"left"`,
/// `"space"`, `"f1"`, `"shift"` (the key itself) or the unshifted character (`"1"`, `"z"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Keystroke {
    pub key: String,
    pub shift: bool,
    pub alt: bool,
    /// Ctrl, or Cmd on macOS.
    pub command: bool,
    /// The OS auto-repeat of a held key.
    pub held: bool,
}

impl Keystroke {
    /// `base` is the key's character with no modifiers (`Tx::key_base`), so Shift+1 still
    /// names "1".
    pub(crate) fn new(event: &KeyEvent, base: Option<char>, held: bool) -> Option<Self> {
        let key = match &event.key {
            Key::Text(text) if text == " " => "space".to_owned(),
            Key::Text(text) => {
                base.map_or_else(|| text.to_lowercase(), |c| c.to_lowercase().to_string())
            }
            Key::Command(' ') => "space".to_owned(),
            Key::Command(c) => base.unwrap_or(*c).to_lowercase().to_string(),
            Key::Enter => "enter".into(),
            Key::Backspace => "backspace".into(),
            Key::Delete => "delete".into(),
            Key::Left => "left".into(),
            Key::Right => "right".into(),
            Key::Up => "up".into(),
            Key::Down => "down".into(),
            Key::Escape => "escape".into(),
            Key::Tab => "tab".into(),
            Key::Function(n) => format!("f{n}"),
            Key::Modifier {
                modifier: Modifier::ShiftLeft | Modifier::ShiftRight,
                ..
            } => "shift".into(),
            _ => return None,
        };
        let mods = event.mods;
        Some(Self {
            key,
            shift: mods.shift,
            alt: mods.alt,
            command: mods.cmd || mods.ctrl,
            held,
        })
    }
}

/// A press on the canvas, in canvas points.
#[derive(Clone, Copy)]
struct Press {
    x: f32,
    y: f32,
    pan_button: bool,
    shift: bool,
    alt: bool,
    command: bool,
    clicks: u32,
}

impl Editor {
    /// Every press, move and release on the canvas.
    pub(super) fn canvas_drag(&mut self, drag: &mut Drag, tx: &mut Tx) {
        let mods = tx.mods();
        match drag.phase {
            Phase::Down => self.pointer_down(
                Press {
                    x: drag.x,
                    y: drag.y,
                    pan_button: tx.button() == PointerButton::Middle,
                    shift: mods.shift,
                    alt: mods.alt,
                    command: mods.cmd || mods.ctrl,
                    clicks: drag.clicks,
                },
                tx,
            ),
            Phase::Move => self.pointer_move(drag.x, drag.y, mods.shift),
            Phase::Up if drag.cancelled() => self.cancel_drag(),
            Phase::Up => self.pointer_up(),
        }
    }

    /// The pointer moved over the canvas with no button held.
    pub(super) fn canvas_hover(&mut self, x: f32, y: f32, shift: bool) {
        if self.drag.is_none() {
            self.pointer_move(x, y, shift);
        }
    }

    fn pointer_down(&mut self, event: Press, tx: &mut Tx) {
        tx.blur();
        self.panel = None;
        let p = self.image_point(event.x, event.y);
        if self.measure.is_some() {
            if !event.pan_button {
                self.update_measure(p, event.shift);
                if let Some(measurement) = self.measure.as_ref().and_then(|m| m.measurement)
                    && let Some(doc) = &mut self.document
                {
                    doc.add_annotation(
                        AnnotationKind::Measurement { measurement },
                        AnnotationStyle::default(),
                    );
                    self.status = format!("Measured {}", measurement.label());
                    self.refresh();
                }
            }
            return;
        }
        let screen = ImagePoint::new(event.x, event.y);
        let zoom = self.transform().zoom();
        let Some(doc) = &mut self.document else {
            return;
        };
        let pan = self.space || event.pan_button;
        let selected = if self.selection.is_some() {
            None
        } else {
            doc.selected()
        };
        let bounds = self.selection.or_else(|| {
            selected
                .and_then(|id| doc.annotation(id))
                .map(|a| a.kind.bounds())
        });
        let arrow_points = selected
            .and_then(|id| doc.annotation(id))
            .and_then(|annotation| annotation.kind.arrow_points());
        let magnifier_circles = selected
            .and_then(|id| doc.annotation(id))
            .and_then(|annotation| annotation.kind.magnifier_circles());
        if !pan
            && !event.alt
            && matches!(self.tool, Tool::Select | Tool::Zoom)
            && let Some(circles) = magnifier_circles
        {
            let tolerance = 7.0 / zoom;
            let edit = circles
                .into_iter()
                .enumerate()
                .rev()
                .find_map(|(index, circle)| {
                    let center = ImagePoint::new(
                        circle.x + circle.width * 0.5,
                        circle.y + circle.height * 0.5,
                    );
                    let edge = ImagePoint::new(center.x + circle.width * 0.5, center.y);
                    let resize =
                        (p.x - edge.x).abs() <= tolerance && (p.y - edge.y).abs() <= tolerance;
                    resize.then_some(EditHandle::Magnifier(
                        if index == 0 {
                            MagnifierPart::Source
                        } else {
                            MagnifierPart::Lens
                        },
                        true,
                    ))
                })
                .or_else(|| {
                    selected
                        .and_then(|id| doc.annotation(id))
                        .and_then(|annotation| annotation.kind.magnifier_part_at(p, tolerance))
                        .map(|part| EditHandle::Magnifier(part, false))
                });
            if let Some(edit) = edit {
                let _ = doc.begin_group();
                self.drag = Some(EditDrag::at(p, selected, Some(edit), false));
                return;
            }
        }
        if !pan
            && matches!(self.tool, Tool::Select | Tool::Arrow)
            && let Some(points) = arrow_points
            && let Some(handle) = points.iter().position(|handle| {
                (p.x - handle.x).abs() <= 7.0 / zoom && (p.y - handle.y).abs() <= 7.0 / zoom
            })
        {
            let _ = doc.begin_group();
            self.drag = Some(EditDrag::at(
                p,
                selected,
                Some(EditHandle::Arrow(handle)),
                event.shift,
            ));
            return;
        }
        if !pan
            && self.tool == Tool::Select
            && arrow_points.is_none()
            && magnifier_circles.is_none()
            && let Some(rect) = bounds
            && let Some(handle) = handle_at(rect, p, 7.0 / zoom)
        {
            if selected.is_some() {
                let _ = doc.begin_group();
            }
            self.drag = Some(EditDrag::at(
                p,
                selected,
                Some(EditHandle::Bounds(handle, rect)),
                event.shift,
            ));
            return;
        }
        if !pan
            && !doc
                .crop()
                .unwrap_or(ImageRect::new(
                    0.0,
                    0.0,
                    doc.width() as f32,
                    doc.height() as f32,
                ))
                .contains(p)
        {
            return;
        }
        if !pan && self.quick_zoom {
            self.unfit();
            self.transform
                .zoom_by(if event.alt { 0.5 } else { 2.0 }, screen);
            return;
        }
        if !pan && self.tool == Tool::Text {
            self.text_origin = p;
            self.text_edit = None;
            self.open_text_panel("");
            return;
        }
        let Some(doc) = &mut self.document else {
            return;
        };
        if !pan && self.tool == Tool::Select && event.command {
            let _ = doc.select(None);
            self.selection = doc.select_monotone_at(p, 12);
            return;
        }
        if !pan && self.tool == Tool::Counter {
            let mut kind = AnnotationKind::Counter {
                center: p,
                value: self.counter,
                font_size: Size::Counter.default_value(),
            };
            let mut style = self.style;
            self.sizes.apply(&mut kind, &mut style);
            let id = doc.add_annotation(kind, style);
            let _ = doc.select(Some(id));
            self.counter = (self.counter + 1).min(99);
            self.refresh();
            return;
        }
        let tool = self.tool;
        let mut moving = if !pan && matches!(tool, Tool::Select | Tool::Arrow | Tool::Zoom) {
            let hit = doc.hit_test(p, 6.0 / zoom).filter(|id| {
                tool == Tool::Select
                    || doc.annotation(*id).is_some_and(|annotation| match tool {
                        Tool::Arrow => matches!(annotation.kind, AnnotationKind::Arrow { .. }),
                        Tool::Zoom => {
                            matches!(annotation.kind, AnnotationKind::Magnifier { .. })
                        }
                        _ => false,
                    })
            });
            let _ = doc.select(hit);
            hit
        } else {
            None
        };
        let mut magnifier_edit = None;
        if let Some(annotation) = moving.and_then(|id| doc.annotation(id)).cloned() {
            self.style = super::drawing_style(self.style, &annotation);
            magnifier_edit = annotation
                .kind
                .magnifier_part_at(p, 6.0 / zoom)
                .map(|part| EditHandle::Magnifier(part, false));
            if event.clicks == 2
                && let AnnotationKind::Text { origin, text, .. } = &annotation.kind
            {
                self.text_origin = *origin;
                self.text_edit = Some(annotation.id);
                let text = text.clone();
                self.open_text_panel(&text);
                return;
            }
            let _ = doc.begin_group();
            if event.alt {
                let id = doc.add_annotation(annotation.kind, annotation.style);
                let _ = doc.select(Some(id));
                moving = Some(id);
                magnifier_edit = None;
            }
        }
        self.selection = None;
        self.drag = Some(EditDrag {
            points: vec![p],
            last: if pan { screen } else { p },
            pan,
            ..EditDrag::at(p, moving, magnifier_edit, event.shift)
        });
    }

    fn pointer_move(&mut self, x: f32, y: f32, shift: bool) {
        let p = self.image_point(x, y);
        self.cursor = p;
        if self.measure.is_some() {
            self.update_measure(p, shift);
            return;
        }
        if let Some(color) = self.visible_color(p) {
            self.sampled = color;
        }
        let transform = self.transform();
        let Some(drag) = &mut self.drag else {
            return;
        };
        if drag.pan {
            let current = ImagePoint::new(x, y);
            let pan = transform.pan();
            let (dx, dy) = (current.x - drag.last.x, current.y - drag.last.y);
            drag.last = current;
            self.unfit();
            self.transform
                .set_pan(ImagePoint::new(pan.x + dx, pan.y + dy));
            return;
        }
        drag.end = p;
        drag.constrain = shift;
        if let Some(EditHandle::Magnifier(part, resize)) = drag.handle {
            if let Some(doc) = &mut self.document
                && let Some(annotation) = drag.moving.and_then(|id| doc.annotation(id)).cloned()
            {
                let mut kind = annotation.kind;
                if resize {
                    if let Some(circles) = kind.magnifier_circles() {
                        let circle = circles[if part == MagnifierPart::Source { 0 } else { 1 }];
                        let center = ImagePoint::new(
                            circle.x + circle.width * 0.5,
                            circle.y + circle.height * 0.5,
                        );
                        let radius = (p.x - center.x)
                            .hypot(p.y - center.y)
                            .min(doc.width().max(doc.height()) as f32);
                        kind.resize_magnifier_circle(part, radius);
                    }
                } else {
                    kind.move_magnifier_circle(
                        part,
                        ImagePoint::new(p.x - drag.last.x, p.y - drag.last.y),
                    );
                }
                let _ = doc.update_annotation(annotation.id, kind, annotation.style);
            }
            drag.last = p;
        } else if let Some(EditHandle::Arrow(handle)) = drag.handle {
            if let Some(doc) = &mut self.document
                && let Some(annotation) = drag.moving.and_then(|id| doc.annotation(id)).cloned()
            {
                let mut kind = annotation.kind;
                let mut point = p;
                if shift
                    && handle != 1
                    && let Some(points) = kind.arrow_points()
                {
                    point = crate::tools::constrain_angle(points[2 - handle], p);
                }
                kind.set_arrow_point(handle, point);
                let _ = doc.update_annotation(annotation.id, kind, annotation.style);
            }
        } else if let Some(EditHandle::Bounds(handle, bounds)) = drag.handle {
            let resized = resize_from_handle(bounds, handle, p, shift);
            if let Some(id) = drag.moving {
                if let Some(doc) = &mut self.document {
                    let _ = doc.resize_annotation(id, resized);
                }
            } else {
                self.selection = Some(resized);
                return;
            }
        } else if let Some(id) = drag.moving {
            if let Some(doc) = &mut self.document {
                let _ =
                    doc.move_annotation(id, ImagePoint::new(p.x - drag.last.x, p.y - drag.last.y));
            }
            drag.last = p;
        } else if matches!(self.tool, Tool::Select | Tool::Ocr) {
            let end = if drag.constrain {
                let dx = p.x - drag.start.x;
                let dy = p.y - drag.start.y;
                let side = dx.abs().max(dy.abs());
                ImagePoint::new(
                    drag.start.x + side * dx.signum(),
                    drag.start.y + side * dy.signum(),
                )
            } else {
                p
            };
            self.selection = Some(ImageRect::from_corners(drag.start, end));
            return;
        } else {
            if self.tool == Tool::Freehand {
                drag.points.push(p);
            }
            self.draft = self
                .tool
                .annotation(
                    drag.start,
                    p,
                    &drag.points,
                    "",
                    self.counter,
                    self.hide,
                    drag.constrain,
                )
                .map(|mut kind| {
                    let mut style = self.style;
                    self.sizes.apply(&mut kind, &mut style);
                    if let AnnotationKind::Arrow { variant, .. } = &mut kind {
                        *variant = self.arrow_variant;
                    }
                    if let AnnotationKind::Magnifier {
                        rect,
                        source: Some(_),
                        ..
                    } = &mut kind
                        && let Some(doc) = &self.document
                    {
                        let radius = rect.width * 0.5;
                        let fit = |center: f32, extent: u32| {
                            if radius * 2.0 <= extent as f32 {
                                center.clamp(radius, extent as f32 - radius)
                            } else {
                                extent as f32 * 0.5
                            }
                        };
                        rect.x = fit(rect.x + radius, doc.width()) - radius;
                        rect.y = fit(rect.y + radius, doc.height()) - radius;
                    }
                    Annotation {
                        id: AnnotationId(0),
                        kind,
                        style,
                    }
                });
        }
        self.refresh();
    }

    fn pointer_up(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.pan {
            return;
        }
        if let Some(doc) = &mut self.document {
            if drag.moving.is_some() {
                let _ = doc.end_group();
            } else if matches!(self.tool, Tool::Select | Tool::Ocr) {
                self.selection = self
                    .selection
                    .map(|r| r.clipped(doc.width(), doc.height()))
                    .filter(|r| r.width >= 1.0 && r.height >= 1.0);
            } else if let Some(annotation) = self.draft.take() {
                let id = doc.add_annotation(annotation.kind, annotation.style);
                let _ = doc.select(Some(id));
            }
        }
        self.refresh();
        if self.tool == Tool::Ocr && self.selection.is_some() {
            self.recognize(false);
        }
    }

    /// Escape or a lost pointer ended a drag: put back what it moved.
    fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take()
            && drag.moving.is_some()
            && let Some(doc) = &mut self.document
        {
            let _ = doc.cancel_group();
        }
        self.draft = None;
        self.refresh();
    }

    pub(super) fn wheel(&mut self, event: WheelEvent) -> bool {
        if self.measure.is_some() {
            let step = if event.dy == 0.0 { event.dx } else { event.dy };
            self.measure_tolerance = (i16::from(self.measure_tolerance)
                + (step.signum() * 5.0) as i16)
                .clamp(0, 254) as u8;
            let point = self.image_point(event.x, event.y);
            self.update_measure(point, event.mods.shift);
            return true;
        }
        self.unfit();
        if event.mods.cmd || event.mods.ctrl || self.tool == Tool::Zoom {
            self.transform
                .zoom_by((event.dy * 0.003).exp(), ImagePoint::new(event.x, event.y));
        } else {
            let pan = self.transform.pan();
            self.transform
                .set_pan(ImagePoint::new(pan.x + event.dx, pan.y + event.dy));
        }
        true
    }

    fn open_text_panel(&mut self, text: &str) {
        self.text_input = rune::TextEdit::new(text);
        self.text_input.focused = true;
        self.panel = Some(Panel::Text);
    }

    /// A key the window heard. True when the editor used it.
    pub(super) fn key(&mut self, event: &Keystroke, tx: &mut Tx) -> bool {
        let key = event.key.as_str();
        if self.recognized_text.is_some() {
            if key == "escape" {
                self.recognized_text = None;
                tx.blur();
                return true;
            }
            return false;
        }
        if key == "escape" {
            if let Some(edit) = self.size_edit.take() {
                if edit.grouped
                    && let Some(doc) = &mut self.document
                {
                    let _ = doc.cancel_group();
                }
                self.sizes.set(edit.size, edit.before);
            }
            if let Some(drag) = self.drag.take()
                && drag.moving.is_some()
                && let Some(doc) = &mut self.document
            {
                let _ = doc.cancel_group();
            }
            self.panel = None;
            self.selection = None;
            self.draft = None;
            self.space = false;
            self.quick_zoom = false;
            self.clear_measure();
            tx.blur();
            self.refresh();
            return true;
        }
        if !event.command
            && !event.alt
            && self.drag.is_none()
            && !matches!(self.panel, Some(Panel::Text))
        {
            if key == "shift"
                && let Some((_, _, outer)) = &mut self.measure_pending
            {
                *outer = true;
                return true;
            }
            if key == "shift" && self.measure.is_some() {
                self.update_measure(self.cursor, true);
                return true;
            }
            let axis = match key {
                "1" | "left" | "right" => Some(MeasurementAxis::Horizontal),
                "2" | "up" | "down" => Some(MeasurementAxis::Vertical),
                _ => None,
            };
            if let Some(axis) = axis
                && self.selection.is_none()
                && let Some(doc) = &self.document
                && doc.selected().is_none()
            {
                if !event.held || self.measure.as_ref().is_none_or(|m| m.key != key) {
                    if u64::from(doc.width()) * u64::from(doc.height()) > INLINE_PIXELS {
                        if self
                            .measure_pending
                            .as_ref()
                            .is_some_and(|(pending, _, _)| pending == key)
                        {
                            return true;
                        }
                        let document = doc.clone();
                        let key = key.to_owned();
                        let revision = self.preview_revision;
                        self.measure_pending = Some((key.clone(), revision, event.shift));
                        self.inbox.spawn(
                            move || document.render_measurement_source(&render_options()),
                            move |this, result, _| {
                                if this.preview_revision != revision {
                                    return;
                                }
                                let Some((pending, pending_revision, outer)) =
                                    &this.measure_pending
                                else {
                                    return;
                                };
                                if *pending != key || *pending_revision != revision {
                                    return;
                                }
                                let outer = *outer;
                                this.measure_pending = None;
                                match result {
                                    Ok((source, origin)) => {
                                        this.measure = Some(MeasureSession {
                                            key,
                                            axis,
                                            source,
                                            origin,
                                            outer,
                                            measurement: None,
                                        });
                                        this.update_measure(this.cursor, outer);
                                    }
                                    Err(error) => this.status = error.to_string(),
                                }
                            },
                        );
                        self.panel = None;
                        return true;
                    }
                    match doc.render_measurement_source(&render_options()) {
                        Ok((source, origin)) => {
                            self.measure = Some(MeasureSession {
                                key: key.to_owned(),
                                axis,
                                source,
                                origin,
                                outer: event.shift,
                                measurement: None,
                            })
                        }
                        Err(error) => {
                            self.status = error.to_string();
                            return true;
                        }
                    }
                }
                self.panel = None;
                self.update_measure(self.cursor, event.shift);
                return true;
            }
        }
        if matches!(self.panel, Some(Panel::Text)) {
            if key == "enter" {
                self.commit_text(tx);
                return true;
            }
            return false;
        }
        let step = if event.shift { 10.0 } else { 1.0 };
        let action = if event.command {
            match key {
                "c" => Some(Command::Copy),
                "q" => Some(Command::Quit),
                "s" => Some(if event.shift {
                    Command::SaveProject
                } else {
                    Command::Save
                }),
                "o" => Some(if self.selection.is_some() {
                    Command::Ocr
                } else {
                    Command::Open
                }),
                "v" => Some(Command::Paste),
                "z" => Some(if event.shift {
                    Command::Redo
                } else {
                    Command::Undo
                }),
                "y" => Some(Command::Redo),
                "1" => Some(Command::Fit),
                "0" => Some(Command::ActualSize),
                "+" | "=" => Some(Command::ZoomIn),
                "-" => Some(Command::ZoomOut),
                "," => Some(Command::Settings),
                "a" => Some(Command::AddCapture),
                "2" => Some(Command::ZoomSelection),
                "left" | "right" | "up" | "down" => {
                    self.resize_selected(key, step, false);
                    None
                }
                _ => return false,
            }
        } else {
            match key {
                "enter" => Some(Command::Crop),
                "backspace" | "delete" => Some(Command::Delete),
                "tab" => {
                    if event.shift
                        && let Some(color) = self
                            .document
                            .as_ref()
                            .and_then(|doc| doc.darkest_color_around(self.cursor, 10))
                    {
                        self.sampled = color;
                    }
                    self.copy_color();
                    self.color_peek = true;
                    None
                }
                "space" => {
                    self.space = true;
                    None
                }
                "z" => {
                    self.quick_zoom = true;
                    None
                }
                "c" if self.selection.is_some() => {
                    if let (Some(doc), Some(rect)) = (&self.document, self.selection)
                        && let Some(color) = doc.average_color(rect)
                    {
                        self.sampled = color;
                        self.copy_color();
                        self.color_peek = true;
                    }
                    None
                }
                "f1" => {
                    self.panel = Some(Panel::Help);
                    None
                }
                "left" | "right" | "up" | "down" => {
                    self.nudge(key, step);
                    None
                }
                "[" | "]" => {
                    self.resize_selected(key, step, true);
                    None
                }
                "q" | "w" => {
                    if let Some(rect) = self.selection {
                        let p = if key == "q" {
                            ImagePoint::new(rect.x, rect.y)
                        } else {
                            ImagePoint::new(rect.x + rect.width, rect.y + rect.height)
                        };
                        let anchor = self.canvas_point(p);
                        self.unfit();
                        self.transform.set_zoom_around(4.0, anchor);
                    }
                    None
                }
                "b" => {
                    self.cycle_hide();
                    None
                }
                _ => match Tool::from_key(key) {
                    Some(tool) => {
                        self.select_tool(tool);
                        None
                    }
                    None => return false,
                },
            }
        };
        if let Some(action) = action {
            self.command(action, tx);
        }
        true
    }

    pub(super) fn key_up(&mut self, event: &Keystroke) -> bool {
        let key = event.key.as_str();
        if self
            .measure_pending
            .as_ref()
            .is_some_and(|(pending, _, _)| pending == key)
        {
            self.measure_pending = None;
        } else if key == "shift"
            && let Some((_, _, outer)) = &mut self.measure_pending
        {
            *outer = false;
        }
        if self
            .measure
            .as_ref()
            .is_some_and(|session| session.key == key)
        {
            self.clear_measure();
        } else if key == "shift" {
            self.update_measure(self.cursor, false);
        }
        if key == "space" {
            self.space = false;
        }
        if key == "z" {
            self.quick_zoom = false;
        }
        if matches!(key, "tab" | "c") {
            self.color_peek = false;
        }
        true
    }

    /// The window lost focus: drop held-key modes so none sticks.
    pub(crate) fn deactivated(&mut self) {
        self.clear_measure();
        self.space = false;
        self.quick_zoom = false;
        self.color_peek = false;
    }

    fn update_measure(&mut self, point: ImagePoint, outer: bool) {
        let (physical, scale, tolerance) = (
            self.physical_units,
            self.measure_scale,
            self.measure_tolerance,
        );
        let Some(session) = &mut self.measure else {
            return;
        };
        session.outer = outer;
        let measurement = measure_at(
            &session.source,
            ImagePoint::new(point.x - session.origin.x, point.y - session.origin.y),
            session.axis,
            tolerance,
            outer,
        )
        .map(|mut measurement| {
            measurement.start.x += session.origin.x;
            measurement.start.y += session.origin.y;
            measurement.end.x += session.origin.x;
            measurement.end.y += session.origin.y;
            measurement.pixels_per_unit = if physical { 1.0 } else { scale };
            measurement.scale_factor = scale;
            measurement
        });
        if measurement == session.measurement {
            return;
        }
        session.measurement = measurement;
        self.measure_overlay = None;
        if let Some(measurement) = measurement
            && let Ok((bounds, image)) = measurement.overlay(super::FONT)
        {
            let (width, height) = image.dimensions();
            self.measure_pixels.set(width, height, image.into_raw());
            self.measure_overlay = Some(bounds);
        }
    }

    pub(super) fn toggle_units(&mut self) {
        self.physical_units = !self.physical_units;
        let outer = self.measure.as_ref().is_some_and(|m| m.outer);
        self.update_measure(self.cursor, outer);
    }

    fn nudge(&mut self, key: &str, step: f32) {
        let delta = match key {
            "left" => ImagePoint::new(-step, 0.0),
            "right" => ImagePoint::new(step, 0.0),
            "up" => ImagePoint::new(0.0, -step),
            _ => ImagePoint::new(0.0, step),
        };
        if let Some(doc) = &mut self.document {
            if let Some(id) = doc.selected() {
                let _ = doc.move_annotation(id, delta);
                self.refresh();
            } else if let Some(r) = self.selection {
                self.selection = Some(ImageRect::new(
                    (r.x + delta.x).clamp(0.0, (doc.width() as f32 - r.width).max(0.0)),
                    (r.y + delta.y).clamp(0.0, (doc.height() as f32 - r.height).max(0.0)),
                    r.width,
                    r.height,
                ));
            }
        }
    }

    fn resize_selected(&mut self, key: &str, step: f32, symmetric: bool) {
        let Some(doc) = &mut self.document else {
            return;
        };
        let selected = if self.selection.is_some() {
            None
        } else {
            doc.selected()
        };
        let Some(mut rect) = self.selection.or_else(|| {
            selected
                .and_then(|id| doc.annotation(id))
                .map(|a| a.kind.bounds())
        }) else {
            return;
        };
        if symmetric {
            rect = rect.expanded(if key == "]" { step } else { -step });
        } else {
            match key {
                "left" => rect.width -= step,
                "right" => rect.width += step,
                "up" => rect.height -= step,
                _ => rect.height += step,
            }
        }
        if rect.width < 1.0 || rect.height < 1.0 {
            return;
        }
        if let Some(id) = selected {
            let _ = doc.resize_annotation(id, rect);
            self.refresh();
        } else {
            self.selection = Some(rect.clipped(doc.width(), doc.height()));
        }
    }

    pub(super) fn commit_text(&mut self, tx: &mut Tx) {
        let text = self.text_input.text.clone();
        if !text.trim().is_empty()
            && let Some(doc) = &mut self.document
        {
            let font_size = self
                .text_edit
                .and_then(|id| doc.annotation(id))
                .and_then(|a| match a.kind {
                    AnnotationKind::Text { font_size, .. } => Some(font_size),
                    _ => None,
                })
                .unwrap_or(self.sizes.get(Size::Text));
            let kind = AnnotationKind::Text {
                origin: self.text_origin,
                text,
                font_size,
            };
            if let Some(id) = self.text_edit.take() {
                let _ = doc.update_annotation(id, kind, self.style);
            } else {
                let id = doc.add_annotation(kind, self.style);
                let _ = doc.select(Some(id));
            }
        }
        self.panel = None;
        tx.blur();
        self.refresh();
    }
}

impl EditDrag {
    fn at(
        p: ImagePoint,
        moving: Option<AnnotationId>,
        handle: Option<EditHandle>,
        constrain: bool,
    ) -> Self {
        Self {
            start: p,
            end: p,
            points: vec![],
            moving,
            last: p,
            pan: false,
            constrain,
            handle,
        }
    }
}
