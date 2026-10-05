use std::{path::PathBuf, sync::Arc};

use gpui_kit::{
    base::slider::{SliderEvent, SliderState},
    component::{
        ActiveTheme, Icon, Selectable, TitleBar,
        button::{Button, ButtonVariants},
        input::{Input, InputState},
    },
    prelude::*,
    *,
};
use sniplet_core::{
    Annotation, AnnotationId, AnnotationKind, AnnotationStyle, ArrowVariant, Backdrop, Background,
    Color, ColorFormat, Document, ImageRect, MagnifierPart, Measurement, MeasurementAxis,
    Point as ImagePoint, RenderOptions, Shadow, ViewportTransform, measure_at,
};
use sniplet_platform::{Clipboard, Settings, SettingsStore, ThemePreference};

use crate::{capture, tools::Tool};

pub const FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSans.ttf");
pub(crate) const BAR: f32 = 54.0;
const ARROW_WIDTH: f32 = 10.0;

pub fn render_options() -> RenderOptions<'static> {
    RenderOptions {
        font_bytes: Some(FONT),
        ..Default::default()
    }
}

pub fn display_image(mut image: image::RgbaImage) -> Arc<RenderImage> {
    let started = std::time::Instant::now();
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
        eprintln!(
            "capture: display image {:?} prepared in {:?}",
            image.dimensions(),
            started.elapsed()
        );
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(image)]))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Menu,
    Tools,
    Text,
    Backdrop,
    Settings,
    Cloud,
    Hotkeys,
    Help,
    Ruler,
}

#[derive(Clone, Copy)]
pub enum Command {
    Open,
    Paste,
    LoadClipboard,
    Copy,
    Save,
    SaveProject,
    Undo,
    Redo,
    Crop,
    AutoAdjust,
    ResetCrop,
    Delete,
    Pin,
    Area,
    AddCapture,
    Screen,
    Window,
    Delayed,
    Scroll,
    ScrollUp,
    ManualScroll,
    Repeat,
    ActiveWindow,
    CaptureOcr,
    ShowEditor,
    Settings,
    GitHub,
    Quit,
    Ocr,
    Qr,
    #[expect(dead_code, reason = "The upload button is hidden for now.")]
    Upload,
    Fit,
    ActualSize,
    ZoomSelection,
    ZoomIn,
    ZoomOut,
}

#[derive(Clone, Copy)]
enum EditHandle {
    Bounds(usize, ImageRect),
    Arrow(usize),
    Magnifier(MagnifierPart, bool),
}

struct Drag {
    start: ImagePoint,
    end: ImagePoint,
    points: Vec<ImagePoint>,
    moving: Option<AnnotationId>,
    last: ImagePoint,
    pan: bool,
    constrain: bool,
    handle: Option<EditHandle>,
}

struct ArrowSizeEdit {
    style: AnnotationStyle,
    width: f32,
    grouped: bool,
}

struct MeasureSession {
    key: String,
    axis: MeasurementAxis,
    source: image::RgbaImage,
    origin: ImagePoint,
    outer: bool,
    measurement: Option<Measurement>,
}

pub struct Editor {
    pub document: Option<Document>,
    pub tool: Tool,
    pub selection: Option<ImageRect>,
    pub status: String,
    pub settings: Settings,
    pub monitor_index: usize,
    pub panel: Option<Panel>,
    _theme_subscription: Subscription,
    #[cfg(test)]
    pub settings_path: Option<PathBuf>,
    focus: FocusHandle,
    input: Entity<InputState>,
    cloud_url: Entity<InputState>,
    cloud_public_url: Entity<InputState>,
    hotkey_inputs: [Entity<InputState>; 8],
    text_origin: ImagePoint,
    text_edit: Option<AnnotationId>,
    style: AnnotationStyle,
    arrow_width: f32,
    arrow_variant: ArrowVariant,
    arrow_size: Entity<SliderState>,
    arrow_size_edit: Option<ArrowSizeEdit>,
    drawing_width: f32,
    counter: u32,
    drag: Option<Drag>,
    draft: Option<Annotation>,
    image: Option<Arc<RenderImage>>,
    preview_dirty: bool,
    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) preview_frames: usize,
    image_size: (u32, u32),
    content_origin: ImagePoint,
    transform: ViewportTransform,
    fit: bool,
    cursor: ImagePoint,
    sampled: Color,
    space: bool,
    quick_zoom: bool,
    measure: Option<MeasureSession>,
    measure_painted: Option<Measurement>,
    measure_overlay: Option<(ImageRect, Arc<RenderImage>)>,
    measure_tolerance: u8,
    measure_scale: f32,
    default_measure_scale: f32,
    physical_units: bool,
    last_saved: Option<PathBuf>,
    pub scroll_frames: Vec<image::RgbaImage>,
    pub scroll_region: Option<ImageRect>,
    pub last_capture: Option<crate::area_capture::Region>,
}

impl Editor {
    pub fn new(
        document: Option<Document>,
        settings: Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::theme::apply(settings.theme, window, cx);
        let theme_subscription =
            crate::theme::observe(window, cx, |editor: &Editor| editor.settings.theme);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type your annotation…"));
        let arrow_size = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(30.0)
                .step(0.25)
                .default_value(ARROW_WIDTH)
        });
        cx.subscribe(&arrow_size, |editor, _, event, cx| {
            editor.arrow_size_event(event, cx);
        })
        .detach();
        cx.observe_window_activation(window, |editor, window, cx| {
            if !window.is_window_active() {
                editor.measure = None;
                editor.space = false;
                editor.quick_zoom = false;
                cx.notify();
            }
        })
        .detach();
        let (upload_url, public_url) = match &settings.cloud_upload {
            Some(sniplet_platform::CloudUploadConfig::PresignedPut { url, public_url }) => {
                (url.clone(), public_url.clone().unwrap_or_default())
            }
            _ => (String::new(), String::new()),
        };
        let cloud_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://… signed PUT URL")
                .default_value(upload_url)
        });
        let cloud_public_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://… public image URL (optional)")
                .default_value(public_url)
        });
        let hotkey_inputs = [
            &settings.hotkeys.capture_area,
            &settings.hotkeys.capture_screen,
            &settings.hotkeys.capture_window,
            &settings.hotkeys.scrolling_capture,
            &settings.hotkeys.repeat_area,
            &settings.hotkeys.active_window,
            &settings.hotkeys.capture_ocr,
            &settings.hotkeys.show_editor,
        ]
        .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value.clone())));
        let c = settings.annotation_color;
        let measure_scale = document
            .as_ref()
            .and_then(|doc| doc.source_scale_factor())
            .unwrap_or(window.scale_factor());
        let mut this = Self {
            document,
            tool: Tool::Select,
            selection: None,
            status: "Ready".into(),
            settings,
            monitor_index: if cfg!(test) {
                0
            } else {
                sniplet_platform::list_monitors()
                    .ok()
                    .and_then(|monitors| {
                        monitors.into_iter().find(|m| m.is_primary).map(|m| m.index)
                    })
                    .unwrap_or(0)
            },
            panel: None,
            _theme_subscription: theme_subscription,
            #[cfg(test)]
            settings_path: None,
            focus,
            input,
            cloud_url,
            cloud_public_url,
            hotkey_inputs,
            text_origin: ImagePoint::default(),
            text_edit: None,
            style: AnnotationStyle {
                stroke: Color::new(c.red, c.green, c.blue, c.alpha),
                ..Default::default()
            },
            arrow_width: ARROW_WIDTH,
            arrow_variant: ArrowVariant::Solid,
            arrow_size,
            arrow_size_edit: None,
            drawing_width: AnnotationStyle::default().stroke_width,
            counter: 1,
            drag: None,
            draft: None,
            image: None,
            preview_dirty: false,
            #[cfg(all(test, feature = "ui-tests"))]
            preview_frames: 0,
            image_size: (0, 0),
            content_origin: ImagePoint::default(),
            transform: ViewportTransform::default(),
            fit: true,
            cursor: ImagePoint::default(),
            sampled: Color::WHITE,
            space: false,
            quick_zoom: false,
            measure: None,
            measure_painted: None,
            measure_overlay: None,
            measure_tolerance: 20,
            measure_scale,
            default_measure_scale: window.scale_factor(),
            physical_units: false,
            last_saved: None,
            scroll_frames: vec![],
            scroll_region: None,
            last_capture: None,
        };
        this.refresh(cx);
        this
    }

    pub fn load(&mut self, document: Document, label: &str, cx: &mut Context<Self>) {
        self.finish_arrow_size_edit();
        self.measure_scale = document
            .source_scale_factor()
            .unwrap_or(self.default_measure_scale);
        self.document = Some(document);
        self.selection = None;
        self.drag = None;
        self.draft = None;
        self.measure = None;
        self.fit = true;
        self.panel = None;
        self.counter = 1;
        self.last_saved = None;
        self.status = label.to_owned();
        self.refresh(cx);
    }

    pub fn set_measure_scale(&mut self, scale: f32) {
        if let Some(doc) = &mut self.document {
            doc.set_source_scale_factor(scale);
            self.measure_scale = doc
                .source_scale_factor()
                .unwrap_or(self.default_measure_scale);
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = &self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id))
        {
            self.style = annotation.style;
        }
        // Pointer events can arrive faster than the display refreshes. Rasterize
        // the latest document once when GPUI draws, rather than blocking input.
        self.preview_dirty = true;
        cx.notify();
    }

    pub fn select_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.finish_arrow_size_edit();
        self.measure = None;
        if tool == Tool::Ruler {
            self.panel = Some(Panel::Ruler);
            cx.notify();
            return;
        }
        self.tool = tool;
        self.style.stroke_width = if tool == Tool::Arrow {
            self.arrow_width
        } else {
            self.drawing_width
        };
        self.selection = None;
        self.panel = None;
        self.status = tool.shortcut().map_or_else(
            || tool.label().to_owned(),
            |key| format!("{} · {key}", tool.label()),
        );
        if let Some(doc) = &mut self.document {
            let _ = doc.select(None);
        }
        cx.notify();
    }

    fn image_point(&self, point: Point<Pixels>) -> ImagePoint {
        let p = self
            .transform
            .screen_to_image(ImagePoint::new(point.x.into(), f32::from(point.y) - BAR));
        ImagePoint::new(p.x - self.content_origin.x, p.y - self.content_origin.y)
    }

    fn canvas_point(&self, p: ImagePoint) -> ImagePoint {
        self.transform.image_to_screen(ImagePoint::new(
            p.x + self.content_origin.x,
            p.y + self.content_origin.y,
        ))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.panel = None;
        let p = self.image_point(event.position);
        if self.measure.is_some() {
            if event.button == MouseButton::Left {
                self.update_measure(p, event.modifiers.shift);
                if let Some(measurement) = self.measure.as_ref().and_then(|m| m.measurement)
                    && let Some(doc) = &mut self.document
                {
                    doc.add_annotation(
                        AnnotationKind::Measurement { measurement },
                        AnnotationStyle::default(),
                    );
                    self.status = format!("Measured {}", measurement.label());
                    self.refresh(cx);
                }
            }
            return;
        }
        let screen = ImagePoint::new(event.position.x.into(), event.position.y.into());
        let Some(doc) = &mut self.document else {
            return;
        };
        let pan =
            self.space || event.button == MouseButton::Right || event.button == MouseButton::Middle;
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
            && !event.modifiers.alt
            && matches!(self.tool, Tool::Select | Tool::Zoom)
            && let Some(circles) = magnifier_circles
        {
            let tolerance = 7.0 / self.transform.zoom();
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
                self.drag = Some(Drag {
                    start: p,
                    end: p,
                    points: vec![],
                    moving: selected,
                    last: p,
                    pan: false,
                    constrain: false,
                    handle: Some(edit),
                });
                cx.notify();
                return;
            }
        }
        if !pan
            && matches!(self.tool, Tool::Select | Tool::Arrow)
            && let Some(points) = arrow_points
            && let Some(handle) = points.iter().position(|handle| {
                (p.x - handle.x).abs() <= 7.0 / self.transform.zoom()
                    && (p.y - handle.y).abs() <= 7.0 / self.transform.zoom()
            })
        {
            let _ = doc.begin_group();
            self.drag = Some(Drag {
                start: p,
                end: p,
                points: vec![],
                moving: selected,
                last: p,
                pan: false,
                constrain: event.modifiers.shift,
                handle: Some(EditHandle::Arrow(handle)),
            });
            cx.notify();
            return;
        }
        if !pan
            && self.tool == Tool::Select
            && arrow_points.is_none()
            && magnifier_circles.is_none()
            && let Some(rect) = bounds
            && let Some(handle) = handle_at(rect, p, 7.0 / self.transform.zoom())
        {
            if selected.is_some() {
                let _ = doc.begin_group();
            }
            self.drag = Some(Drag {
                start: p,
                end: p,
                points: vec![],
                moving: selected,
                last: p,
                pan: false,
                constrain: event.modifiers.shift,
                handle: Some(EditHandle::Bounds(handle, rect)),
            });
            cx.notify();
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
            self.fit = false;
            self.transform.zoom_by(
                if event.modifiers.alt { 0.5 } else { 2.0 },
                ImagePoint::new(screen.x, screen.y - BAR),
            );
            cx.notify();
            return;
        }
        if !pan && self.tool == Tool::Text {
            self.text_origin = p;
            self.text_edit = None;
            self.panel = Some(Panel::Text);
            self.input.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            cx.notify();
            return;
        }
        if !pan
            && self.tool == Tool::Select
            && (event.modifiers.control || event.modifiers.platform)
        {
            let _ = doc.select(None);
            self.selection = doc.select_monotone_at(p, 12);
            cx.notify();
            return;
        }
        if !pan && self.tool == Tool::Counter {
            let id = doc.add_annotation(
                AnnotationKind::Counter {
                    center: p,
                    value: self.counter,
                    font_size: 28.0,
                },
                self.style,
            );
            let _ = doc.select(Some(id));
            self.counter = (self.counter + 1).min(99);
            self.refresh(cx);
            return;
        }
        let mut moving = if !pan && matches!(self.tool, Tool::Select | Tool::Arrow | Tool::Zoom) {
            let hit = doc.hit_test(p, 6.0 / self.transform.zoom()).filter(|id| {
                self.tool == Tool::Select
                    || doc
                        .annotation(*id)
                        .is_some_and(|annotation| match self.tool {
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
        if moving.is_none() && self.tool == Tool::Arrow {
            self.style.stroke_width = self.arrow_width;
        }
        let mut magnifier_edit = None;
        if let Some(annotation) = moving.and_then(|id| doc.annotation(id)).cloned() {
            self.style = annotation.style;
            magnifier_edit = annotation
                .kind
                .magnifier_part_at(p, 6.0 / self.transform.zoom())
                .map(|part| EditHandle::Magnifier(part, false));
            if event.click_count == 2
                && let AnnotationKind::Text { origin, text, .. } = &annotation.kind
            {
                self.text_origin = *origin;
                self.text_edit = Some(annotation.id);
                self.panel = Some(Panel::Text);
                self.input.update(cx, |input, cx| {
                    input.set_value(text.clone(), window, cx);
                    input.focus(window, cx);
                });
                cx.notify();
                return;
            }
            let _ = doc.begin_group();
            if event.modifiers.alt {
                let id = doc.add_annotation(annotation.kind, annotation.style);
                let _ = doc.select(Some(id));
                moving = Some(id);
                magnifier_edit = None;
            }
        }
        self.selection = None;
        self.drag = Some(Drag {
            start: p,
            end: p,
            points: vec![p],
            moving,
            last: if pan { screen } else { p },
            pan,
            constrain: event.modifiers.shift,
            handle: magnifier_edit,
        });
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.image_point(event.position);
        self.cursor = p;
        if self.measure.is_some() {
            self.update_measure(p, event.modifiers.shift);
            cx.notify();
            return;
        }
        if let Some(color) = self.visible_color(p) {
            self.sampled = color;
        }
        let Some(drag) = &mut self.drag else {
            cx.notify();
            return;
        };
        if drag.pan {
            let current = ImagePoint::new(event.position.x.into(), event.position.y.into());
            let pan = self.transform.pan();
            self.transform.set_pan(ImagePoint::new(
                pan.x + current.x - drag.last.x,
                pan.y + current.y - drag.last.y,
            ));
            self.fit = false;
            drag.last = current;
            cx.notify();
            return;
        }
        drag.end = p;
        drag.constrain = event.modifiers.shift;
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
                if event.modifiers.shift
                    && handle != 1
                    && let Some(points) = kind.arrow_points()
                {
                    point = crate::tools::constrain_angle(points[2 - handle], p);
                }
                kind.set_arrow_point(handle, point);
                let _ = doc.update_annotation(annotation.id, kind, annotation.style);
            }
        } else if let Some(EditHandle::Bounds(handle, bounds)) = drag.handle {
            let resized = resize_from_handle(bounds, handle, p, event.modifiers.shift);
            if let Some(id) = drag.moving {
                if let Some(doc) = &mut self.document {
                    let _ = doc.resize_annotation(id, resized);
                }
            } else {
                self.selection = Some(resized);
                cx.notify();
                return;
            }
        } else if let Some(id) = drag.moving {
            if let Some(doc) = &mut self.document {
                let _ =
                    doc.move_annotation(id, ImagePoint::new(p.x - drag.last.x, p.y - drag.last.y));
            }
            drag.last = p;
        } else if self.tool == Tool::Select {
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
            cx.notify();
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
                    drag.constrain,
                )
                .map(|mut kind| {
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
                        style: self.style,
                    }
                });
        }
        self.refresh(cx);
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.pan {
            cx.notify();
            return;
        }
        if let Some(doc) = &mut self.document {
            if drag.moving.is_some() {
                let _ = doc.end_group();
            } else if self.tool == Tool::Select {
                self.selection = self
                    .selection
                    .map(|r| r.clipped(doc.width(), doc.height()))
                    .filter(|r| r.width >= 1.0 && r.height >= 1.0);
            } else if let Some(annotation) = self.draft.take() {
                let id = doc.add_annotation(annotation.kind, annotation.style);
                let _ = doc.select(Some(id));
            }
        }
        self.refresh(cx);
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = match event.delta {
            ScrollDelta::Pixels(p) => ImagePoint::new(p.x.into(), p.y.into()),
            ScrollDelta::Lines(p) => ImagePoint::new(p.x * 30.0, p.y * 30.0),
        };
        if self.measure.is_some() {
            let step = if delta.y == 0.0 { delta.x } else { delta.y };
            self.measure_tolerance = (i16::from(self.measure_tolerance)
                + (step.signum() * 5.0) as i16)
                .clamp(0, 254) as u8;
            self.update_measure(self.image_point(event.position), event.modifiers.shift);
            cx.notify();
            return;
        }
        self.fit = false;
        if event.modifiers.control || event.modifiers.platform || self.tool == Tool::Zoom {
            self.transform.zoom_by(
                (delta.y * 0.003).exp(),
                ImagePoint::new(event.position.x.into(), f32::from(event.position.y) - BAR),
            );
        } else {
            let pan = self.transform.pan();
            self.transform
                .set_pan(ImagePoint::new(pan.x + delta.x, pan.y + delta.y));
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let command = modifiers.control || modifiers.platform;
        if key == "escape" {
            cx.stop_active_drag(window);
            if let Some(edit) = self.arrow_size_edit.take() {
                if edit.grouped
                    && let Some(doc) = &mut self.document
                {
                    let _ = doc.cancel_group();
                }
                self.style = edit.style;
                self.arrow_width = edit.width;
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
            self.measure = None;
            self.focus.focus(window, cx);
            self.refresh(cx);
            return;
        }
        if !command
            && !modifiers.alt
            && self.drag.is_none()
            && !matches!(
                self.panel,
                Some(Panel::Text | Panel::Cloud | Panel::Hotkeys)
            )
        {
            if key == "shift" && self.measure.is_some() {
                self.update_measure(self.cursor, true);
                cx.notify();
                return;
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
                if !event.is_held || self.measure.as_ref().is_none_or(|m| m.key != key) {
                    match doc.render_measurement_source(&render_options()) {
                        Ok((source, origin)) => {
                            self.measure = Some(MeasureSession {
                                key: key.to_owned(),
                                axis,
                                source,
                                origin,
                                outer: modifiers.shift,
                                measurement: None,
                            })
                        }
                        Err(error) => {
                            self.status = error.to_string();
                            cx.notify();
                            return;
                        }
                    }
                }
                self.panel = None;
                self.update_measure(self.cursor, modifiers.shift);
                cx.notify();
                return;
            }
        }
        if matches!(
            self.panel,
            Some(Panel::Text | Panel::Cloud | Panel::Hotkeys)
        ) {
            if key == "enter" && self.panel == Some(Panel::Text) {
                self.commit_text(window, cx);
            }
            return;
        }
        let action = if command {
            match key {
                "c" => Some(Command::Copy),
                "q" => Some(Command::Quit),
                "s" => Some(if modifiers.shift {
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
                "z" => Some(if modifiers.shift {
                    Command::Redo
                } else {
                    Command::Undo
                }),
                "y" => Some(Command::Redo),
                "1" => Some(Command::Fit),
                "0" => Some(Command::ActualSize),
                "+" | "=" => Some(Command::ZoomIn),
                "-" => Some(Command::ZoomOut),
                "," => {
                    self.panel = Some(Panel::Settings);
                    None
                }
                "a" => Some(Command::AddCapture),
                "2" => Some(Command::ZoomSelection),
                "left" | "right" | "up" | "down" => {
                    self.resize_selected(key, if modifiers.shift { 10.0 } else { 1.0 }, false, cx);
                    None
                }
                _ => None,
            }
        } else {
            match key {
                "enter" => Some(Command::Crop),
                "backspace" | "delete" => Some(Command::Delete),
                "tab" => {
                    if modifiers.shift
                        && let Some(color) = self
                            .document
                            .as_ref()
                            .and_then(|doc| doc.darkest_color_around(self.cursor, 10))
                    {
                        self.sampled = color;
                    }
                    self.copy_color(cx);
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
                        self.copy_color(cx);
                    }
                    None
                }
                "f1" => {
                    self.panel = Some(Panel::Help);
                    None
                }
                "left" | "right" | "up" | "down" => {
                    self.nudge(key, if modifiers.shift { 10.0 } else { 1.0 }, cx);
                    None
                }
                "[" | "]" => {
                    self.resize_selected(key, if modifiers.shift { 10.0 } else { 1.0 }, true, cx);
                    None
                }
                "q" | "w" => {
                    if let Some(rect) = self.selection {
                        self.fit = false;
                        let p = if key == "q" {
                            ImagePoint::new(rect.x, rect.y)
                        } else {
                            ImagePoint::new(rect.x + rect.width, rect.y + rect.height)
                        };
                        let anchor = self.canvas_point(p);
                        self.transform.set_zoom_around(4.0, anchor);
                    }
                    None
                }
                "b" => {
                    self.cycle_blur_family(cx);
                    None
                }
                _ => {
                    if let Some(tool) = Tool::from_key(key) {
                        self.select_tool(tool, cx);
                    }
                    None
                }
            }
        };
        if let Some(action) = action {
            self.command(action, window, cx);
        }
        cx.notify();
    }

    fn update_measure(&mut self, point: ImagePoint, outer: bool) {
        let Some(session) = &mut self.measure else {
            return;
        };
        session.outer = outer;
        let measurement = measure_at(
            &session.source,
            ImagePoint::new(point.x - session.origin.x, point.y - session.origin.y),
            session.axis,
            self.measure_tolerance,
            outer,
        )
        .map(|mut measurement| {
            measurement.start.x += session.origin.x;
            measurement.start.y += session.origin.y;
            measurement.end.x += session.origin.x;
            measurement.end.y += session.origin.y;
            measurement.pixels_per_unit = if self.physical_units {
                1.0
            } else {
                self.measure_scale
            };
            measurement.scale_factor = self.measure_scale;
            measurement
        });
        if measurement == session.measurement {
            return;
        }
        session.measurement = measurement;
    }

    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if self
            .measure
            .as_ref()
            .is_some_and(|session| session.key == key)
        {
            self.measure = None;
        } else if key == "shift" {
            self.update_measure(self.cursor, false);
        }
        if key == "space" {
            self.space = false;
        }
        if key == "z" {
            self.quick_zoom = false;
        }
        cx.notify();
    }

    fn nudge(&mut self, key: &str, step: f32, cx: &mut Context<Self>) {
        let delta = match key {
            "left" => ImagePoint::new(-step, 0.0),
            "right" => ImagePoint::new(step, 0.0),
            "up" => ImagePoint::new(0.0, -step),
            _ => ImagePoint::new(0.0, step),
        };
        if let Some(doc) = &mut self.document {
            if let Some(id) = doc.selected() {
                let _ = doc.move_annotation(id, delta);
                self.refresh(cx);
            } else if let Some(r) = self.selection {
                self.selection = Some(ImageRect::new(
                    (r.x + delta.x).clamp(0.0, (doc.width() as f32 - r.width).max(0.0)),
                    (r.y + delta.y).clamp(0.0, (doc.height() as f32 - r.height).max(0.0)),
                    r.width,
                    r.height,
                ));
                cx.notify();
            }
        }
    }

    fn resize_selected(&mut self, key: &str, step: f32, symmetric: bool, cx: &mut Context<Self>) {
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
            self.refresh(cx);
        } else {
            self.selection = Some(rect.clipped(doc.width(), doc.height()));
            cx.notify();
        }
    }

    pub fn add_image(
        &mut self,
        image: image::RgbaImage,
        label: &str,
        append: bool,
        cx: &mut Context<Self>,
    ) {
        if self.document.is_none() {
            self.load(Document::new(image), label, cx);
            return;
        }
        let origin = if append {
            ImagePoint::new(self.document.as_ref().unwrap().width() as f32, 0.0)
        } else {
            self.selection
                .map(|r| ImagePoint::new(r.x, r.y))
                .unwrap_or(ImagePoint::new(20.0, 20.0))
        };
        let inserted_size = if append {
            None
        } else {
            self.selection.map(|rect| sniplet_core::ImageSize {
                width: rect.width.round().max(1.0) as u32,
                height: rect.height.round().max(1.0) as u32,
            })
        };
        let mut png = Vec::new();
        use image::ImageEncoder;
        match image::codecs::png::PngEncoder::new(&mut png).write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        ) {
            Ok(()) => {
                let doc = self.document.as_mut().unwrap();
                if let Err(error) = doc.begin_group() {
                    self.status = error.to_string();
                    cx.notify();
                    return;
                }
                let (width, height) = inserted_size
                    .map(|size| (size.width, size.height))
                    .unwrap_or(image.dimensions());
                doc.expand_canvas_to(
                    (origin.x.ceil() as u32).saturating_add(width),
                    (origin.y.ceil() as u32).saturating_add(height),
                );
                if append {
                    doc.clear_crop();
                }
                let id = doc.add_annotation(
                    AnnotationKind::Image {
                        origin,
                        png_bytes: png,
                        size: inserted_size,
                    },
                    self.style,
                );
                let _ = doc.select(Some(id));
                let _ = doc.end_group();
                self.selection = None;
                self.tool = Tool::Select;
                self.status = label.to_owned();
                self.fit = true;
                self.refresh(cx);
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
            }
        }
    }

    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
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
                .unwrap_or(24.0);
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
        self.focus.focus(window, cx);
        self.refresh(cx);
    }

    pub fn command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_arrow_size_edit();
        self.panel = None;
        self.measure = None;
        match command {
            Command::Open => self.open(window, cx),
            Command::Save => self.save(false, window, cx),
            Command::SaveProject => self.save(true, window, cx),
            Command::Copy => {
                let result = self
                    .export_pixels()
                    .and_then(|image| Ok(Clipboard::new()?.set_image(&image)?));
                let hide = result.is_ok()
                    && self.selection.is_none()
                    && self.settings.hide_after_export
                    && crate::runtime::has_tray(cx);
                self.result(result, "Image copied", cx);
                if hide {
                    crate::runtime::hide_editor(window, cx);
                }
            }
            Command::Paste | Command::LoadClipboard => {
                match Clipboard::new().and_then(|mut c| c.image()) {
                    Ok(image) if matches!(command, Command::LoadClipboard) => {
                        self.load(Document::new(image), "Loaded image from clipboard", cx);
                    }
                    Ok(image) => self.add_image(image, "Pasted image", false, cx),
                    Err(error) => {
                        self.status = error.to_string();
                        cx.notify();
                    }
                }
            }
            Command::Undo | Command::Redo => {
                if let Some(doc) = &mut self.document {
                    if matches!(command, Command::Undo) {
                        doc.undo();
                    } else {
                        doc.redo();
                    }
                }
                self.selection = None;
                self.fit = true;
                self.refresh(cx);
            }
            Command::Crop => {
                if let (Some(doc), Some(rect)) = (&mut self.document, self.selection.take()) {
                    doc.set_crop(rect);
                    self.fit = true;
                }
                self.refresh(cx);
            }
            Command::AutoAdjust => {
                if let (Some(doc), Some(rect)) = (&self.document, self.selection) {
                    self.selection = doc.auto_adjust_selection(rect);
                }
                cx.notify();
            }
            Command::ResetCrop => {
                if let Some(doc) = &mut self.document {
                    doc.clear_crop();
                    self.fit = true;
                }
                self.refresh(cx);
            }
            Command::Delete => {
                if let Some(doc) = &mut self.document {
                    if let Some(id) = doc.selected() {
                        let _ = doc.delete_annotation(id);
                    } else if let Some(rect) = self.selection.take() {
                        doc.add_annotation(
                            AnnotationKind::RemoveFill { rect, sample: None },
                            self.style,
                        );
                    }
                }
                self.refresh(cx);
            }
            Command::Pin => {
                if let Ok(image) = self.export_pixels() {
                    capture::pin(image, window, cx);
                }
            }
            Command::Window => {
                self.panel = None;
                cx.notify();
                if let Err(error) = crate::window_picker::open(cx.entity(), window, cx) {
                    self.status = error;
                    cx.notify();
                }
            }
            Command::Area
            | Command::AddCapture
            | Command::Screen
            | Command::Delayed
            | Command::Scroll
            | Command::ScrollUp
            | Command::ManualScroll
            | Command::Repeat
            | Command::CaptureOcr => {
                let result = capture::start(
                    capture::CaptureRequest {
                        command,
                        monitor: self.monitor_index,
                        region: self.scroll_region,
                        last_capture: self.last_capture.clone(),
                    },
                    cx.entity(),
                    window,
                    cx,
                );
                if let Err(error) = result {
                    self.status = error;
                    cx.notify();
                }
            }
            Command::ActiveWindow => {
                if let Err(error) = capture::active_window(cx.entity(), window, cx) {
                    self.status = error;
                    cx.notify();
                }
            }
            Command::ShowEditor => crate::runtime::show_editor(window, cx),
            Command::Settings => {
                crate::runtime::show_editor(window, cx);
                self.panel = Some(Panel::Settings);
                cx.notify();
            }
            Command::GitHub => cx.open_url("https://github.com/m4tta/sniplet"),
            Command::Quit => cx.quit(),
            Command::Upload => self.upload(window, cx),
            Command::Ocr => self.ocr(window, cx),
            Command::Qr => {
                let result = self.export_pixels().and_then(|image| {
                    let codes = sniplet_platform::scan_qr_codes(&image)?;
                    if codes.is_empty() {
                        anyhow::bail!("No QR code found in the selected image");
                    }
                    let text = codes
                        .iter()
                        .map(|code| code.content.clone())
                        .collect::<Vec<_>>()
                        .join("\n");
                    Clipboard::new()?.set_text(text)?;
                    Ok(())
                });
                self.result(result, "QR contents copied", cx);
            }
            Command::Fit => {
                self.fit = true;
                cx.notify();
            }
            Command::ZoomSelection => {
                if let Some(rect) = self.selection.or_else(|| {
                    self.document.as_ref().and_then(|d| {
                        d.selected()
                            .and_then(|id| d.annotation(id))
                            .map(|a| a.kind.bounds())
                    })
                }) {
                    let view = window.viewport_size();
                    let (w, h) = (f32::from(view.width), f32::from(view.height) - BAR);
                    let zoom = ((w - 80.0) / rect.width)
                        .min((h - 80.0) / rect.height)
                        .clamp(0.05, 16.0);
                    self.transform = ViewportTransform::new(
                        zoom,
                        ImagePoint::new(
                            w / 2.0 - (rect.x + rect.width / 2.0 + self.content_origin.x) * zoom,
                            h / 2.0 - (rect.y + rect.height / 2.0 + self.content_origin.y) * zoom,
                        ),
                    );
                    self.fit = false;
                    cx.notify();
                }
            }
            Command::ActualSize | Command::ZoomIn | Command::ZoomOut => {
                self.fit = false;
                let center = ImagePoint::new(
                    f32::from(window.viewport_size().width) / 2.0,
                    (f32::from(window.viewport_size().height) - BAR) / 2.0,
                );
                if matches!(command, Command::ActualSize) {
                    self.transform.set_zoom_around(1.0, center);
                } else {
                    self.transform.zoom_by(
                        if matches!(command, Command::ZoomIn) {
                            1.25
                        } else {
                            0.8
                        },
                        center,
                    );
                }
                cx.notify();
            }
        }
    }

    pub fn export_pixels(&self) -> anyhow::Result<image::RgbaImage> {
        let doc = self
            .document
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Capture or open an image first"))?;
        if let Some(rect) = self.selection {
            let mut selected = doc.clone();
            selected.set_crop(rect);
            selected.set_backdrop(Backdrop::default());
            Ok(selected.render(&render_options())?)
        } else {
            Ok(doc.render(&render_options())?)
        }
    }

    /// Export on drag-out so the file contains the same pixels as Save and Copy.
    pub(crate) fn export_drag_payload(&self) -> anyhow::Result<ExternalDragPayload> {
        let image = self.export_pixels()?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("Sniplet-{timestamp}.png"));
        image.save(&path)?;
        Ok(ExternalDragPayload::Files(FileDragPaths::new([(
            path, false,
        )])))
    }

    fn copy_color(&mut self, cx: &mut Context<Self>) {
        let text = self.sampled.format(ColorFormat::HexRgb);
        let result = Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.clone()));
        match result {
            Ok(_) => self.status = format!("Copied {text}"),
            Err(error) => self.status = error.to_string(),
        }
        cx.notify();
    }

    fn visible_color(&self, point: ImagePoint) -> Option<Color> {
        let x = (point.x + self.content_origin.x).floor();
        let y = (point.y + self.content_origin.y).floor();
        if x < 0.0 || y < 0.0 || x >= self.image_size.0 as f32 || y >= self.image_size.1 as f32 {
            return None;
        }
        let offset = (y as usize * self.image_size.0 as usize + x as usize) * 4;
        let pixel = self.image.as_ref()?.as_bytes(0)?.get(offset..offset + 4)?;
        Some(Color::new(pixel[2], pixel[1], pixel[0], pixel[3]))
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn preview_image(&self) -> Option<Arc<RenderImage>> {
        self.image.clone()
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn sampled_color(&self) -> Color {
        self.sampled
    }

    fn result(&mut self, result: anyhow::Result<()>, message: &str, cx: &mut Context<Self>) {
        self.status = result.map_or_else(|e| e.to_string(), |_| message.to_owned());
        cx.notify();
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open image or Sniplet project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let result = if sniplet_core::Project::is_project_path(path) {
                    sniplet_core::Project::load(path).and_then(|p| p.open_document())
                } else {
                    image::open(path)
                        .map(|image| Document::new(image.to_rgba8()))
                        .map_err(sniplet_core::SnipletError::from)
                };
                let label = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let _ = this.update(cx, |this, cx| match result {
                    Ok(doc) => this.load(doc, &label, cx),
                    Err(error) => {
                        this.status = error.to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn save(&mut self, project: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut document) = self.document.clone() else {
            return;
        };
        if !project && let Some(rect) = self.selection {
            document.set_crop(rect);
            document.set_backdrop(Backdrop::default());
        }
        let format = self.settings.format;
        let hide = !project
            && self.selection.is_none()
            && self.settings.hide_after_export
            && crate::runtime::has_tray(cx);
        let directory =
            sniplet_platform::default_export_directory().unwrap_or_else(|_| std::env::temp_dir());
        let suggested = format!(
            "Sniplet.{}",
            if project {
                "sniplet"
            } else {
                format.extension()
            }
        );
        let path = cx.prompt_for_new_path(&directory, Some(&suggested));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = path.await {
                let result = (|| -> anyhow::Result<()> {
                    if project {
                        let source = path.with_extension("source.png");
                        document.original().save(&source)?;
                        document
                            .to_project(PathBuf::from(source.file_name().unwrap()))
                            .save(&path)?;
                    } else {
                        let image = document.render(&render_options())?;
                        let extension = path
                            .extension()
                            .map(|x| x.to_string_lossy().to_ascii_lowercase());
                        let chosen = match extension.as_deref() {
                            Some("jpg" | "jpeg") => sniplet_platform::ExportFormat::Jpeg,
                            Some("webp") => sniplet_platform::ExportFormat::Webp,
                            _ => sniplet_platform::ExportFormat::Png,
                        };
                        sniplet_platform::export_image(&image, &path, chosen)?;
                    }
                    Ok(())
                })();
                let saved = result.is_ok();
                let _ = this.update(cx, |this, cx| {
                    if result.is_ok() {
                        this.last_saved = Some(path.clone());
                    }
                    this.result(result, &format!("Saved {}", path.display()), cx);
                });
                if saved && hide {
                    let _ = cx.update(|window, cx| crate::runtime::hide_editor(window, cx));
                }
            }
        })
        .detach();
    }

    fn ocr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(image) = self.export_pixels() else {
            return;
        };
        self.status = "Recognizing text…".into();
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let codes = sniplet_platform::scan_qr_codes(&image)?;
            if !codes.is_empty() {
                return Ok(codes
                    .into_iter()
                    .map(|code| code.content)
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
            sniplet_platform::recognize_text(&image, &sniplet_platform::OcrOptions::default())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(text) => match Clipboard::new().and_then(|mut c| c.set_text(text.clone())) {
                        Ok(_) => {
                            this.status =
                                format!("Text copied · {} characters", text.chars().count())
                        }
                        Err(error) => this.status = error.to_string(),
                    },
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.settings.cloud_upload.clone() else {
            self.panel = Some(Panel::Cloud);
            cx.notify();
            return;
        };
        let image = match self.export_pixels() {
            Ok(image) => image,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                return;
            }
        };
        self.status = "Uploading image…".into();
        cx.notify();
        let key = format!(
            "Sniplet-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let task = cx
            .background_executor()
            .spawn(async move { sniplet_platform::upload_image(&image, &key, &config) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(upload) => match Clipboard::new().and_then(|mut c| c.set_text(upload.url)) {
                        Ok(_) => "Uploaded · image link copied".into(),
                        Err(error) => format!("Uploaded, but could not copy the link: {error}"),
                    },
                    Err(error) => format!("Upload failed: {error}"),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn arrow_size_event(&mut self, event: &SliderEvent, cx: &mut Context<Self>) {
        match event {
            SliderEvent::Change(value) => {
                if self.arrow_size_edit.is_none() {
                    let grouped = self.document.as_mut().is_some_and(|doc| {
                        let arrow = doc
                            .selected()
                            .and_then(|id| doc.annotation(id))
                            .is_some_and(|annotation| {
                                matches!(annotation.kind, AnnotationKind::Arrow { .. })
                            });
                        arrow && doc.begin_group().is_ok()
                    });
                    self.arrow_size_edit = Some(ArrowSizeEdit {
                        style: self.style,
                        width: self.arrow_width,
                        grouped,
                    });
                }
                self.change_style(None, Some(value.end().clamp(1.0, 30.0)), None, cx);
            }
            SliderEvent::Release(_) => self.finish_arrow_size_edit(),
        }
    }

    fn finish_arrow_size_edit(&mut self) {
        if let Some(edit) = self.arrow_size_edit.take()
            && edit.grouped
            && let Some(doc) = &mut self.document
        {
            let _ = doc.end_group();
        }
    }

    fn change_arrow_variant(&mut self, variant: ArrowVariant, cx: &mut Context<Self>) {
        self.finish_arrow_size_edit();
        self.arrow_variant = variant;
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
        {
            let mut kind = annotation.kind;
            if let AnnotationKind::Arrow {
                variant: current, ..
            } = &mut kind
            {
                *current = variant;
                let _ = doc.update_annotation(annotation.id, kind, annotation.style);
            }
        }
        self.refresh(cx);
    }

    fn change_style(
        &mut self,
        color: Option<Color>,
        width: Option<f32>,
        fill: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if let Some(color) = color {
            self.style.stroke = color;
            self.style.fill = self.style.fill.map(|_| color);
        }
        if let Some(width) = width {
            self.style.stroke_width = width;
            let arrow = self
                .document
                .as_ref()
                .and_then(|doc| doc.selected().and_then(|id| doc.annotation(id)))
                .map_or(self.tool == Tool::Arrow, |annotation| {
                    matches!(annotation.kind, AnnotationKind::Arrow { .. })
                });
            if arrow {
                self.arrow_width = width;
            } else {
                self.drawing_width = width;
            }
        }
        if let Some(fill) = fill {
            self.style.fill = fill.then_some(self.style.stroke);
        }
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
        {
            let _ = doc.update_annotation(annotation.id, annotation.kind, self.style);
        }
        self.refresh(cx);
    }

    fn cycle_blur_family(&mut self, cx: &mut Context<Self>) {
        let selected = self.document.as_ref().and_then(|doc| {
            doc.selected().and_then(|id| doc.annotation(id)).and_then(
                |annotation| match &annotation.kind {
                    AnnotationKind::Blur { .. } => Some(Tool::Blur),
                    AnnotationKind::Pixelate { .. } => Some(Tool::Pixelate),
                    AnnotationKind::RemoveFill { .. } => Some(Tool::Erase),
                    AnnotationKind::Redaction { .. } => Some(Tool::Redact),
                    _ => None,
                },
            )
        });
        let tool = match self.tool {
            Tool::Blur => Tool::Pixelate,
            Tool::Pixelate => Tool::Erase,
            Tool::Erase => Tool::Redact,
            Tool::Redact => Tool::Blur,
            _ => selected.unwrap_or(Tool::Blur),
        };
        self.set_blur_family(tool, cx);
    }

    fn set_blur_family(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tool = tool;
        self.selection = None;
        self.panel = None;
        self.status = tool.label().to_owned();
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
        {
            let source_tool = match &annotation.kind {
                AnnotationKind::Blur { .. } => Some(Tool::Blur),
                AnnotationKind::Pixelate { .. } => Some(Tool::Pixelate),
                AnnotationKind::RemoveFill { .. } => Some(Tool::Erase),
                AnnotationKind::Redaction { .. } => Some(Tool::Redact),
                _ => None,
            };
            let rect = match &annotation.kind {
                AnnotationKind::Blur { rect, .. }
                | AnnotationKind::Pixelate { rect, .. }
                | AnnotationKind::RemoveFill { rect, .. }
                | AnnotationKind::Redaction { rect } => Some(*rect),
                _ => None,
            };
            if source_tool.is_none() {
                let _ = doc.select(None);
            }
            if let Some(rect) = rect
                && source_tool != Some(tool)
            {
                let kind = match tool {
                    Tool::Blur => AnnotationKind::Blur {
                        rect,
                        radius: match &annotation.kind {
                            AnnotationKind::Blur { radius, .. } => *radius,
                            _ => 8.0,
                        },
                    },
                    Tool::Pixelate => AnnotationKind::Pixelate {
                        rect,
                        block_size: match &annotation.kind {
                            AnnotationKind::Pixelate { block_size, .. } => *block_size,
                            _ => 12,
                        },
                    },
                    Tool::Erase => AnnotationKind::RemoveFill {
                        rect,
                        sample: match &annotation.kind {
                            AnnotationKind::RemoveFill { sample, .. } => *sample,
                            _ => None,
                        },
                    },
                    Tool::Redact => AnnotationKind::Redaction { rect },
                    _ => return,
                };
                let _ = doc.update_annotation(annotation.id, kind, annotation.style);
            }
        }
        self.refresh(cx);
    }

    fn adjust_text_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
            && let AnnotationKind::Text {
                origin,
                text,
                font_size,
            } = annotation.kind
        {
            let kind = AnnotationKind::Text {
                origin,
                text,
                font_size: (font_size + delta).clamp(6.0, 192.0),
            };
            let _ = doc.update_annotation(annotation.id, kind, annotation.style);
        }
        self.refresh(cx);
    }

    fn adjust_counter_value(&mut self, delta: i32, cx: &mut Context<Self>) {
        let mut next_counter = None;
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
            && let AnnotationKind::Counter {
                center,
                value,
                font_size,
            } = annotation.kind
        {
            let value = (value as i32 + delta).clamp(0, 99) as u32;
            let kind = AnnotationKind::Counter {
                center,
                value,
                font_size,
            };
            let _ = doc.update_annotation(annotation.id, kind, annotation.style);
            next_counter = Some((value + 1).min(99));
        }
        if let Some(next_counter) = next_counter {
            self.counter = next_counter;
        }
        self.refresh(cx);
    }

    fn adjust_magnifier_zoom(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
            && let AnnotationKind::Magnifier { rect, zoom, source } = annotation.kind
        {
            let kind = AnnotationKind::Magnifier {
                rect,
                zoom: (zoom + delta).clamp(1.0, 16.0),
                source,
            };
            let _ = doc.update_annotation(annotation.id, kind, annotation.style);
        }
        self.refresh(cx);
    }

    fn adjust_spotlight_dim(&mut self, delta: i32, cx: &mut Context<Self>) {
        let current = self
            .document
            .as_ref()
            .and_then(|doc| doc.selected().and_then(|id| doc.annotation(id)))
            .filter(|annotation| matches!(&annotation.kind, AnnotationKind::Spotlight { .. }))
            .and_then(|annotation| annotation.style.fill)
            .or(self.style.fill)
            .map(|color| ((color.a as f32 / 25.0).round() as i32).clamp(1, 9))
            .unwrap_or(6);
        let level = (current + delta).clamp(1, 9);
        let fill = Color::new(0, 0, 0, (level * 25) as u8);
        self.style.fill = Some(fill);
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
            && matches!(&annotation.kind, AnnotationKind::Spotlight { .. })
        {
            let mut style = annotation.style;
            style.fill = Some(fill);
            let _ = doc.update_annotation(annotation.id, annotation.kind, style);
        }
        self.refresh(cx);
    }

    fn toolbar_button(
        &self,
        id: &'static str,
        icon: &'static str,
        tooltip: &'static str,
        command: Command,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .ghost()
            .icon(tool_icon(icon))
            .tooltip(tooltip)
            .w(px(34.0))
            .h(px(36.0))
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
    }

    fn panel_button(
        &self,
        id: &'static str,
        label: &'static str,
        panel: Panel,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .ghost()
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.panel = Some(panel);
                cx.notify();
            }))
    }

    fn menu_row(
        &self,
        id: &'static str,
        label: &'static str,
        shortcut: &'static str,
        command: Command,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                Button::new(id)
                    .ghost()
                    .label(label)
                    .w(px(225.0))
                    .on_click(cx.listener(move |this, _, w, cx| this.command(command, w, cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(shortcut),
            )
    }

    fn arrow_properties(&self) -> Option<(f32, ArrowVariant)> {
        let selected = self
            .document
            .as_ref()
            .and_then(|doc| doc.selected().and_then(|id| doc.annotation(id)));
        match selected {
            Some(Annotation {
                kind: AnnotationKind::Arrow { variant, .. },
                style,
                ..
            }) => Some((style.stroke_width, *variant)),
            None if self.tool == Tool::Arrow => Some((self.arrow_width, self.arrow_variant)),
            _ => None,
        }
    }

    fn properties(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = [
            Color::new(255, 0, 0, 255),
            Color::new(255, 153, 0, 255),
            Color::new(255, 218, 48, 255),
            Color::new(39, 174, 96, 255),
            Color::new(0, 122, 255, 255),
            Color::new(158, 71, 238, 255),
            Color::WHITE,
            Color::BLACK,
        ];
        let selected = self
            .document
            .as_ref()
            .and_then(|doc| doc.selected().and_then(|id| doc.annotation(id)).cloned());
        let arrow = self.arrow_properties();
        let family = selected
            .as_ref()
            .and_then(|annotation| match &annotation.kind {
                AnnotationKind::Blur { .. } => Some(Tool::Blur),
                AnnotationKind::Pixelate { .. } => Some(Tool::Pixelate),
                AnnotationKind::RemoveFill { .. } => Some(Tool::Erase),
                AnnotationKind::Redaction { .. } => Some(Tool::Redact),
                _ => None,
            })
            .or(match self.tool {
                Tool::Blur | Tool::Pixelate | Tool::Erase | Tool::Redact => Some(self.tool),
                _ => None,
            });
        let text_size = selected
            .as_ref()
            .and_then(|annotation| match &annotation.kind {
                AnnotationKind::Text { font_size, .. } => Some(*font_size),
                _ => None,
            });
        let counter_value = selected
            .as_ref()
            .and_then(|annotation| match &annotation.kind {
                AnnotationKind::Counter { value, .. } => Some(*value),
                _ => None,
            });
        let magnifier_zoom = selected
            .as_ref()
            .and_then(|annotation| match &annotation.kind {
                AnnotationKind::Magnifier { zoom, .. } => Some(*zoom),
                _ => None,
            });
        let spotlight_selected = selected
            .as_ref()
            .is_some_and(|annotation| matches!(&annotation.kind, AnnotationKind::Spotlight { .. }));
        let spotlight_level = (spotlight_selected || self.tool == Tool::Spotlight).then(|| {
            let fill = selected
                .as_ref()
                .filter(|annotation| matches!(&annotation.kind, AnnotationKind::Spotlight { .. }))
                .and_then(|annotation| annotation.style.fill)
                .or(self.style.fill);
            fill.map(|color| ((color.a as f32 / 25.0).round() as i32).clamp(1, 9))
                .unwrap_or(6)
        });

        let mut properties = div()
            .occlude()
            .absolute()
            .top(px(12.0))
            .right(px(16.0))
            .flex()
            .items_center()
            .gap_2()
            .p_2()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .bg(cx.theme().popover.opacity(0.95))
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .shadow_sm()
            .children(colors.into_iter().enumerate().map(|(index, color)| {
                let color_button = div()
                    .id(("color", index))
                    .size(px(21.0))
                    .rounded_md()
                    .bg(color_ui(color))
                    .border_1()
                    .border_color(if self.style.stroke == color {
                        cx.theme().foreground
                    } else {
                        cx.theme().border
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_style(Some(color), None, None, cx)
                    }));
                #[cfg(feature = "ui-tests")]
                let color_button = {
                    use gpui_kit::test::TestSupportExt;
                    color_button.test_support()
                };
                color_button
            }))
            .child(div().w(px(1.0)).h(px(22.0)).bg(cx.theme().border));

        if let Some((width, active)) = arrow {
            let mut variants = div().flex().gap(px(1.0)).rounded_md().bg(cx.theme().accent);
            for (variant, label) in [
                (ArrowVariant::Solid, "Solid arrow"),
                (ArrowVariant::HandDrawn, "Hand drawn arrow"),
                (ArrowVariant::Thin, "Thin arrow"),
                (ArrowVariant::DoubleEnded, "Double ended arrow"),
            ] {
                let selected = active == variant;
                let color: Hsla = if selected {
                    rgb(0xffffff).into()
                } else {
                    cx.theme().foreground
                };
                variants = variants.child(
                    Button::new(("arrow-variant", variant as usize))
                        .ghost()
                        .compact()
                        .selected(selected)
                        .toggled(selected)
                        .tooltip(label)
                        .w(px(34.0))
                        .h(px(32.0))
                        .px_1()
                        .when(selected, |button| button.bg(rgb(0x007aff)))
                        .child(crate::arrow_palette::variant_icon(variant, color))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.change_arrow_variant(variant, cx)
                        })),
                );
            }
            properties = properties
                .child(crate::arrow_palette::size_slider(&self.arrow_size, cx))
                .child(
                    div()
                        .id("arrow-size-value")
                        .w(px(34.0))
                        .text_xs()
                        .child(format!("{width:.2}")),
                )
                .child(variants);
        } else {
            properties = properties
                .children([2.0_f32, 4.0, 8.0].into_iter().map(|width| {
                    Button::new(("width", width as usize))
                        .ghost()
                        .label(format!("{width:.0}"))
                        .selected(self.style.stroke_width == width)
                        .w(px(40.0))
                        .px_2()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.change_style(None, Some(width), None, cx)
                        }))
                }))
                .child(
                    Button::new("fill")
                        .ghost()
                        .icon(tool_icon("paint-bucket"))
                        .tooltip("Toggle fill")
                        .selected(self.style.fill.is_some())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.change_style(None, None, Some(this.style.fill.is_none()), cx)
                        })),
                );
        }

        if let Some(active) = family {
            properties = properties.child(div().w(px(1.0)).h(px(22.0)).bg(cx.theme().border));
            for (id, label, tool) in [
                ("family-blur", "Blur", Tool::Blur),
                ("family-pixelate", "Pixelate", Tool::Pixelate),
                ("family-erase", "Erase", Tool::Erase),
                ("family-redact", "Redact", Tool::Redact),
            ] {
                properties = properties.child(
                    Button::new(id)
                        .ghost()
                        .label(label)
                        .selected(active == tool)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_blur_family(tool, cx)),
                        ),
                );
            }
        }

        if let Some(font_size) = text_size {
            properties = properties
                .child(div().w(px(1.0)).h(px(22.0)).bg(cx.theme().border))
                .child(
                    Button::new("text-size-down")
                        .ghost()
                        .label("−")
                        .on_click(cx.listener(|this, _, _, cx| this.adjust_text_size(-2.0, cx))),
                )
                .child(div().text_sm().child(format!("{font_size:.0} pt")))
                .child(
                    Button::new("text-size-up")
                        .ghost()
                        .label("+")
                        .on_click(cx.listener(|this, _, _, cx| this.adjust_text_size(2.0, cx))),
                );
        }

        if let Some(value) = counter_value {
            properties = properties
                .child(div().w(px(1.0)).h(px(22.0)).bg(cx.theme().border))
                .child(
                    Button::new("counter-value-down")
                        .ghost()
                        .label("−")
                        .on_click(cx.listener(|this, _, _, cx| this.adjust_counter_value(-1, cx))),
                )
                .child(div().text_sm().child(value.to_string()))
                .child(
                    Button::new("counter-value-up")
                        .ghost()
                        .label("+")
                        .on_click(cx.listener(|this, _, _, cx| this.adjust_counter_value(1, cx))),
                );
        }

        if let Some(zoom) = magnifier_zoom {
            properties = properties
                .child(div().w(px(1.0)).h(px(22.0)).bg(cx.theme().border))
                .child(
                    Button::new("magnifier-zoom-down")
                        .ghost()
                        .label("−")
                        .on_click(
                            cx.listener(|this, _, _, cx| this.adjust_magnifier_zoom(-1.0, cx)),
                        ),
                )
                .child(div().text_sm().child(format!("{zoom:.1}×")))
                .child(
                    Button::new("magnifier-zoom-up")
                        .ghost()
                        .label("+")
                        .on_click(
                            cx.listener(|this, _, _, cx| this.adjust_magnifier_zoom(1.0, cx)),
                        ),
                );
        }

        if let Some(level) = spotlight_level {
            properties = properties
                .child(div().w(px(1.0)).h(px(22.0)).bg(cx.theme().border))
                .child(
                    Button::new("spotlight-dim-down")
                        .ghost()
                        .label("−")
                        .on_click(cx.listener(|this, _, _, cx| this.adjust_spotlight_dim(-1, cx))),
                )
                .child(div().text_sm().child(format!("Dim {level}")))
                .child(
                    Button::new("spotlight-dim-up")
                        .ghost()
                        .label("+")
                        .on_click(cx.listener(|this, _, _, cx| this.adjust_spotlight_dim(1, cx))),
                );
        }

        properties
    }

    fn measure_units(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let units = if self.physical_units {
            1.0
        } else {
            self.measure_scale
        };
        let label = format!(
            "{:.0} × {:.0} {}",
            self.image_size.0 as f32 / units,
            self.image_size.1 as f32 / units,
            if self.physical_units { "px" } else { "pt" }
        );
        let control = div()
            .id("measure-units")
            .flex()
            .flex_col()
            .px_3()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(label)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Image size"),
            )
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.physical_units = !this.physical_units;
                let outer = this.measure.as_ref().is_some_and(|m| m.outer);
                this.update_measure(this.cursor, outer);
                cx.notify();
            }));
        #[cfg(feature = "ui-tests")]
        let control = {
            use gpui_kit::test::TestSupportExt;
            control.test_support()
        };
        control
    }

    fn render_image_drag(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let grip = || {
            div().flex().gap(px(2.5)).children((0..2).map(|_| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.5))
                    .children((0..3).map(|_| {
                        div()
                            .size(px(1.5))
                            .rounded_full()
                            .bg(cx.theme().muted_foreground)
                    }))
            }))
        };
        let control = div()
            .id("drag-file")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .gap(px(3.0))
            .w(px(44.0))
            .h(px(26.0))
            .rounded_full()
            .bg(cx.theme().foreground.opacity(0.08))
            .hover(|style| style.bg(cx.theme().foreground.opacity(0.12)))
            .child(grip())
            .child(tool_icon("file").size(px(17.0)))
            .child(grip())
            .tooltip(|window, cx| {
                component::tooltip::Tooltip::new("Drag'n'Drop Image")
                    .rounded_full()
                    .px(px(14.0))
                    .py(px(6.0))
                    .text_size(px(14.0))
                    .build(window, cx)
            })
            .when(self.document.is_none(), |control| control.opacity(0.4))
            .when(self.document.is_some(), |control| {
                control
                    .cursor(CursorStyle::OpenHand)
                    .on_drag(cx.entity(), |_, _, _, cx| cx.new(|_| Empty))
                    .external_drag_payload(|editor: &Entity<Editor>, _, cx| {
                        editor.update(cx, |editor, cx| match editor.export_drag_payload() {
                            Ok(payload) => Some(payload),
                            Err(error) => {
                                editor.status = format!("Could not drag image: {error}");
                                cx.notify();
                                None
                            }
                        })
                    })
            });
        #[cfg(feature = "ui-tests")]
        let control = {
            use gpui_kit::test::TestSupportExt;
            control.test_support()
        };
        control
    }

    fn render_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = self.panel else {
            return div().into_any_element();
        };
        let mut body = div()
            .id("floating-panel")
            .flex()
            .flex_col()
            .gap_2()
            .w(px(340.0))
            .max_h(window.viewport_size().height - px(BAR + 32.0))
            .overflow_y_scroll()
            .p_4()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        body = body.child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(match panel {
                    Panel::Menu => "Sniplet",
                    Panel::Tools => "Tools",
                    Panel::Text => "Text annotation",
                    Panel::Backdrop => "Backdrop",
                    Panel::Settings => "Settings",
                    Panel::Cloud => "Cloud upload",
                    Panel::Hotkeys => "Capture shortcuts",
                    Panel::Help => "Keyboard shortcuts",
                    Panel::Ruler => "Use the keyboard",
                }))
                .child(
                    Button::new("close-panel")
                        .ghost()
                        .icon(tool_icon("x"))
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.panel = None;
                            this.focus.focus(w, cx);
                            cx.notify();
                        })),
                ),
        );
        match panel {
            Panel::Ruler => {
                body = body.child("Hold 1 or Left/Right to measure the width. Hold 2 or Up/Down to measure the height. Move the pointer to the area or gap.")
                    .child("Hold Shift to include the outer edges. Use the wheel to adjust sensitivity. Click to place the measurement on the image.")
                    .child("Click the image size to switch between points and physical pixels.");
            }
            Panel::Menu => {
                for (id, label, key, command) in [
                    (
                        "capture-area",
                        "Capture area",
                        "Ctrl/Cmd Shift 2",
                        Command::Area,
                    ),
                    (
                        "capture-screen",
                        "Capture screen",
                        "Ctrl/Cmd Shift 1",
                        Command::Screen,
                    ),
                    (
                        "capture-window",
                        "Capture window",
                        "Ctrl/Cmd Shift 3",
                        Command::Window,
                    ),
                    (
                        "capture-repeat",
                        "Repeat area capture",
                        "Ctrl/Cmd Shift 5",
                        Command::Repeat,
                    ),
                    (
                        "capture-active",
                        "Capture active window",
                        "Ctrl/Cmd Shift 6",
                        Command::ActiveWindow,
                    ),
                    (
                        "capture-ocr",
                        "Capture text / QR",
                        "Ctrl/Cmd Shift 7",
                        Command::CaptureOcr,
                    ),
                    (
                        "capture-delayed",
                        "Capture after 3 seconds",
                        "",
                        Command::Delayed,
                    ),
                    (
                        "capture-scroll",
                        "Scrolling capture",
                        "Ctrl/Cmd Shift 4",
                        Command::Scroll,
                    ),
                    (
                        "capture-scroll-manual",
                        "Manual scrolling capture",
                        "",
                        Command::ManualScroll,
                    ),
                    ("open-image", "Open image…", "Ctrl/Cmd O", Command::Open),
                    (
                        "paste-image",
                        "Load from clipboard",
                        "Ctrl/Cmd V",
                        Command::Paste,
                    ),
                    ("ocr", "Recognize text", "", Command::Ocr),
                    ("qr", "Read QR code", "", Command::Qr),
                    (
                        "auto-adjust",
                        "Auto-adjust selection",
                        "",
                        Command::AutoAdjust,
                    ),
                    ("reset-crop", "Reset crop", "", Command::ResetCrop),
                    ("quit-sniplet", "Quit Sniplet", "Ctrl/Cmd Q", Command::Quit),
                ] {
                    body = body.child(self.menu_row(id, label, key, command, cx));
                }
                body = body
                    .child(self.panel_button("settings", "Settings…", Panel::Settings, cx))
                    .child(self.panel_button("help", "Keyboard shortcuts", Panel::Help, cx));
            }
            Panel::Tools => {
                body = body.child(self.menu_row(
                    "add-capture",
                    "Add capture",
                    "Ctrl/Cmd A",
                    Command::AddCapture,
                    cx,
                ));
                for tool in Tool::ALL {
                    body = body.child(
                        Button::new(("more-tool", tool as usize))
                            .ghost()
                            .icon(tool_icon(tool.icon()))
                            .label(format!("{}    {}", tool.label(), tool.key()))
                            .selected(self.tool == tool)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_tool(tool, cx)),
                            ),
                    );
                }
            }
            Panel::Text => {
                body = body
                    .child(Input::new(&self.input).id("annotation-text"))
                    .child(
                        Button::new("add-text")
                            .primary()
                            .label(if self.text_edit.is_some() {
                                "Update text"
                            } else {
                                "Add text"
                            })
                            .on_click(cx.listener(|this, _, w, cx| this.commit_text(w, cx))),
                    );
            }
            Panel::Backdrop => {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Choose a background. Export includes padding, corners, and shadow.",
                        ),
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
                    body = body.child(
                        Button::new(("backdrop", index))
                            .ghost()
                            .label(name)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(doc) = &mut this.document {
                                    doc.set_backdrop(Backdrop {
                                        padding: 56,
                                        corner_radius: 12,
                                        background: Background::LinearGradient {
                                            start: hex_color(a),
                                            end: hex_color(b),
                                            angle_degrees: 135.0,
                                        },
                                        shadow: Some(Shadow {
                                            offset: ImagePoint::new(0.0, 8.0),
                                            blur_radius: 16.0,
                                            color: Color::new(0, 0, 0, 75),
                                        }),
                                    });
                                }
                                this.fit = true;
                                this.refresh(cx);
                            })),
                    );
                }
                body = body.child(
                    Button::new("no-backdrop")
                        .ghost()
                        .label("Remove backdrop")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(doc) = &mut this.document {
                                doc.set_backdrop(Backdrop::default());
                            }
                            this.fit = true;
                            this.refresh(cx);
                        })),
                );
                for (label, padding) in [
                    ("Compact · 24 px", 24),
                    ("Comfortable · 56 px", 56),
                    ("Wide · 100 px", 100),
                ] {
                    body = body.child(
                        Button::new(("padding", padding as usize))
                            .ghost()
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(doc) = &mut this.document {
                                    let mut backdrop = doc.backdrop();
                                    backdrop.padding = padding;
                                    doc.set_backdrop(backdrop);
                                }
                                this.fit = true;
                                this.refresh(cx);
                            })),
                    );
                }
                body = body
                    .child(
                        Button::new("toggle-corners")
                            .ghost()
                            .label("Toggle rounded corners")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(doc) = &mut this.document {
                                    let mut b = doc.backdrop();
                                    b.corner_radius = if b.corner_radius == 0 { 12 } else { 0 };
                                    doc.set_backdrop(b);
                                }
                                this.refresh(cx);
                            })),
                    )
                    .child(
                        Button::new("toggle-shadow")
                            .ghost()
                            .label("Toggle shadow")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(doc) = &mut this.document {
                                    let mut b = doc.backdrop();
                                    b.shadow = if b.shadow.is_some() {
                                        None
                                    } else {
                                        Some(Shadow {
                                            offset: ImagePoint::new(0.0, 8.0),
                                            blur_radius: 16.0,
                                            color: Color::new(0, 0, 0, 75),
                                        })
                                    };
                                    doc.set_backdrop(b);
                                }
                                this.refresh(cx);
                            })),
                    );
            }
            Panel::Settings => {
                body = body.child(div().text_sm().child("Appearance")).child(
                    div().flex().gap_2().children(
                        [
                            ("theme-system", "System", ThemePreference::System),
                            ("theme-light", "Light", ThemePreference::Light),
                            ("theme-dark", "Dark", ThemePreference::Dark),
                        ]
                        .map(|(id, label, preference)| {
                            Button::new(id)
                                .ghost()
                                .label(label)
                                .selected(self.settings.theme == preference)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.settings.theme = preference;
                                    crate::theme::apply(preference, window, cx);
                                    this.persist_settings(cx);
                                }))
                        }),
                    ),
                );
                body = body.child(div().text_sm().child("Image format"));
                for format in [
                    sniplet_platform::ExportFormat::Png,
                    sniplet_platform::ExportFormat::Jpeg,
                    sniplet_platform::ExportFormat::Webp,
                ] {
                    body = body.child(
                        Button::new(format.extension())
                            .ghost()
                            .label(format.extension().to_uppercase())
                            .selected(self.settings.format == format)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings.format = format;
                                this.persist_settings(cx);
                            })),
                    );
                }
                body = body
                    .child(
                        Button::new("auto-copy")
                            .ghost()
                            .label(format!(
                                "{} Copy captures automatically",
                                if self.settings.auto_copy {
                                    "✓"
                                } else {
                                    "○"
                                }
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.auto_copy = !this.settings.auto_copy;
                                this.persist_settings(cx);
                            })),
                    )
                    .child(
                        Button::new("hide-after-export")
                            .ghost()
                            .label(format!(
                                "{} Hide editor after copy / save",
                                if self.settings.hide_after_export {
                                    "✓"
                                } else {
                                    "○"
                                }
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.hide_after_export = !this.settings.hide_after_export;
                                this.persist_settings(cx);
                            })),
                    )
                    .child(
                        Button::new("always-on-top")
                            .ghost()
                            .label(format!(
                                "{} Keep editor on top (restart)",
                                if self.settings.always_on_top {
                                    "✓"
                                } else {
                                    "○"
                                }
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.always_on_top = !this.settings.always_on_top;
                                this.persist_settings(cx);
                            })),
                    )
                    .child(self.panel_button(
                        "hotkey-settings",
                        "Capture shortcuts…",
                        Panel::Hotkeys,
                        cx,
                    ))
                    .child(self.panel_button("cloud-settings", "Cloud upload…", Panel::Cloud, cx));
                if let Ok(monitors) = sniplet_platform::list_monitors() {
                    body = body.child(div().mt_2().child("Capture display"));
                    for monitor in monitors {
                        let index = monitor.index;
                        body = body.child(
                            Button::new(("monitor", index))
                                .ghost()
                                .label(format!(
                                    "{} · {} × {}",
                                    monitor.name, monitor.width, monitor.height
                                ))
                                .selected(self.monitor_index == index)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.monitor_index = index;
                                    cx.notify();
                                })),
                        );
                    }
                }
                body = body.child(
                    Button::new("reveal-settings")
                        .ghost()
                        .label("Show settings file")
                        .on_click(|_, _, cx| {
                            if let Ok(store) = SettingsStore::for_app() {
                                cx.reveal_path(store.path());
                            }
                        }),
                );
            }
            Panel::Cloud => {
                body = body.child(div().text_sm().child("Configure a signed upload URL. Upload sends the image only when you click the cloud button."))
                    .child(div().text_xs().child("Signed PUT URL")).child(Input::new(&self.cloud_url).id("upload-url"))
                    .child(div().text_xs().child("Image link")).child(Input::new(&self.cloud_public_url).id("upload-public-url"))
                    .child(Button::new("save-upload").primary().label("Save destination").on_click(cx.listener(|this, _, w, cx| {
                        let url = this.cloud_url.read(cx).value().trim().to_owned(); let public_url = this.cloud_public_url.read(cx).value().trim().to_owned();
                        if !url.starts_with("https://") && !url.starts_with("http://localhost") && !url.starts_with("http://127.0.0.1") { this.status = "Enter an HTTPS upload URL".into(); cx.notify(); return; }
                        this.settings.cloud_upload = Some(sniplet_platform::CloudUploadConfig::PresignedPut { url, public_url: (!public_url.is_empty()).then_some(public_url) }); this.persist_settings(cx); this.panel = None; this.focus.focus(w, cx);
                    })))
                    .child(Button::new("disable-upload").ghost().label("Disable upload").on_click(cx.listener(|this, _, _, cx| { this.settings.cloud_upload = None; this.persist_settings(cx); })))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("S3-compatible destinations are configured in settings.json. Credentials use AWS environment variables."));
            }
            Panel::Hotkeys => {
                for (i, label) in [
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
                    body = body
                        .child(div().text_xs().child(label))
                        .child(Input::new(&self.hotkey_inputs[i]).id(("hotkey-input", i)));
                }
                body = body.child(
                    Button::new("save-hotkeys")
                        .primary()
                        .label("Save shortcuts · restart to apply")
                        .on_click(cx.listener(|this, _, w, cx| {
                            let values = this
                                .hotkey_inputs
                                .each_ref()
                                .map(|input| input.read(cx).value().trim().to_owned());
                            if let Err(error) = crate::runtime::validate_hotkeys(&values) {
                                this.status = error;
                                cx.notify();
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
                            this.settings.hotkeys = sniplet_platform::HotkeySettings {
                                capture_area,
                                capture_screen,
                                capture_window,
                                scrolling_capture,
                                repeat_area,
                                active_window,
                                capture_ocr,
                                show_editor,
                            };
                            this.persist_settings(cx);
                            this.panel = None;
                            this.focus.focus(w, cx);
                        })),
                ).child(div().text_xs().text_color(cx.theme().muted_foreground).child("Leave a shortcut empty to disable it. Each enabled shortcut must be unique."));
            }
            Panel::Help => {
                for text in [
                    "V Select / Crop    Enter Apply crop",
                    "A Arrow · drag its middle handle to bend",
                    "O Oval    L Line    C Counter / average color",
                    "B Blur / Pixelate / Erase / Redact    H Highlighter    S Spotlight",
                    "Z + click Quick zoom    Alt Zoom out",
                    "Shift Constrain shapes / angles",
                    "Space + drag or right-drag Pan",
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
                    body = body.child(div().text_sm().child(text));
                }
            }
        }
        let _ = window;
        div()
            .absolute()
            .top(px(10.0))
            .right(px(14.0))
            .child(body)
            .into_any_element()
    }

    fn persist_settings(&mut self, cx: &mut Context<Self>) {
        let store = SettingsStore::for_app();
        #[cfg(test)]
        let store = self
            .settings_path
            .as_ref()
            .map_or(store, |path| Ok(SettingsStore::at(path)));
        match store.and_then(|s| s.save(&self.settings)) {
            Ok(_) => self.status = "Settings saved".into(),
            Err(e) => self.status = e.to_string(),
        }
        cx.notify();
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let measurement = self
            .measure
            .as_ref()
            .and_then(|session| session.measurement);
        if measurement != self.measure_painted {
            self.measure_painted = measurement;
            if let Some((_, previous)) = self.measure_overlay.take() {
                let _ = window.drop_image(previous);
            }
            self.measure_overlay = measurement
                .and_then(|m| m.overlay(FONT).ok())
                .map(|(bounds, image)| (bounds, display_image(image)));
        }
        if self.preview_dirty {
            let preview_started = std::time::Instant::now();
            self.preview_dirty = false;
            if let Some(doc) = &self.document {
                let options = RenderOptions {
                    extra_annotations: self.draft.as_slice(),
                    ..render_options()
                };
                match doc.render(&options) {
                    Ok(image) => {
                        self.image_size = image.dimensions();
                        self.content_origin = doc.render_content_origin(&options);
                        if let Some(previous) = self.image.replace(display_image(image)) {
                            let _ = window.drop_image(previous);
                        }
                        #[cfg(all(test, feature = "ui-tests"))]
                        {
                            self.preview_frames += 1;
                        }
                        if let Some(color) = self.visible_color(self.cursor) {
                            self.sampled = color;
                        }
                    }
                    Err(error) => self.status = error.to_string(),
                }
                if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
                    eprintln!(
                        "capture: editor preview ready in {:?}",
                        preview_started.elapsed()
                    );
                }
            }
        }
        if let Some((width, _)) = self.arrow_properties()
            && self.arrow_size.read(cx).value().end() != width
        {
            self.arrow_size
                .update(cx, |slider, cx| slider.set_value(width, window, cx));
        }
        let size = window.viewport_size();
        let width = f32::from(size.width);
        let height = f32::from(size.height) - BAR;
        if self.fit && self.image_size.0 > 0 {
            let zoom = ((width - 48.0) / self.image_size.0 as f32)
                .min((height - 48.0) / self.image_size.1 as f32)
                .clamp(0.01, 1.0);
            self.transform = ViewportTransform::new(
                zoom,
                ImagePoint::new(
                    (width - self.image_size.0 as f32 * zoom) / 2.0,
                    (height - self.image_size.1 as f32 * zoom) / 2.0,
                ),
            );
        }
        let mut bar = div()
            .id("editor-toolbar")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .items_center()
            .gap(px(2.0))
            .h(px(BAR))
            .w_full()
            .bg(cx.theme().title_bar)
            .text_color(cx.theme().foreground);
        bar = bar
            .child(
                Button::new("app-menu")
                    .ghost()
                    .icon(tool_icon("scissors"))
                    .tooltip("Sniplet · capture and settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel = if this.panel == Some(Panel::Menu) {
                            None
                        } else {
                            Some(Panel::Menu)
                        };
                        cx.notify();
                    })),
            )
            .child(self.toolbar_button(
                "copy",
                "copy",
                "Copy image · Ctrl/Cmd C",
                Command::Copy,
                cx,
            ))
            .child(self.toolbar_button(
                "save",
                "save",
                "Save image · Ctrl/Cmd S",
                Command::Save,
                cx,
            ))
            .child(self.toolbar_button("pin", "pin", "Pin image on screen", Command::Pin, cx))
            .child(self.render_image_drag(cx))
            .child(div().w(px(1.0)).h(px(24.0)).mx_2().bg(cx.theme().border));
        let primary = [
            Tool::Select,
            Tool::Arrow,
            Tool::Text,
            Tool::Ruler,
            Tool::Rectangle,
            Tool::Freehand,
            Tool::Zoom,
        ];
        let visible = (((width - 740.0) / 36.0).floor() as usize).clamp(3, 12);
        let tools: Vec<_> = primary
            .into_iter()
            .chain(Tool::ALL.into_iter().filter(|tool| !primary.contains(tool)))
            .take(visible)
            .collect();
        for tool in tools {
            if tool == Tool::Freehand {
                bar = bar.child(
                    Button::new("backdrop")
                        .ghost()
                        .icon(tool_icon("layers"))
                        .tooltip("Backdrop")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.panel = Some(Panel::Backdrop);
                            cx.notify();
                        })),
                );
            }
            bar = bar.child(
                Button::new(("tool", tool as usize))
                    .ghost()
                    .icon(tool_icon(tool.icon()))
                    .w(px(34.0))
                    .h(px(36.0))
                    .selected(self.tool == tool)
                    .tooltip(tool.shortcut().map_or_else(
                        || tool.label().to_owned(),
                        |key| format!("{} · {key}", tool.label()),
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_tool(tool, cx))),
            );
        }
        bar = bar
            .child(
                Button::new("more-tools")
                    .ghost()
                    .icon(tool_icon("ellipsis"))
                    .tooltip("More tools")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel = Some(Panel::Tools);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, w, _| w.start_window_move()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .size(px(18.0))
                            .rounded_full()
                            .bg(color_ui(self.sampled))
                            .border_1()
                            .border_color(cx.theme().border),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(self.sampled.format(ColorFormat::HexRgb))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Tab to copy"),
                            ),
                    ),
            )
            .child(self.measure_units(cx))
            .child(
                Button::new("zoom-fit")
                    .ghost()
                    .label(format!("{:.0}%", self.transform.zoom() * 100.0))
                    .tooltip("Fit image · Ctrl/Cmd 1")
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::Fit, w, cx))),
            );
        let mut canvas = div()
            .id("editor-canvas")
            .relative()
            .flex_1()
            .overflow_hidden()
            .bg(cx.theme().muted)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_scroll_wheel(cx.listener(Self::scroll));
        if let Some(image) = &self.image {
            let pan = self.transform.pan();
            let zoom = self.transform.zoom();
            canvas = canvas.child(
                img(image.clone())
                    .absolute()
                    .left(px(pan.x))
                    .top(px(pan.y))
                    .w(px(self.image_size.0 as f32 * zoom))
                    .h(px(self.image_size.1 as f32 * zoom)),
            );
            if let Some((bounds, image)) = &self.measure_overlay {
                let origin = self.canvas_point(ImagePoint::new(bounds.x, bounds.y));
                let preview = div()
                    .id("measure-preview")
                    .absolute()
                    .left(px(origin.x))
                    .top(px(origin.y))
                    .w(px(bounds.width * zoom))
                    .h(px(bounds.height * zoom))
                    .child(img(image.clone()).size_full());
                #[cfg(feature = "ui-tests")]
                let preview = {
                    use gpui_kit::test::TestSupportExt;
                    preview.test_support()
                };
                canvas = canvas.child(preview);
            }
            let selected = self
                .document
                .as_ref()
                .and_then(|doc| doc.selected().and_then(|id| doc.annotation(id)));
            let arrow_points = selected.and_then(|annotation| annotation.kind.arrow_points());
            let magnifier_circles =
                selected.and_then(|annotation| annotation.kind.magnifier_circles());
            let selection = self.selection.or_else(|| selected.map(|a| a.kind.bounds()));
            if self.selection.is_none()
                && let Some(points) = arrow_points
            {
                for (index, point) in points.into_iter().enumerate() {
                    let point = self.canvas_point(point);
                    let handle = div()
                        .id(("arrow-handle", index))
                        .absolute()
                        .left(px(point.x - 4.0))
                        .top(px(point.y - 4.0))
                        .size(px(8.0))
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(0x777777))
                        .rounded(px(2.0))
                        .cursor_crosshair();
                    #[cfg(feature = "ui-tests")]
                    let handle = {
                        use gpui_kit::test::TestSupportExt;
                        handle.test_support()
                    };
                    canvas = canvas.child(handle);
                }
            } else if self.selection.is_none()
                && let Some(circles) = magnifier_circles
            {
                for (index, circle) in circles.into_iter().enumerate() {
                    for (control, point) in [
                        ImagePoint::new(
                            circle.x + circle.width * 0.5,
                            circle.y + circle.height * 0.5,
                        ),
                        ImagePoint::new(circle.x + circle.width, circle.y + circle.height * 0.5),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let point = self.canvas_point(point);
                        let handle = div()
                            .id(("magnifier-handle", index * 2 + control))
                            .absolute()
                            .left(px(point.x - 4.0))
                            .top(px(point.y - 4.0))
                            .size(px(8.0))
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(rgb(0x777777))
                            .rounded(px(2.0))
                            .cursor_crosshair();
                        #[cfg(feature = "ui-tests")]
                        let handle = {
                            use gpui_kit::test::TestSupportExt;
                            handle.test_support()
                        };
                        canvas = canvas.child(handle);
                    }
                }
            } else if let Some(rect) = selection {
                let p = self.canvas_point(ImagePoint::new(rect.x, rect.y));
                let w = rect.width * zoom;
                let h = rect.height * zoom;
                let mut outline = div()
                    .absolute()
                    .left(px(p.x))
                    .top(px(p.y))
                    .w(px(w.max(1.0)))
                    .h(px(h.max(1.0)))
                    .border_1()
                    .border_color(rgb(0x007aff));
                for (x, y) in [
                    (0.0, 0.0),
                    (w / 2.0, 0.0),
                    (w, 0.0),
                    (0.0, h / 2.0),
                    (w, h / 2.0),
                    (0.0, h),
                    (w / 2.0, h),
                    (w, h),
                ] {
                    outline = outline.child(
                        div()
                            .absolute()
                            .left(px(x - 3.0))
                            .top(px(y - 3.0))
                            .size(px(6.0))
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(rgb(0x777777))
                            .rounded_sm(),
                    );
                }
                canvas = canvas.child(outline).child(
                    div()
                        .absolute()
                        .left(px(p.x))
                        .top(px((p.y + h + 10.0).min(height - 30.0)))
                        .bg(cx.theme().popover.opacity(0.95))
                        .text_color(cx.theme().popover_foreground)
                        .rounded_md()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .child(format!(
                            "{:.0} × {:.0} px{}",
                            rect.width,
                            rect.height,
                            if self.selection.is_some() {
                                " · Enter to crop"
                            } else {
                                ""
                            }
                        )),
                );
            }
            if self.tool != Tool::Select
                || self
                    .document
                    .as_ref()
                    .is_some_and(|doc| doc.selected().is_some())
            {
                canvas = canvas.child(self.properties(cx));
            }
        } else {
            canvas = canvas.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        div()
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child("Sniplet"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Capture. Annotate. Copy."),
                    )
                    .child(
                        Button::new("welcome-capture")
                            .primary()
                            .icon(tool_icon("scan"))
                            .label("Capture an area")
                            .on_click(
                                cx.listener(|this, _, w, cx| this.command(Command::Area, w, cx)),
                            ),
                    )
                    .child(
                        Button::new("welcome-open")
                            .ghost()
                            .label("Open an image…")
                            .on_click(
                                cx.listener(|this, _, w, cx| this.command(Command::Open, w, cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Ctrl/Cmd + Shift + 2 · available anywhere"),
                    ),
            );
        }
        canvas = canvas.child(
            div()
                .absolute()
                .left(px(12.0))
                .bottom(px(12.0))
                .max_w(px(width - 24.0))
                .px_3()
                .py_1()
                .rounded_md()
                .bg(cx.theme().popover.opacity(0.95))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(self.status.clone()),
        );
        canvas = canvas.child(self.render_panel(window, cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family(if cfg!(target_os = "windows") {
                "Segoe UI"
            } else if cfg!(target_os = "macos") {
                ".AppleSystemUIFont"
            } else {
                "Noto Sans"
            })
            .text_sm()
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .key_context("SnipletEditor")
            .on_key_down(cx.listener(Self::key))
            .on_key_up(cx.listener(Self::key_up))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.update_measure(this.cursor, event.modifiers.shift);
                cx.notify();
            }))
            .child(
                TitleBar::new()
                    .h(px(BAR))
                    .on_close_window(|_, window, cx| {
                        if crate::runtime::has_tray(cx) {
                            crate::runtime::hide_editor(window, cx);
                        } else {
                            cx.quit();
                        }
                    })
                    .child(bar),
            )
            .child(canvas)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if let Some(path) = paths.paths().first() {
                    match image::open(path) {
                        Ok(image) => {
                            this.load(Document::new(image.to_rgba8()), "Opened dropped image", cx)
                        }
                        Err(error) => {
                            this.status = error.to_string();
                            cx.notify();
                        }
                    }
                }
            }))
    }
}

pub fn tool_icon(name: &str) -> Icon {
    Icon::default()
        .path(format!("icons/{name}.svg"))
        .size(px(19.0))
}
fn handles(rect: ImageRect) -> [(f32, f32); 8] {
    let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
    [
        (x, y),
        (x + w / 2.0, y),
        (x + w, y),
        (x, y + h / 2.0),
        (x + w, y + h / 2.0),
        (x, y + h),
        (x + w / 2.0, y + h),
        (x + w, y + h),
    ]
}
fn handle_at(rect: ImageRect, point: ImagePoint, tolerance: f32) -> Option<usize> {
    handles(rect)
        .iter()
        .position(|(x, y)| (point.x - x).abs() <= tolerance && (point.y - y).abs() <= tolerance)
}
fn resize_from_handle(rect: ImageRect, handle: usize, p: ImagePoint, constrain: bool) -> ImageRect {
    let (mut l, mut t, mut r, mut b) = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
    if matches!(handle, 0 | 3 | 5) {
        l = p.x.min(r - 1.0);
    }
    if matches!(handle, 2 | 4 | 7) {
        r = p.x.max(l + 1.0);
    }
    if matches!(handle, 0..=2) {
        t = p.y.min(b - 1.0);
    }
    if matches!(handle, 5..=7) {
        b = p.y.max(t + 1.0);
    }
    if constrain && matches!(handle, 0 | 2 | 5 | 7) {
        let h = (r - l) * rect.height / rect.width.max(1.0);
        if matches!(handle, 0 | 2) {
            t = b - h;
        } else {
            b = t + h;
        }
    }
    ImageRect::new(l, t, r - l, b - t)
}
fn hex_color(rgb: u32) -> Color {
    Color::new((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
}
fn color_ui(c: Color) -> Hsla {
    rgba(((c.r as u32) << 24) | ((c.g as u32) << 16) | ((c.b as u32) << 8) | c.a as u32).into()
}
