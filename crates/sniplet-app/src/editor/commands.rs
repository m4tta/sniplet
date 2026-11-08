//! Menu, toolbar and shortcut commands: files, the clipboard, captures and the camera.
use std::path::PathBuf;

use rune::{FileDialogOptions, Tx};
use sniplet_core::{
    AnnotationKind, Backdrop, ColorFormat, Document, Point as ImagePoint, ViewportTransform,
};
use sniplet_platform::Clipboard;

use super::{Editor, Panel, render_options};
use crate::{
    capture,
    clipboard_jobs::{self, Recognized, Source},
    quick_menu::Action,
    tools::Tool,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    #[cfg(target_os = "linux")]
    GitHub,
    Quit,
    Ocr,
    Qr,
    Upload,
    Fit,
    ActualSize,
    ZoomSelection,
    ZoomIn,
    ZoomOut,
}

struct ImageInsertion {
    png: Vec<u8>,
    dimensions: (u32, u32),
    origin: ImagePoint,
    size: Option<sniplet_core::ImageSize>,
    label: String,
    append: bool,
}

impl Editor {
    pub fn command(&mut self, command: Command, tx: &mut Tx) {
        if self.updates.installing() {
            return;
        }
        self.finish_size_edit();
        self.panel = None;
        self.clear_measure();
        match command {
            Command::Open => self.open(tx),
            Command::Save => self.save(false, tx),
            Command::SaveProject => self.save(true, tx),
            Command::Copy => self.copy_image(),
            Command::Paste | Command::LoadClipboard => {
                if matches!(command, Command::LoadClipboard) {
                    crate::shell::show_editor();
                }
                self.load_revision += 1;
                let revision = self.load_revision;
                self.inbox.spawn(
                    || {
                        Clipboard::new()
                            .and_then(|mut clipboard| clipboard.image())
                            .map(Document::new)
                    },
                    move |this, result, _| {
                        if this.load_revision != revision {
                            return;
                        }
                        match result {
                            Ok(document)
                                if matches!(command, Command::LoadClipboard)
                                    || this.document.is_none() =>
                            {
                                this.load(document, "Loaded image from clipboard");
                            }
                            Ok(document) => {
                                this.add_image(document.original().clone(), "Pasted image", false)
                            }
                            Err(error) => this.status = error.to_string(),
                        }
                    },
                );
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
                self.refresh();
            }
            Command::Crop => {
                if let (Some(doc), Some(rect)) = (&mut self.document, self.selection.take()) {
                    doc.set_crop(rect);
                    self.fit = true;
                }
                self.refresh();
            }
            Command::AutoAdjust => {
                if let (Some(doc), Some(rect)) = (&self.document, self.selection) {
                    self.selection = doc.auto_adjust_selection(rect);
                }
            }
            Command::ResetCrop => {
                if let Some(doc) = &mut self.document {
                    doc.clear_crop();
                    self.fit = true;
                }
                self.refresh();
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
                self.refresh();
            }
            Command::Pin => {
                if let Ok(document) = self.export_document() {
                    self.inbox.spawn(
                        move || document.render(&render_options()),
                        |editor, image, _| {
                            if let Ok(image) = image {
                                capture::pin(image, &editor.inbox);
                            }
                        },
                    );
                }
            }
            Command::Window => {
                if let Err(error) = crate::window_picker::open(self) {
                    self.status = error;
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
                let request = capture::CaptureRequest {
                    command,
                    monitor: self.monitor_index,
                    region: self.scroll_region,
                    last_capture: self.last_capture.clone(),
                };
                if let Err(error) = capture::start(request, self) {
                    self.status = error;
                }
            }
            Command::ActiveWindow => {
                if let Err(error) = capture::active_window(self) {
                    self.status = error;
                }
            }
            Command::ShowEditor => crate::shell::show_editor(),
            Command::Settings => self.open_settings(crate::settings_window::Page::General),
            #[cfg(target_os = "linux")]
            Command::GitHub => {
                let _ = tx.open_url("https://github.com/m4tta/sniplet");
            }
            Command::Quit => self.quit(tx),
            Command::Upload => self.upload(),
            Command::Ocr => self.recognize(false),
            Command::Qr => self.recognize(true),
            Command::Fit => self.fit = true,
            Command::ZoomSelection => {
                if let Some(rect) = self.selection.or_else(|| {
                    self.document.as_ref().and_then(|d| {
                        d.selected()
                            .and_then(|id| d.annotation(id))
                            .map(|a| a.kind.bounds())
                    })
                }) {
                    let (w, h) = self.canvas;
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
                }
            }
            Command::ActualSize | Command::ZoomIn | Command::ZoomOut => {
                self.unfit();
                let center = ImagePoint::new(self.canvas.0 / 2.0, self.canvas.1 / 2.0);
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
            }
        }
    }

    /// A quick menu item was chosen.
    pub(crate) fn choose(&mut self, action: Action, tx: &mut Tx) {
        match action {
            Action::Run(command) => self.command(command, tx),
            Action::Help => self.panel = Some(Panel::Help),
        }
    }

    fn export_document(&self) -> anyhow::Result<Document> {
        let mut document = self
            .document
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Capture or open an image first"))?;
        if let Some(rect) = self.selection {
            document.set_crop(rect);
            document.set_backdrop(Backdrop::default());
        }
        Ok(document)
    }

    pub fn export_pixels(&self) -> anyhow::Result<image::RgbaImage> {
        Ok(self.export_document()?.render(&render_options())?)
    }

    /// Export to a temporary PNG for a drag into another app, so the file holds the same
    /// pixels as Save and Copy.
    pub(crate) fn export_drag_file(&self) -> anyhow::Result<PathBuf> {
        let image = self.export_pixels()?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("Sniplet-{timestamp}.png"));
        image.save(&path)?;
        Ok(path)
    }

    fn copy_image(&mut self) {
        let document = match self.export_document() {
            Ok(document) => document,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let revision = self.preview_revision;
        let hide =
            self.selection.is_none() && self.settings.hide_after_export && crate::shell::has_tray();
        self.inbox.spawn(
            clipboard_jobs::copy(Source::Rendered(document)),
            move |this, result, _| {
                if this.preview_revision != revision || matches!(result, Ok(None)) {
                    return;
                }
                let copied = result.is_ok();
                this.result(result.map(|_| ()), "Image copied");
                if copied && hide {
                    crate::shell::hide_editor();
                }
            },
        );
    }

    pub(super) fn copy_color(&mut self) {
        let text = self.sampled.format(ColorFormat::HexRgb);
        let revision = self.preview_revision;
        let message = format!("Copied {text}");
        self.inbox.spawn(
            clipboard_jobs::copy(Source::Text(text)),
            move |this, result, _| {
                if this.preview_revision != revision || matches!(result, Ok(None)) {
                    return;
                }
                this.result(result.map(|_| ()), &message);
            },
        );
    }

    /// Copy the recognized text dialog's contents.
    pub(super) fn copy_recognized_text(&mut self) {
        let Some(edit) = &self.recognized_text else {
            return;
        };
        let text = edit.text.clone();
        let count = text.chars().count();
        let recognition = self.recognition_revision;
        self.inbox.spawn(
            clipboard_jobs::copy(Source::Text(text)),
            move |this, result, _| {
                if this.recognized_text.is_none()
                    || this.recognition_revision != recognition
                    || matches!(result, Ok(None))
                {
                    return;
                }
                this.result(
                    result.map(|_| ()),
                    &format!("Text copied · {count} characters"),
                );
            },
        );
    }

    fn result(&mut self, result: anyhow::Result<()>, message: &str) {
        self.status = result.map_or_else(|e| e.to_string(), |_| message.to_owned());
    }

    fn open(&mut self, tx: &mut Tx) {
        self.load_revision += 1;
        let revision = self.load_revision;
        let options = FileDialogOptions {
            title: "Open image or Sniplet project".into(),
            ..Default::default()
        };
        tx.open_file(options, move |this: &mut Editor, result, _| match result {
            Ok(Some(path)) if this.load_revision == revision => this.load_path(path),
            Ok(_) => {}
            Err(error) => this.status = format!("Could not open a file: {error}"),
        });
    }

    pub(crate) fn load_path(&mut self, path: PathBuf) {
        self.load_revision += 1;
        let revision = self.load_revision;
        let label = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        self.inbox.spawn(
            move || {
                if sniplet_core::Project::is_project_path(&path) {
                    sniplet_core::Project::load(path).and_then(|project| project.open_document())
                } else {
                    image::open(path)
                        .map(|image| Document::new(image.to_rgba8()))
                        .map_err(sniplet_core::SnipletError::from)
                }
            },
            move |this, result, _| {
                if this.load_revision != revision {
                    return;
                }
                match result {
                    Ok(doc) => this.load(doc, &label),
                    Err(error) => this.status = error.to_string(),
                }
            },
        );
    }

    fn save(&mut self, project: bool, tx: &mut Tx) {
        let Some(mut document) = self.document.clone() else {
            return;
        };
        if !project && let Some(rect) = self.selection {
            document.set_crop(rect);
            document.set_backdrop(Backdrop::default());
        }
        let format = self.settings.format;
        let scale = if self.settings.downscale_on_save {
            document.source_scale_factor().unwrap_or(1.0)
        } else {
            1.0
        };
        let hide = !project
            && self.selection.is_none()
            && self.settings.hide_after_export
            && crate::shell::has_tray();
        let directory = self
            .settings
            .screenshot_directory
            .clone()
            .unwrap_or_else(|| {
                sniplet_platform::default_export_directory()
                    .unwrap_or_else(|_| std::env::temp_dir())
            });
        let suggested = format!(
            "Sniplet.{}",
            if project {
                "sniplet"
            } else {
                format.extension()
            }
        );
        let revision = self.preview_revision;
        let options = FileDialogOptions {
            title: if project {
                "Save project"
            } else {
                "Save image"
            }
            .into(),
            directory: Some(directory),
            file_name: Some(suggested),
            ..Default::default()
        };
        tx.save_file(options, move |this: &mut Editor, result, _| {
            let path = match result {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(error) => {
                    this.status = format!("Could not save: {error}");
                    return;
                }
            };
            let destination = path.clone();
            this.inbox.spawn(
                move || write_export(document, &destination, project, scale),
                move |this, result, _| {
                    if this.preview_revision != revision {
                        return;
                    }
                    let saved = result.is_ok();
                    if saved {
                        this.last_saved = Some(path.clone());
                    }
                    this.result(result, &format!("Saved {}", path.display()));
                    if saved && hide {
                        crate::shell::hide_editor();
                    }
                },
            );
        });
    }

    pub(super) fn recognize(&mut self, qr_only: bool) {
        let document = match self.export_document() {
            Ok(document) => document,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.status = "Recognizing text…".into();
        // A new document or a newer recognition makes the result stale. Edits and clicks
        // during recognition must not drop it.
        self.recognition_revision += 1;
        let revision = (self.load_revision, self.recognition_revision);
        self.inbox.spawn(
            clipboard_jobs::recognize(document, qr_only),
            move |this, result, _| {
                if (this.load_revision, this.recognition_revision) != revision {
                    return;
                }
                match result {
                    Ok(Recognized { text, copied }) => {
                        let count = text.chars().count();
                        this.status = match copied {
                            Ok(true) if qr_only => "QR contents copied".into(),
                            Ok(true) => format!("Text copied · {count} characters"),
                            Ok(false) => {
                                "Text recognized · a newer copy stays on the clipboard".into()
                            }
                            Err(error) => format!("Text recognized but not copied: {error}"),
                        };
                        let mut edit = rune::TextEdit::new(text);
                        edit.focused = true;
                        this.panel = None;
                        this.recognized_text = Some(edit);
                    }
                    Err(error) => this.status = error.to_string(),
                }
            },
        );
    }

    fn upload(&mut self) {
        let Some(config) = self.settings.cloud_upload.clone() else {
            self.open_settings(crate::settings_window::Page::Share);
            return;
        };
        let document = match self.export_document() {
            Ok(document) => document,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.status = "Uploading image…".into();
        let key = format!(
            "Sniplet-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        self.inbox.spawn(
            move || {
                let image = document.render(&render_options())?;
                Ok::<_, anyhow::Error>(sniplet_platform::upload_image(&image, &key, &config)?)
            },
            |this, result, _| {
                this.status = match result {
                    Ok(upload) => match Clipboard::new().and_then(|mut c| c.set_text(upload.url)) {
                        Ok(_) => "Uploaded · image link copied".into(),
                        Err(error) => format!("Uploaded, but could not copy the link: {error}"),
                    },
                    Err(error) => format!("Upload failed: {error}"),
                };
            },
        );
    }

    pub fn add_image(&mut self, image: image::RgbaImage, label: &str, append: bool) {
        let Some(document) = &self.document else {
            self.load(Document::new(image), label);
            return;
        };
        let origin = if append {
            ImagePoint::new(document.width() as f32, 0.0)
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
        let dimensions = image.dimensions();
        let pixels = u64::from(dimensions.0) * u64::from(dimensions.1);
        let label = label.to_owned();
        let encode = move || {
            let mut png = Vec::new();
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut png).write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            Ok::<_, image::ImageError>(ImageInsertion {
                png,
                dimensions,
                origin,
                size: inserted_size,
                label,
                append,
            })
        };
        if pixels <= super::INLINE_PIXELS {
            self.insert_image(encode());
        } else {
            let revision = self.load_revision;
            self.inbox.spawn(encode, move |this, result, _| {
                if this.load_revision == revision {
                    this.insert_image(result);
                }
            });
        }
    }

    fn insert_image(&mut self, result: Result<ImageInsertion, image::ImageError>) {
        let insertion = match result {
            Ok(insertion) => insertion,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        if let Err(error) = doc.begin_group() {
            self.status = error.to_string();
            return;
        }
        let (width, height) = insertion
            .size
            .map(|size| (size.width, size.height))
            .unwrap_or(insertion.dimensions);
        doc.expand_canvas_to(
            (insertion.origin.x.ceil() as u32).saturating_add(width),
            (insertion.origin.y.ceil() as u32).saturating_add(height),
        );
        if insertion.append {
            doc.clear_crop();
        }
        let id = doc.add_annotation(
            AnnotationKind::Image {
                origin: insertion.origin,
                png_bytes: insertion.png,
                size: insertion.size,
            },
            self.style,
        );
        let _ = doc.select(Some(id));
        let _ = doc.end_group();
        self.selection = None;
        self.tool = Tool::Select;
        self.status = insertion.label;
        self.fit = true;
        self.refresh();
    }

    pub(crate) fn open_settings(&mut self, page: crate::settings_window::Page) {
        self.panel = None;
        if let Err(error) = crate::settings_window::open(self, page) {
            self.status = format!("Could not open settings: {error}");
        }
    }
}

/// Write an image export, or a project with its source image beside it.
fn write_export(
    document: Document,
    path: &std::path::Path,
    project: bool,
    scale: f32,
) -> anyhow::Result<()> {
    if project {
        let source = path.with_extension("source.png");
        document.original().save(&source)?;
        document
            .to_project(PathBuf::from(source.file_name().unwrap()))
            .save(path)?;
    } else {
        let image = sniplet_platform::image_at_1x(document.render(&render_options())?, scale);
        let extension = path
            .extension()
            .map(|x| x.to_string_lossy().to_ascii_lowercase());
        let chosen = match extension.as_deref() {
            Some("jpg" | "jpeg") => sniplet_platform::ExportFormat::Jpeg,
            Some("webp") => sniplet_platform::ExportFormat::Webp,
            _ => sniplet_platform::ExportFormat::Png,
        };
        sniplet_platform::export_image(&image, path, chosen)?;
    }
    Ok(())
}
