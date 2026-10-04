use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::{
    Annotation, AnnotationId, AnnotationKind, AnnotationStyle, Color, ImageRect, Point, Result,
    SnipletError,
};

const PROJECT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Background {
    Solid {
        color: Color,
    },
    LinearGradient {
        start: Color,
        end: Color,
        angle_degrees: f32,
    },
}

impl Default for Background {
    fn default() -> Self {
        Self::Solid {
            color: Color::TRANSPARENT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub offset: Point,
    pub blur_radius: f32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Backdrop {
    pub padding: u32,
    pub corner_radius: u32,
    pub background: Background,
    pub shadow: Option<Shadow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub source_image: PathBuf,
    pub source_size: ImageSize,
    /// Editable canvas dimensions. Projects created before canvas expansion
    /// omit this field and open at the source image size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_size: Option<ImageSize>,
    pub annotations: Vec<Annotation>,
    pub crop: Option<ImageRect>,
    pub backdrop: Backdrop,
}

impl Project {
    pub fn is_project_path(path: impl AsRef<Path>) -> bool {
        path.as_ref()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("sniplet")
                    || extension.eq_ignore_ascii_case("clippy")
            })
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.to_json_pretty()?)?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut project = Self::from_json(&std::fs::read_to_string(path)?)?;
        if project.source_image.is_relative()
            && let Some(parent) = path.parent()
        {
            project.source_image = parent.join(&project.source_image);
        }
        Ok(project)
    }

    pub fn open_document(&self) -> Result<Document> {
        let image = image::open(&self.source_image)?.to_rgba8();
        Document::from_project(self.clone(), image)
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    original: Arc<RgbaImage>,
    original_is_opaque: bool,
    canvas_size: ImageSize,
    annotations: Vec<Annotation>,
    selected: Option<AnnotationId>,
    crop: Option<ImageRect>,
    backdrop: Backdrop,
    next_id: u64,
    undo: Vec<EditState>,
    redo: Vec<EditState>,
    group_start: Option<EditState>,
}

#[derive(Clone, Debug, PartialEq)]
struct EditState {
    canvas_size: ImageSize,
    annotations: Vec<Annotation>,
    selected: Option<AnnotationId>,
    crop: Option<ImageRect>,
    backdrop: Backdrop,
    next_id: u64,
}

impl Document {
    pub fn new(original: RgbaImage) -> Self {
        let canvas_size = ImageSize {
            width: original.width(),
            height: original.height(),
        };
        let original_is_opaque = original.pixels().all(|pixel| pixel[3] == 255);
        Self {
            original: Arc::new(original),
            original_is_opaque,
            canvas_size,
            annotations: Vec::new(),
            selected: None,
            crop: None,
            backdrop: Backdrop::default(),
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            group_start: None,
        }
    }

    pub fn from_project(project: Project, original: RgbaImage) -> Result<Self> {
        if project.version != PROJECT_VERSION {
            return Err(SnipletError::UnsupportedProjectVersion {
                expected: PROJECT_VERSION,
                actual: project.version,
            });
        }
        let actual = ImageSize {
            width: original.width(),
            height: original.height(),
        };
        if actual != project.source_size {
            return Err(SnipletError::ProjectImageMismatch {
                path: project.source_image,
                expected_width: project.source_size.width,
                expected_height: project.source_size.height,
                actual_width: actual.width,
                actual_height: actual.height,
            });
        }
        let next_id = project
            .annotations
            .iter()
            .map(|annotation| annotation.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut document = Self::new(original);
        document.canvas_size = project
            .canvas_size
            .map(|size| ImageSize {
                width: size.width.max(actual.width),
                height: size.height.max(actual.height),
            })
            .unwrap_or(actual);
        document.annotations = project.annotations;
        for annotation in &mut document.annotations {
            annotation
                .kind
                .normalize_and_clip(document.canvas_size.width, document.canvas_size.height);
        }
        document.crop = project
            .crop
            .map(|crop| crop.clipped(document.canvas_size.width, document.canvas_size.height));
        document.backdrop = project.backdrop;
        document.next_id = next_id;
        Ok(document)
    }

    pub fn to_project(&self, source_image: impl Into<PathBuf>) -> Project {
        Project {
            version: PROJECT_VERSION,
            source_image: source_image.into(),
            source_size: ImageSize {
                width: self.original.width(),
                height: self.original.height(),
            },
            canvas_size: Some(self.canvas_size),
            annotations: self.annotations.clone(),
            crop: self.crop,
            backdrop: self.backdrop,
        }
    }

    /// The immutable, undecorated source pixels.
    pub fn original(&self) -> &RgbaImage {
        &self.original
    }

    pub(crate) fn original_is_opaque(&self) -> bool {
        self.original_is_opaque
    }

    pub fn dimensions(&self) -> ImageSize {
        self.canvas_size
    }

    pub fn width(&self) -> u32 {
        self.canvas_size.width
    }

    pub fn height(&self) -> u32 {
        self.canvas_size.height
    }

    /// Grows the editable canvas to at least `width` by `height`, anchored at
    /// the immutable source image's top-left. Returns whether it changed.
    pub fn expand_canvas_to(&mut self, width: u32, height: u32) -> bool {
        let expanded = ImageSize {
            width: self.canvas_size.width.max(width),
            height: self.canvas_size.height.max(height),
        };
        if expanded == self.canvas_size {
            return false;
        }
        self.record_before_change();
        self.canvas_size = expanded;
        true
    }

    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn annotation(&self, id: AnnotationId) -> Option<&Annotation> {
        self.annotations
            .iter()
            .find(|annotation| annotation.id == id)
    }

    pub fn selected(&self) -> Option<AnnotationId> {
        self.selected
    }

    pub fn select(&mut self, id: Option<AnnotationId>) -> Result<()> {
        if let Some(id) = id
            && self.annotation(id).is_none()
        {
            return Err(SnipletError::UnknownAnnotation(id.0));
        }
        self.selected = id;
        Ok(())
    }

    pub fn crop(&self) -> Option<ImageRect> {
        self.crop
    }

    pub fn backdrop(&self) -> Backdrop {
        self.backdrop
    }

    pub fn sample_color(&self, point: Point) -> Option<Color> {
        let x = point.x.floor() as i64;
        let y = point.y.floor() as i64;
        (x >= 0 && y >= 0 && x < self.original.width() as i64 && y < self.original.height() as i64)
            .then(|| Color::from(*self.original.get_pixel(x as u32, y as u32)))
    }

    pub fn average_color(&self, rect: ImageRect) -> Option<Color> {
        let (left, top, right, bottom) =
            rect.pixel_bounds(self.original.width(), self.original.height())?;
        let mut sum = [0_u64; 4];
        let count = (right - left) as u64 * (bottom - top) as u64;
        for y in top..bottom {
            for x in left..right {
                for (total, channel) in sum.iter_mut().zip(self.original.get_pixel(x, y).0) {
                    *total += channel as u64;
                }
            }
        }
        Some(Color::new(
            (sum[0] / count) as u8,
            (sum[1] / count) as u8,
            (sum[2] / count) as u8,
            (sum[3] / count) as u8,
        ))
    }

    /// Finds the darkest source pixel in a circular image-space neighborhood.
    pub fn darkest_color_around(&self, point: Point, radius: u32) -> Option<Color> {
        let center_x = point.x.floor() as i64;
        let center_y = point.y.floor() as i64;
        let radius = radius as i64;
        let radius_squared = (radius as i128).pow(2);
        let start_x = center_x.saturating_sub(radius).max(0);
        let end_x = center_x
            .saturating_add(radius)
            .min(self.original.width() as i64 - 1);
        let start_y = center_y.saturating_sub(radius).max(0);
        let end_y = center_y
            .saturating_add(radius)
            .min(self.original.height() as i64 - 1);
        let mut darkest = None::<(u32, Color)>;
        for y in start_y..=end_y {
            for x in start_x..=end_x {
                let dx = (x - center_x) as i128;
                let dy = (y - center_y) as i128;
                if dx * dx + dy * dy > radius_squared {
                    continue;
                }
                let color = Color::from(*self.original.get_pixel(x as u32, y as u32));
                let luminance =
                    2_126 * color.r as u32 + 7_152 * color.g as u32 + 722 * color.b as u32;
                if darkest.is_none_or(|(darkest_luminance, _)| luminance < darkest_luminance) {
                    darkest = Some((luminance, color));
                }
            }
        }
        darkest.map(|(_, color)| color)
    }

    /// Trims a rough selection to pixels that differ from its corner-derived background.
    pub fn auto_adjust_selection(&self, rect: ImageRect) -> Option<ImageRect> {
        let (left, top, right, bottom) =
            rect.pixel_bounds(self.original.width(), self.original.height())?;
        let corners = [
            Color::from(*self.original.get_pixel(left, top)),
            Color::from(*self.original.get_pixel(right - 1, top)),
            Color::from(*self.original.get_pixel(left, bottom - 1)),
            Color::from(*self.original.get_pixel(right - 1, bottom - 1)),
        ];
        let background = average_colors(&corners);
        let corner_spread = corners
            .iter()
            .map(|color| color_difference(*color, background))
            .max()
            .unwrap_or(0);
        let tolerance = corner_spread.saturating_add(8).min(32);
        let (mut found_left, mut found_top) = (right, bottom);
        let (mut found_right, mut found_bottom) = (left, top);
        let mut found = false;
        for y in top..bottom {
            for x in left..right {
                let color = Color::from(*self.original.get_pixel(x, y));
                if color_difference(color, background) > tolerance {
                    found = true;
                    found_left = found_left.min(x);
                    found_top = found_top.min(y);
                    found_right = found_right.max(x + 1);
                    found_bottom = found_bottom.max(y + 1);
                }
            }
        }
        found.then(|| {
            ImageRect::new(
                found_left as f32,
                found_top as f32,
                (found_right - found_left) as f32,
                (found_bottom - found_top) as f32,
            )
        })
    }

    /// Flood-fills source pixels whose channels remain within `tolerance` of
    /// the seed and returns the region's image-space bounding rectangle.
    pub fn select_monotone_at(&self, point: Point, tolerance: u8) -> Option<ImageRect> {
        let seed_x = point.x.floor() as i64;
        let seed_y = point.y.floor() as i64;
        if seed_x < 0
            || seed_y < 0
            || seed_x >= self.original.width() as i64
            || seed_y >= self.original.height() as i64
        {
            return None;
        }
        let width = self.original.width();
        let height = self.original.height();
        let pixel_count = usize::try_from(width.checked_mul(height)?).ok()?;
        let seed = Color::from(*self.original.get_pixel(seed_x as u32, seed_y as u32));
        let mut visited = vec![false; pixel_count];
        let mut queue = VecDeque::from([(seed_x as u32, seed_y as u32)]);
        let (mut left, mut right, mut top, mut bottom) =
            (seed_x as u32, seed_x as u32, seed_y as u32, seed_y as u32);
        while let Some((x, y)) = queue.pop_front() {
            let index = y as usize * width as usize + x as usize;
            if visited[index] {
                continue;
            }
            visited[index] = true;
            let color = Color::from(*self.original.get_pixel(x, y));
            if color_difference(color, seed) > tolerance {
                continue;
            }
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            if x > 0 {
                queue.push_back((x - 1, y));
            }
            if x + 1 < width {
                queue.push_back((x + 1, y));
            }
            if y > 0 {
                queue.push_back((x, y - 1));
            }
            if y + 1 < height {
                queue.push_back((x, y + 1));
            }
        }
        Some(ImageRect::new(
            left as f32,
            top as f32,
            (right - left + 1) as f32,
            (bottom - top + 1) as f32,
        ))
    }

    pub fn add_annotation(
        &mut self,
        mut kind: AnnotationKind,
        style: AnnotationStyle,
    ) -> AnnotationId {
        kind.normalize_and_clip(self.width(), self.height());
        self.record_before_change();
        let id = AnnotationId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.annotations.push(Annotation { id, kind, style });
        id
    }

    pub fn update_annotation(
        &mut self,
        id: AnnotationId,
        mut kind: AnnotationKind,
        style: AnnotationStyle,
    ) -> Result<()> {
        let index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or(SnipletError::UnknownAnnotation(id.0))?;
        kind.normalize_and_clip(self.width(), self.height());
        let replacement = Annotation { id, kind, style };
        if self.annotations[index] != replacement {
            self.record_before_change();
            self.annotations[index] = replacement;
        }
        Ok(())
    }

    pub fn move_annotation(&mut self, id: AnnotationId, delta: Point) -> Result<()> {
        let index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or(SnipletError::UnknownAnnotation(id.0))?;
        if delta.x == 0.0 && delta.y == 0.0 {
            return Ok(());
        }
        self.record_before_change();
        self.annotations[index].kind.translate(delta);
        Ok(())
    }

    /// Resizes an annotation into `target_bounds` and records one undo step.
    ///
    /// Vector points map from their current bounds into the target. Text and
    /// counters choose a font size that fits while preserving glyph aspect.
    /// Embedded PNG annotations scale non-destructively; their encoded pixels
    /// stay unchanged and the target pixel size is stored separately.
    pub fn resize_annotation(&mut self, id: AnnotationId, target_bounds: ImageRect) -> Result<()> {
        let index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or(SnipletError::UnknownAnnotation(id.0))?;
        if !target_bounds.x.is_finite()
            || !target_bounds.y.is_finite()
            || !target_bounds.width.is_finite()
            || !target_bounds.height.is_finite()
            || target_bounds.width <= 0.0
            || target_bounds.height <= 0.0
        {
            return Err(SnipletError::InvalidAnnotationBounds);
        }
        let mut resized = self.annotations[index].kind.clone();
        resized.resize_to(target_bounds);
        if resized != self.annotations[index].kind {
            self.record_before_change();
            self.annotations[index].kind = resized;
        }
        Ok(())
    }

    pub fn delete_annotation(&mut self, id: AnnotationId) -> Result<Annotation> {
        let index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or(SnipletError::UnknownAnnotation(id.0))?;
        self.record_before_change();
        let removed = self.annotations.remove(index);
        if self.selected == Some(id) {
            self.selected = None;
        }
        Ok(removed)
    }

    pub fn clear_annotations(&mut self) {
        if !self.annotations.is_empty() {
            self.record_before_change();
            self.annotations.clear();
            self.selected = None;
        }
    }

    /// Returns the topmost annotation under an image-space point.
    pub fn hit_test(&self, point: Point, tolerance: f32) -> Option<AnnotationId> {
        self.annotations.iter().rev().find_map(|annotation| {
            annotation
                .kind
                .hit_test(point, tolerance.max(0.0), annotation.style.stroke_width)
                .then_some(annotation.id)
        })
    }

    pub fn select_at(&mut self, point: Point, tolerance: f32) -> Option<AnnotationId> {
        self.selected = self.hit_test(point, tolerance);
        self.selected
    }

    pub fn set_crop(&mut self, rect: ImageRect) {
        let rect = rect.clipped(self.width(), self.height());
        let crop = (rect.width > 0.0 && rect.height > 0.0).then_some(rect);
        if self.crop != crop {
            self.record_before_change();
            self.crop = crop;
        }
    }

    pub fn clear_crop(&mut self) {
        if self.crop.is_some() {
            self.record_before_change();
            self.crop = None;
        }
    }

    pub fn set_backdrop(&mut self, backdrop: Backdrop) {
        if self.backdrop != backdrop {
            self.record_before_change();
            self.backdrop = backdrop;
        }
    }

    /// Starts an undo group. All edits until [`Self::end_group`] undo together.
    pub fn begin_group(&mut self) -> Result<()> {
        if self.group_start.is_some() {
            return Err(SnipletError::EditGroupAlreadyOpen);
        }
        self.group_start = Some(self.state());
        Ok(())
    }

    pub fn end_group(&mut self) -> Result<()> {
        let start = self.group_start.take().ok_or(SnipletError::NoEditGroup)?;
        if start != self.state() {
            self.undo.push(start);
            self.redo.clear();
        }
        Ok(())
    }

    /// Rolls back every edit since [`Self::begin_group`] without changing the
    /// undo or redo stacks.
    pub fn cancel_group(&mut self) -> Result<()> {
        let start = self.group_start.take().ok_or(SnipletError::NoEditGroup)?;
        self.restore(start);
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        if self.group_start.is_some() {
            return false;
        }
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.state());
        self.restore(previous);
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.group_start.is_some() {
            return false;
        }
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.state());
        self.restore(next);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn record_before_change(&mut self) {
        if self.group_start.is_none() {
            self.undo.push(self.state());
            self.redo.clear();
        }
    }

    fn state(&self) -> EditState {
        EditState {
            canvas_size: self.canvas_size,
            annotations: self.annotations.clone(),
            selected: self.selected,
            crop: self.crop,
            backdrop: self.backdrop,
            next_id: self.next_id,
        }
    }

    fn restore(&mut self, state: EditState) {
        self.canvas_size = state.canvas_size;
        self.annotations = state.annotations;
        self.selected = state.selected.filter(|id| {
            self.annotations
                .iter()
                .any(|annotation| annotation.id == *id)
        });
        self.crop = state.crop;
        self.backdrop = state.backdrop;
        self.next_id = state.next_id;
    }
}

fn average_colors(colors: &[Color]) -> Color {
    let count = colors.len().max(1) as u32;
    let sum = colors.iter().fold([0_u32; 4], |mut sum, color| {
        sum[0] += color.r as u32;
        sum[1] += color.g as u32;
        sum[2] += color.b as u32;
        sum[3] += color.a as u32;
        sum
    });
    Color::new(
        (sum[0] / count) as u8,
        (sum[1] / count) as u8,
        (sum[2] / count) as u8,
        (sum[3] / count) as u8,
    )
}

fn color_difference(left: Color, right: Color) -> u8 {
    left.r
        .abs_diff(right.r)
        .max(left.g.abs_diff(right.g))
        .max(left.b.abs_diff(right.b))
        .max(left.a.abs_diff(right.a))
}
