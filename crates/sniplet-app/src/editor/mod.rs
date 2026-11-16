//! The editor window: one document, its tools, and the rendered preview.
//!
//! `mod.rs` holds the state and the model operations. `input.rs` turns pointer and key input
//! into those operations, `commands.rs` runs menu and shortcut commands, and `view.rs` and
//! `panels.rs` draw it.
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

use rune::{Pixels, TextEdit};
use sniplet_core::{
    Annotation, AnnotationId, AnnotationKind, AnnotationStyle, ArrowVariant, Color, Document,
    ImageRect, Measurement, MeasurementAxis, Point as ImagePoint, RenderOptions, ViewportTransform,
};
use sniplet_platform::{Settings, SettingsStore};

use crate::{
    drag_out::{DragFile, DragFiles},
    tools::{HideStyle, Size, Sizes, Tool},
    ui::Inbox,
};

mod commands;
mod input;
mod panels;
#[cfg(test)]
mod tests;
mod view;

pub(crate) use commands::{Command, Export};
pub(crate) use input::Keystroke;
pub(crate) use view::{on_key, view};

pub const FONT: &[u8] = sniplet_core::FONT;
/// The toolbar's height, which is also the window's title bar.
pub(crate) const BAR: f32 = 54.0;
/// Canvas height kept clear below the image at fit zoom, for the tool dock.
const DOCK_SPACE: f32 = 80.0;
/// Gap between the dock and the bottom of the canvas.
const DOCK_BOTTOM: f32 = 16.0;
/// The dock's height: a 36 pt button, 5 pt padding, and a 1 pt border on each side.
const DOCK_HEIGHT: f32 = 48.0;
/// Rune draws images up to this many pixels on a side.
const MAX_TEXTURE: u32 = 8192;
/// Previews and measurements up to this many pixels render on the UI thread.
const INLINE_PIXELS: u64 = 262_144;

pub fn render_options() -> RenderOptions<'static> {
    RenderOptions {
        font_bytes: Some(FONT),
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Menu,
    Text,
    Backdrop,
    Help,
}

#[derive(Clone, Copy)]
enum EditHandle {
    Bounds(usize, ImageRect),
    Arrow(usize),
    Magnifier(sniplet_core::MagnifierPart, bool),
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

/// A size slider drag in progress. Escape restores `before`.
struct SizeEdit {
    size: Size,
    before: f32,
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

/// A finished preview render: the full image for color sampling, and what the canvas shows.
struct Preview {
    size: (u32, u32),
    origin: ImagePoint,
    image: Arc<image::RgbaImage>,
    display: (u32, u32, Vec<u8>),
}

pub struct Editor {
    pub document: Option<Document>,
    pub tool: Tool,
    pub selection: Option<ImageRect>,
    pub status: String,
    /// Set when Sniplet started without the macOS Screen Recording permission.
    pub(crate) screen_access: Option<crate::screen_access::ScreenAccess>,
    /// An upload is running. A second one waits for it, see `upload`.
    pub(crate) uploading: bool,
    pub settings: Settings,
    pub(crate) hotkey_error: Option<String>,
    /// True while Settings records a new shortcut, so the OS does not take the keypress.
    pub(crate) hotkeys_paused: bool,
    pub panel: Option<Panel>,
    #[cfg(test)]
    pub settings_path: Option<PathBuf>,
    /// Results from workers and messages from other windows.
    pub(crate) inbox: Inbox<Editor>,
    text_input: TextEdit,
    pub(crate) recognized_text: Option<TextEdit>,
    text_origin: ImagePoint,
    text_edit: Option<AnnotationId>,
    style: AnnotationStyle,
    arrow_variant: ArrowVariant,
    hide: HideStyle,
    sizes: Sizes,
    size_edit: Option<SizeEdit>,
    /// Shows the sampled color next to the pointer while Tab or C is held.
    color_peek: bool,
    /// An arrow-key run holds an undo group open, so a held key undoes in one step.
    key_group: bool,
    drag: Option<Drag>,
    draft: Option<Annotation>,
    /// What the canvas draws: the document rendered with the draft.
    pixels: Pixels,
    /// The same render at full resolution, for color sampling.
    preview: Option<Arc<image::RgbaImage>>,
    /// The `preview_revision` that `preview` shows with no draft. When it is the current
    /// one, an export of the whole image uses the preview instead of rendering again.
    export_revision: Option<u64>,
    /// The PNG a drag out of the window hands over. A mutex, as the drag asks with `&Editor`.
    drag_file: Mutex<DragFiles>,
    drag_file_running: bool,
    preview_dirty: bool,
    preview_running: bool,
    preview_revision: u64,
    document_revision: u64,
    pub(crate) load_revision: u64,
    recognition_revision: u64,
    pub(crate) settings_revision: u64,
    pub(crate) settings_save_running: bool,
    pub(crate) settings_save_result: Option<(u64, Option<String>)>,
    #[cfg(test)]
    pub(crate) preview_frames: usize,
    image_size: (u32, u32),
    content_origin: ImagePoint,
    /// The camera while `fit` is off. `transform()` gives the one in use.
    transform: ViewportTransform,
    fit: bool,
    /// The canvas's size in points, below the toolbar.
    canvas: (f32, f32),
    cursor: ImagePoint,
    sampled: Color,
    space: bool,
    quick_zoom: bool,
    measure: Option<MeasureSession>,
    measure_pending: Option<(String, bool)>,
    /// Where the measurement overlay sits in image space, and its pixels.
    measure_overlay: Option<ImageRect>,
    measure_pixels: Pixels,
    measure_tolerance: u8,
    measure_scale: f32,
    default_measure_scale: f32,
    physical_units: bool,
    last_saved: Option<PathBuf>,
    /// A manual scrolling capture's page so far, and its area.
    pub scroll_image: Option<Arc<image::RgbaImage>>,
    pub scroll_region: Option<crate::capture::ScrollRegion>,
    pub last_capture: Option<crate::area_capture::Region>,
    pub(crate) updates: crate::updates::Updates,
}

impl Editor {
    /// `scale` is the primary display's pixels per point, for measurements of images that do
    /// not record their own.
    pub fn new(document: Option<Document>, settings: Settings, scale: f32) -> Self {
        let c = settings.annotation_color;
        let measure_scale = document
            .as_ref()
            .and_then(|doc| doc.source_scale_factor())
            .unwrap_or(scale);
        let mut this = Self {
            document,
            tool: Tool::Select,
            selection: None,
            status: "Ready".into(),
            screen_access: None,
            uploading: false,
            settings,
            hotkey_error: None,
            hotkeys_paused: false,
            panel: None,
            #[cfg(test)]
            settings_path: None,
            inbox: Inbox::default(),
            text_input: TextEdit::new(""),
            recognized_text: None,
            text_origin: ImagePoint::default(),
            text_edit: None,
            style: AnnotationStyle {
                stroke: Color::new(c.red, c.green, c.blue, c.alpha),
                ..Default::default()
            },
            arrow_variant: ArrowVariant::Solid,
            hide: HideStyle::default(),
            sizes: Sizes::default(),
            size_edit: None,
            color_peek: false,
            key_group: false,
            drag: None,
            draft: None,
            pixels: Pixels::new(),
            preview: None,
            export_revision: None,
            drag_file: Mutex::default(),
            drag_file_running: false,
            preview_dirty: false,
            preview_running: false,
            preview_revision: 0,
            document_revision: 0,
            load_revision: 0,
            recognition_revision: 0,
            settings_revision: 0,
            settings_save_running: false,
            settings_save_result: None,
            #[cfg(test)]
            preview_frames: 0,
            image_size: (0, 0),
            content_origin: ImagePoint::default(),
            transform: ViewportTransform::default(),
            fit: true,
            canvas: (1280.0, 850.0 - BAR),
            cursor: ImagePoint::default(),
            sampled: Color::WHITE,
            space: false,
            quick_zoom: false,
            measure: None,
            measure_pending: None,
            measure_overlay: None,
            measure_pixels: Pixels::new(),
            measure_tolerance: 20,
            measure_scale,
            default_measure_scale: scale,
            physical_units: false,
            last_saved: None,
            scroll_image: None,
            scroll_region: None,
            last_capture: None,
            updates: crate::updates::Updates::new(),
        };
        this.refresh();
        this
    }

    pub fn load(&mut self, document: Document, label: &str) {
        if self.updates.installing() {
            return;
        }
        self.load_revision += 1;
        self.document_revision += 1;
        self.finish_size_edit();
        self.end_key_group();
        self.measure_scale = document
            .source_scale_factor()
            .unwrap_or(self.default_measure_scale);
        let unedited = document
            .unedited()
            .filter(|image| image.width().max(image.height()) <= MAX_TEXTURE);
        self.document = Some(document);
        self.selection = None;
        self.drag = None;
        self.draft = None;
        self.clear_measure();
        self.fit = true;
        self.panel = None;
        self.recognized_text = None;
        self.last_saved = None;
        self.status = label.to_owned();
        match unedited {
            // A capture with nothing on it is its own preview. Show it now, rather than the
            // last image until a render finishes; a render still running is for that image.
            Some(image) => {
                self.preview_revision += 1;
                self.preview_dirty = false;
                let preview = Preview {
                    size: image.dimensions(),
                    origin: ImagePoint::default(),
                    display: display_pixels(&image),
                    image,
                };
                self.apply_preview(Ok(preview), Some(self.preview_revision));
            }
            None => self.refresh(),
        }
    }

    pub fn set_measure_scale(&mut self, scale: f32) {
        if let Some(doc) = &mut self.document {
            doc.set_source_scale_factor(scale);
            self.measure_scale = doc
                .source_scale_factor()
                .unwrap_or(self.default_measure_scale);
        }
    }

    /// The document changed: render a new preview. One render runs at a time; input during
    /// it only marks the latest state dirty, and the render that follows picks it up.
    fn refresh(&mut self) {
        if let Some(doc) = &self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id))
        {
            self.style = drawing_style(self.style, annotation);
        }
        self.preview_revision += 1;
        self.preview_dirty = true;
        self.prepare_preview();
    }

    fn prepare_preview(&mut self) {
        if !self.preview_dirty || self.preview_running {
            return;
        }
        self.preview_dirty = false;
        let Some(doc) = self.document.as_ref().map(Document::snapshot) else {
            return;
        };
        let draft = self.draft.clone();
        let export_revision = draft.is_none().then_some(self.preview_revision);
        let document_revision = self.document_revision;
        let padding = doc.backdrop().padding;
        let pixels = (doc.width() as u64 + 2 * padding as u64)
            .saturating_mul(doc.height() as u64 + 2 * padding as u64);
        let prepare = move || {
            let options = RenderOptions {
                extra_annotations: draft.as_slice(),
                ..render_options()
            };
            let image = doc.render(&options)?;
            Ok::<_, sniplet_core::SnipletError>(Preview {
                size: image.dimensions(),
                origin: doc.render_content_origin(&options),
                display: display_pixels(&image),
                image: Arc::new(image),
            })
        };
        // A small preview fits within one frame. Large images use a worker so pointer input
        // and drawing continue during rasterization.
        if pixels <= INLINE_PIXELS {
            self.apply_preview(prepare(), export_revision);
        } else {
            self.preview_running = true;
            self.inbox.spawn(prepare, move |this, result, _| {
                this.preview_running = false;
                // Show completed work during a continuous drag, then render the latest
                // input. A different document must never use it.
                if this.document_revision == document_revision {
                    this.apply_preview(result, export_revision);
                }
                this.prepare_preview();
            });
        }
    }

    /// Shows a finished render. `export_revision` is the revision it shows exactly, when it
    /// has no draft.
    fn apply_preview(
        &mut self,
        result: sniplet_core::Result<Preview>,
        export_revision: Option<u64>,
    ) {
        match result {
            Ok(preview) => {
                self.image_size = preview.size;
                self.content_origin = preview.origin;
                let (width, height, rgba) = preview.display;
                self.pixels.set(width, height, rgba);
                self.preview = Some(preview.image);
                self.export_revision = export_revision;
                #[cfg(test)]
                {
                    self.preview_frames += 1;
                }
                if let Some(color) = self.visible_color(self.cursor) {
                    self.sampled = color;
                }
                self.prepare_drag_file();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    /// The preview, when it shows the document exactly as it is now: rendered with export
    /// options, no draft, and no edit since.
    fn current_preview(&self) -> Option<Arc<image::RgbaImage>> {
        self.preview
            .clone()
            .filter(|_| self.export_revision == Some(self.preview_revision))
    }

    /// Writes the file a drag out hands over on a worker once the preview settles, so a drag
    /// starts at once. Only macOS and Windows drag files out.
    fn prepare_drag_file(&mut self) {
        if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
            return;
        }
        if self.drag_file_running
            || self.drag.is_some()
            || self.size_edit.is_some()
            || self.key_group
        {
            return;
        }
        let Some(image) = self.current_preview() else {
            return;
        };
        let revision = self.preview_revision;
        let slot = self
            .drag_file
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        if slot
            .current
            .as_ref()
            .is_some_and(|file| file.revision == Some(revision))
        {
            return;
        }
        self.drag_file_running = true;
        self.inbox.spawn(
            move || DragFile::write(&image, Some(revision)),
            move |this, result, _| {
                this.drag_file_running = false;
                // A failed write is not tried again until the preview changes.
                let written = result.is_ok();
                // A file for an older preview is dropped, which deletes it. So is one that a
                // drag already wrote for this preview, so the dragged file stays.
                let slot = this
                    .drag_file
                    .get_mut()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Ok(file) = result
                    && this.preview_revision == revision
                    && slot
                        .current
                        .as_ref()
                        .is_none_or(|old| old.revision != Some(revision))
                {
                    slot.replace(file);
                }
                if written {
                    this.prepare_drag_file();
                }
            },
        );
    }

    /// Ends the undo group of an arrow-key run, when one is open.
    fn end_key_group(&mut self) {
        if std::mem::take(&mut self.key_group) {
            if let Some(doc) = &mut self.document {
                let _ = doc.end_group();
            }
            self.prepare_drag_file();
        }
    }

    pub fn select_tool(&mut self, tool: Tool) {
        self.finish_size_edit();
        self.clear_measure();
        self.tool = tool;
        self.selection = None;
        self.panel = None;
        self.status = if tool == Tool::Ocr {
            "OCR · drag over text or a QR code to copy it".into()
        } else {
            tool.shortcut().map_or_else(
                || tool.label().to_owned(),
                |key| format!("{} · {key}", tool.label()),
            )
        };
        if let Some(doc) = &mut self.document {
            let _ = doc.select(None);
        }
    }

    /// The camera in use: the fitted one while `fit` is on.
    fn transform(&self) -> ViewportTransform {
        if !self.fit || self.image_size.0 == 0 {
            return self.transform;
        }
        let (width, height) = self.canvas;
        let (image_width, image_height) = (self.image_size.0 as f32, self.image_size.1 as f32);
        // Fit between a 24 pt top margin and the dock. Center the image when it has room,
        // and otherwise keep its bottom clear of the dock.
        let zoom = ((width - 48.0) / image_width)
            .min((height - 24.0 - DOCK_SPACE) / image_height)
            .clamp(0.01, 1.0);
        let top =
            ((height - image_height * zoom) / 2.0).min(height - DOCK_SPACE - image_height * zoom);
        ViewportTransform::new(
            zoom,
            ImagePoint::new((width - image_width * zoom) / 2.0, top),
        )
    }

    /// Leave fit mode, keeping the current camera, before moving it by hand.
    fn unfit(&mut self) {
        self.transform = self.transform();
        self.fit = false;
    }

    /// A canvas point in image space.
    fn image_point(&self, x: f32, y: f32) -> ImagePoint {
        let p = self.transform().screen_to_image(ImagePoint::new(x, y));
        ImagePoint::new(p.x - self.content_origin.x, p.y - self.content_origin.y)
    }

    /// An image point in canvas space.
    fn canvas_point(&self, p: ImagePoint) -> ImagePoint {
        self.transform().image_to_screen(ImagePoint::new(
            p.x + self.content_origin.x,
            p.y + self.content_origin.y,
        ))
    }

    fn clear_measure(&mut self) {
        self.measure = None;
        self.measure_pending = None;
        self.measure_overlay = None;
    }

    fn visible_color(&self, point: ImagePoint) -> Option<Color> {
        let image = self.preview.as_ref()?;
        let x = (point.x + self.content_origin.x).floor();
        let y = (point.y + self.content_origin.y).floor();
        if x < 0.0 || y < 0.0 || x >= image.width() as f32 || y >= image.height() as f32 {
            return None;
        }
        let pixel = image.get_pixel(x as u32, y as u32).0;
        Some(Color::new(pixel[0], pixel[1], pixel[2], pixel[3]))
    }

    #[cfg(test)]
    pub(crate) fn sampled_color(&self) -> Color {
        self.sampled
    }

    #[cfg(test)]
    pub(crate) fn preview_image(&self) -> Option<Arc<image::RgbaImage>> {
        self.preview.clone()
    }

    fn selected_annotation(&self) -> Option<&Annotation> {
        let doc = self.document.as_ref()?;
        doc.selected().and_then(|id| doc.annotation(id))
    }

    /// The size the slider edits: the selected annotation's, else the current tool's.
    fn size_target(&self) -> Option<Size> {
        match self.selected_annotation() {
            Some(annotation) => Size::for_kind(&annotation.kind),
            None => Size::for_tool(self.tool, self.hide),
        }
    }

    fn size_value(&self, size: Size) -> f32 {
        self.selected_annotation()
            .filter(|annotation| Size::for_kind(&annotation.kind) == Some(size))
            .map_or(self.sizes.get(size), |annotation| {
                size.get(&annotation.kind, &annotation.style)
            })
    }

    /// The size slider moved to `fraction` of its range. One drag is one undo step.
    fn slide_size(&mut self, fraction: f32, phase: rune::Phase) {
        let Some(size) = self.size_target() else {
            return;
        };
        if self.size_edit.is_none() {
            self.end_key_group();
            let grouped = self.selected_annotation().is_some()
                && self
                    .document
                    .as_mut()
                    .is_some_and(|doc| doc.begin_group().is_ok());
            self.size_edit = Some(SizeEdit {
                size,
                before: self.sizes.get(size),
                grouped,
            });
        }
        self.set_size(size, size.value_at(fraction));
        if phase == rune::Phase::Up {
            self.finish_size_edit();
        }
    }

    /// Escape ended a size drag: put the size back, and the annotation with it.
    fn cancel_size_edit(&mut self) {
        if let Some(edit) = self.size_edit.take() {
            if edit.grouped
                && let Some(doc) = &mut self.document
            {
                let _ = doc.cancel_group();
            }
            self.sizes.set(edit.size, edit.before);
            self.refresh();
        }
    }

    fn finish_size_edit(&mut self) {
        let Some(edit) = self.size_edit.take() else {
            return;
        };
        if edit.grouped
            && let Some(doc) = &mut self.document
        {
            let _ = doc.end_group();
        }
        self.prepare_drag_file();
    }

    /// Sets the size for new annotations, and for the selected one when it has this size.
    fn set_size(&mut self, size: Size, value: f32) {
        self.sizes.set(size, value);
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
            && Size::for_kind(&annotation.kind) == Some(size)
        {
            let (mut kind, mut style) = (annotation.kind, annotation.style);
            size.set(&mut kind, &mut style, value);
            let _ = doc.update_annotation(annotation.id, kind, style);
        }
        self.refresh();
    }

    fn change_arrow_variant(&mut self, variant: ArrowVariant) {
        self.finish_size_edit();
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
        self.refresh();
    }

    fn change_style(&mut self, color: Option<Color>, fill: Option<bool>) {
        if let Some(color) = color {
            self.style.stroke = color;
            self.style.fill = self.style.fill.map(|_| color);
        }
        if let Some(fill) = fill {
            self.style.fill = fill.then_some(self.style.stroke);
        }
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
        {
            let _ = doc.update_annotation(annotation.id, annotation.kind, self.style);
        }
        self.refresh();
    }

    /// B selects Hide. Each press while Hide is active moves to the next style.
    fn cycle_hide(&mut self) {
        let style = if self.tool == Tool::Hide {
            self.hide.next()
        } else {
            self.selected_annotation()
                .and_then(|annotation| HideStyle::of(&annotation.kind))
                .map_or(self.hide, |(style, _)| style)
        };
        self.set_hide(style);
    }

    /// Selects Hide with `style`, and converts a selected hiding annotation to it.
    fn set_hide(&mut self, style: HideStyle) {
        self.finish_size_edit();
        self.tool = Tool::Hide;
        self.hide = style;
        self.selection = None;
        self.panel = None;
        self.status = format!("Hide · {}", style.label());
        if let Some(doc) = &mut self.document
            && let Some(annotation) = doc.selected().and_then(|id| doc.annotation(id)).cloned()
        {
            match HideStyle::of(&annotation.kind) {
                Some((current, rect)) if current != style => {
                    let mut kind = style.kind(rect);
                    let mut annotation_style = annotation.style;
                    self.sizes.apply(&mut kind, &mut annotation_style);
                    let _ = doc.update_annotation(annotation.id, kind, annotation_style);
                }
                Some(_) => {}
                None => {
                    let _ = doc.select(None);
                }
            }
        }
        self.refresh();
    }

    fn adjust_counter_value(&mut self, delta: i32) {
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
        }
        self.refresh();
    }

    /// The image size in the current units, such as "1392 × 722 pt".
    fn image_size_label(&self) -> String {
        let units = if self.physical_units {
            1.0
        } else {
            self.measure_scale
        };
        format!(
            "{:.0} × {:.0} {}",
            self.image_size.0 as f32 / units,
            self.image_size.1 as f32 / units,
            if self.physical_units { "px" } else { "pt" }
        )
    }

    pub(crate) fn settings_store(&self) -> anyhow::Result<SettingsStore> {
        #[cfg(test)]
        if let Some(path) = &self.settings_path {
            return Ok(SettingsStore::at(path));
        }
        Ok(SettingsStore::for_app()?)
    }

    pub(crate) fn persist_settings(&mut self) {
        self.settings_revision += 1;
        self.save_settings();
    }

    fn save_settings(&mut self) {
        if self.settings_save_running {
            return;
        }
        self.settings_save_running = true;
        let revision = self.settings_revision;
        let load_revision = self.load_revision;
        let settings = self.settings.clone();
        let store = self.settings_store();
        self.inbox.spawn(
            move || store.and_then(|store| Ok(store.save(&settings)?)),
            move |this, result, _| {
                this.settings_save_running = false;
                if this.settings_revision != revision {
                    this.save_settings();
                    return;
                }
                let error = result.err().map(|error| error.to_string());
                // The Settings window confirms its own saves. The status shows only failures, so
                // a save never hides the status of the command that caused it, such as an upload.
                if let Some(error) = &error
                    && this.load_revision == load_revision
                {
                    this.status = error.clone();
                }
                this.settings_save_result = Some((revision, error));
            },
        );
    }
}

/// An image as the canvas draws it: unchanged, or scaled down to fit Rune's texture limit.
pub(crate) fn display_pixels(image: &image::RgbaImage) -> (u32, u32, Vec<u8>) {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    if longest <= MAX_TEXTURE {
        return (width, height, image.as_raw().clone());
    }
    let scale = MAX_TEXTURE as f32 / longest as f32;
    let (w, h) = (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    );
    let small = image::imageops::resize(image, w, h, image::imageops::FilterType::Triangle);
    (w, h, small.into_raw())
}

/// The number a new counter shows: one more than the highest counter in `annotations`, so
/// undo, redo and delete keep the numbering. Counters stop at 99.
fn next_counter(annotations: &[Annotation]) -> u32 {
    annotations
        .iter()
        .filter_map(|annotation| match annotation.kind {
            AnnotationKind::Counter { value, .. } => Some(value),
            _ => None,
        })
        .max()
        .map_or(1, |highest| highest.saturating_add(1).min(99))
}

/// The style for new annotations after `annotation` is selected. A spotlight's fill is its
/// dim, not a shape fill, so the current fill stays.
fn drawing_style(current: AnnotationStyle, annotation: &Annotation) -> AnnotationStyle {
    match annotation.kind {
        AnnotationKind::Spotlight { .. } => AnnotationStyle {
            fill: current.fill,
            ..annotation.style
        },
        _ => annotation.style,
    }
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
