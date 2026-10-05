use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Pixel, Rgba, RgbaImage};
use imageproc::{
    drawing::{
        draw_filled_circle_mut, draw_filled_ellipse_mut, draw_filled_rect_mut,
        draw_hollow_ellipse_mut, draw_hollow_rect_mut, draw_text_mut, text_size,
    },
    rect::Rect,
};
use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::{
    Annotation, AnnotationKind, ArrowVariant, Backdrop, Background, Color, Document, ImageRect,
    Measurement, Point, Result, Shadow, SnipletError,
    annotation::CompositeLayer,
    geometry::{
        ArrowHeadGeometry, QuadraticCurve, arrow_geometry, distance_to_segment, quadratic_bounds,
    },
};

#[derive(Clone, Copy, Debug)]
pub struct RenderOptions<'a> {
    pub font_bytes: Option<&'a [u8]>,
    pub apply_crop: bool,
    pub apply_backdrop: bool,
    /// Transient annotations placed last within each layer, useful for live previews.
    pub extra_annotations: &'a [Annotation],
}

impl Default for RenderOptions<'_> {
    fn default() -> Self {
        Self {
            font_bytes: None,
            apply_crop: true,
            apply_backdrop: true,
            extra_annotations: &[],
        }
    }
}

impl Measurement {
    /// Uses the same small overlay for the live ruler and the exported imprint.
    pub fn overlay(self, font_bytes: &[u8]) -> Result<(ImageRect, RgbaImage)> {
        let font = FontRef::try_from_slice(font_bytes).map_err(|_| SnipletError::InvalidFont)?;
        self.overlay_with_font(&font)
    }

    fn overlay_with_font(self, font: &FontRef<'_>) -> Result<(ImageRect, RgbaImage)> {
        if ![self.start.x, self.start.y, self.end.x, self.end.y]
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(SnipletError::InvalidAnnotationBounds);
        }
        let scale = self.scale();
        let text = self.label();
        let font_size = 14.0 * scale;
        let (tw, _) = text_size(PxScale::from(font_size), font, &text);
        let scaled = font.as_scaled(font_size);
        let (text_top, text_bottom) = text
            .chars()
            .filter_map(|character| {
                font.outline_glyph(
                    font.glyph_id(character)
                        .with_scale_and_position(font_size, ab_glyph::point(0.0, scaled.ascent())),
                )
            })
            .map(|glyph| glyph.px_bounds())
            .fold(
                (f32::INFINITY, f32::NEG_INFINITY),
                |(top, bottom), bounds| (top.min(bounds.min.y), bottom.max(bounds.max.y)),
            );
        let horizontal = (self.end.x - self.start.x).abs() >= (self.end.y - self.start.y).abs();
        let middle = Point::new(
            (self.start.x + self.end.x) * 0.5,
            (self.start.y + self.end.y) * 0.5,
        );
        let (w, h) = (
            tw as f32 + 8.0 * scale,
            text_bottom - text_top + 4.0 * scale,
        );
        let label = if horizontal {
            ImageRect::new(middle.x - w * 0.5, middle.y - h - 4.0 * scale, w, h)
        } else {
            ImageRect::new(middle.x + 4.0 * scale, middle.y - h * 0.5, w, h)
        };
        let bounds = union_rect(
            ImageRect::from_corners(self.start, self.end).expanded(5.0 * scale),
            label,
        )
        .expanded(1.0);
        let origin = Point::new(bounds.x.floor(), bounds.y.floor());
        let mut overlay = RgbaImage::new(
            (bounds.x + bounds.width - origin.x).ceil() as u32,
            (bounds.y + bounds.height - origin.y).ceil() as u32,
        );
        let local = |p: Point| Point::new(p.x - origin.x, p.y - origin.y);
        let red = Rgba([255, 59, 48, 255]);
        draw_thick_line(&mut overlay, local(self.start), local(self.end), scale, red);
        for end in [self.start, self.end] {
            let (a, b) = if horizontal {
                (
                    Point::new(end.x, end.y - 4.0 * scale),
                    Point::new(end.x, end.y + 4.0 * scale),
                )
            } else {
                (
                    Point::new(end.x - 4.0 * scale, end.y),
                    Point::new(end.x + 4.0 * scale, end.y),
                )
            };
            draw_thick_line(&mut overlay, local(a), local(b), scale, red);
        }
        draw_rect_fill(
            &mut overlay,
            label.translated(Point::new(-origin.x, -origin.y)),
            red,
        );
        draw_label(
            &mut overlay,
            local(Point::new(
                label.x + 4.0 * scale,
                label.y + 2.0 * scale - text_top,
            )),
            &text,
            font_size,
            Rgba([255, 255, 255, 255]),
            font,
        );
        Ok((
            ImageRect::new(
                origin.x,
                origin.y,
                overlay.width() as f32,
                overlay.height() as f32,
            ),
            overlay,
        ))
    }
}

impl Document {
    /// Measurement reads visible base pixels and reports their origin in source coordinates.
    pub fn render_measurement_source(
        &self,
        options: &RenderOptions<'_>,
    ) -> Result<(RgbaImage, Point)> {
        let source = self.render_raster(options)?;
        let Some(crop) = self.crop() else {
            return Ok((source, Point::default()));
        };
        Ok(match crop.pixel_bounds(source.width(), source.height()) {
            Some((left, top, right, bottom)) => (
                image::imageops::crop_imm(&source, left, top, right - left, bottom - top)
                    .to_image(),
                Point::new(left as f32, top as f32),
            ),
            None => (RgbaImage::new(0, 0), Point::default()),
        })
    }

    pub fn render_raster(&self, options: &RenderOptions<'_>) -> Result<RgbaImage> {
        self.raster_copy().render_source(&RenderOptions {
            extra_annotations: &[],
            ..*options
        })
    }

    pub fn render(&self, options: &RenderOptions<'_>) -> Result<RgbaImage> {
        let font = match options.font_bytes {
            Some(bytes) => {
                Some(FontRef::try_from_slice(bytes).map_err(|_| SnipletError::InvalidFont)?)
            }
            None => None,
        };
        let mut canvas = if self.dimensions().width == self.original().width()
            && self.dimensions().height == self.original().height()
        {
            if self.original_is_opaque() {
                self.original().clone()
            } else {
                clone_source_for_overlay(self.original())
            }
        } else {
            let mut canvas = RgbaImage::new(self.width(), self.height());
            image::imageops::overlay(&mut canvas, self.original(), 0, 0);
            canvas
        };
        let mut annotations: Vec<_> = self
            .annotations()
            .iter()
            .chain(options.extra_annotations.iter())
            .collect();
        annotations.sort_by_key(|annotation| annotation.kind.composite_layer());
        let (base, drawings) = annotations.split_at(annotations.partition_point(|annotation| {
            annotation.kind.composite_layer() < CompositeLayer::Spotlight
        }));
        let mut annotation_layer = None;
        for annotation in base {
            draw_annotation(
                &mut canvas,
                self.original(),
                annotation,
                font.as_ref(),
                &mut annotation_layer,
            )?;
        }
        draw_spotlights(&mut canvas, &annotations);
        for annotation in drawings {
            draw_annotation(
                &mut canvas,
                self.original(),
                annotation,
                font.as_ref(),
                &mut annotation_layer,
            )?;
        }

        if options.apply_crop
            && let Some(crop) = self.crop()
            && let Some((left, top, right, bottom)) =
                crop.pixel_bounds(canvas.width(), canvas.height())
        {
            canvas = image::imageops::crop_imm(&canvas, left, top, right - left, bottom - top)
                .to_image();
        }
        if options.apply_backdrop && !backdrop_is_identity(self.backdrop()) {
            canvas = apply_backdrop(&canvas, self.backdrop());
        }
        Ok(canvas)
    }

    /// Renders annotations over the full canvas coordinate space, ignoring crop and backdrop.
    pub fn render_source(&self, options: &RenderOptions<'_>) -> Result<RgbaImage> {
        let source_options = RenderOptions {
            apply_crop: false,
            apply_backdrop: false,
            ..*options
        };
        self.render(&source_options)
    }

    /// Location of source-image (0, 0) in the rendered image.
    pub fn render_content_origin(&self, options: &RenderOptions<'_>) -> Point {
        let (crop_x, crop_y) = if options.apply_crop {
            self.crop()
                .and_then(|crop| crop.pixel_bounds(self.width(), self.height()))
                .map(|(left, top, _, _)| (left as f32, top as f32))
                .unwrap_or((0.0, 0.0))
        } else {
            (0.0, 0.0)
        };
        let padding = if options.apply_backdrop {
            self.backdrop().padding as f32
        } else {
            0.0
        };
        Point::new(-crop_x + padding, -crop_y + padding)
    }
}

fn draw_annotation(
    canvas: &mut RgbaImage,
    source: &RgbaImage,
    annotation: &Annotation,
    font: Option<&FontRef<'_>>,
    annotation_layer: &mut Option<RgbaImage>,
) -> Result<()> {
    match &annotation.kind {
        AnnotationKind::Measurement { measurement } => {
            let (bounds, overlay) =
                measurement.overlay_with_font(font.ok_or(SnipletError::MissingFont)?)?;
            image::imageops::overlay(canvas, &overlay, bounds.x as i64, bounds.y as i64);
        }
        AnnotationKind::Pixelate { rect, block_size } => {
            pixelate(canvas, *rect, (*block_size).max(2));
        }
        AnnotationKind::Blur { rect, radius } => blur(canvas, *rect, (*radius).max(0.1)),
        AnnotationKind::RemoveFill {
            rect,
            sample: sampled_point,
        } => {
            let fill = sampled_point
                .and_then(|point| sample(source, point))
                .unwrap_or_else(|| average_border(source, *rect));
            draw_rect_fill(canvas, *rect, fill.into());
        }
        AnnotationKind::Image {
            origin,
            png_bytes,
            size,
        } => {
            let pasted = image::load_from_memory(png_bytes)?.to_rgba8();
            let pasted = match size {
                Some(size) if size.width > 0 && size.height > 0 => image::imageops::resize(
                    &pasted,
                    size.width,
                    size.height,
                    image::imageops::FilterType::CatmullRom,
                ),
                _ => pasted,
            };
            image::imageops::overlay(
                canvas,
                &pasted,
                origin.x.round() as i64,
                origin.y.round() as i64,
            );
        }
        AnnotationKind::Arrow {
            start,
            end,
            bend,
            variant,
        } => draw_arrow(
            canvas,
            [
                *start,
                bend.unwrap_or_else(|| {
                    Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5)
                }),
                *end,
            ],
            annotation.style.stroke_width.max(1.0),
            annotation.style.stroke.into(),
            *variant,
            CompositeMode::Over,
        ),
        AnnotationKind::Magnifier { rect, zoom, .. } => {
            let layer = annotation_layer
                .get_or_insert_with(|| RgbaImage::new(canvas.width(), canvas.height()));
            if annotation.kind.magnifier_circles().is_some() {
                draw_linked_magnifier(layer, source, annotation);
            } else {
                draw_magnifier(layer, source, *rect, *zoom, annotation);
            }
            composite_annotation_layer(
                canvas,
                layer,
                annotation_layer_bounds(annotation, font, canvas.width(), canvas.height()),
            );
            #[cfg(test)]
            assert!(
                layer.pixels().all(|pixel| pixel[3] == 0),
                "magnifier bounds left painted pixels in the shared layer"
            );
        }
        AnnotationKind::Spotlight { .. } => {}
        _ => {
            let layer = annotation_layer
                .get_or_insert_with(|| RgbaImage::new(canvas.width(), canvas.height()));
            draw_layer_annotation(layer, annotation, font)?;
            composite_annotation_layer(
                canvas,
                layer,
                annotation_layer_bounds(annotation, font, canvas.width(), canvas.height()),
            );
            #[cfg(test)]
            assert!(
                layer.pixels().all(|pixel| pixel[3] == 0),
                "bounds left painted pixels in the shared layer for {:?}",
                annotation.kind
            );
        }
    }
    Ok(())
}

fn clone_source_for_overlay(source: &RgbaImage) -> RgbaImage {
    let mut canvas = source.clone();
    for pixel in canvas.pixels_mut() {
        if pixel[3] != 255 {
            let source_pixel = *pixel;
            *pixel = Rgba([0, 0, 0, 0]);
            pixel.blend(&source_pixel);
        }
    }
    canvas
}

fn draw_layer_annotation(
    layer: &mut RgbaImage,
    annotation: &Annotation,
    font: Option<&FontRef<'_>>,
) -> Result<()> {
    let color: Rgba<u8> = annotation.style.stroke.into();
    let width = annotation.style.stroke_width.max(1.0);
    match &annotation.kind {
        AnnotationKind::Line { start, end } => draw_thick_line(layer, *start, *end, width, color),
        AnnotationKind::Arrow {
            start,
            end,
            bend,
            variant,
        } => draw_arrow(
            layer,
            [
                *start,
                bend.unwrap_or_else(|| {
                    Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5)
                }),
                *end,
            ],
            width,
            color,
            *variant,
            CompositeMode::Replace,
        ),
        AnnotationKind::Rectangle { rect } => {
            if let Some(fill) = annotation.style.fill {
                draw_rect_fill(layer, *rect, fill.into());
            }
            draw_rect_outline(layer, *rect, width, color);
        }
        AnnotationKind::Ellipse { rect } => {
            draw_ellipse(layer, *rect, width, color, annotation.style.fill)
        }
        AnnotationKind::Text {
            origin,
            text,
            font_size,
        } => draw_label(
            layer,
            *origin,
            text,
            *font_size,
            annotation
                .style
                .fill
                .unwrap_or(annotation.style.stroke)
                .into(),
            font.ok_or(SnipletError::MissingFont)?,
        ),
        AnnotationKind::Counter {
            center,
            value,
            font_size,
        } => draw_counter(
            layer,
            *center,
            *value,
            *font_size,
            color,
            annotation.style.fill.unwrap_or(Color::WHITE).into(),
            font.ok_or(SnipletError::MissingFont)?,
        ),
        AnnotationKind::Highlight { rect } => draw_rect_fill(
            layer,
            *rect,
            annotation
                .style
                .fill
                .unwrap_or(Color::new(255, 225, 0, 96))
                .into(),
        ),
        AnnotationKind::Freehand { points } => {
            for points in points.windows(2) {
                draw_thick_line(layer, points[0], points[1], width, color);
            }
            if let Some(point) = points.first() {
                draw_thick_line(layer, *point, *point, width, color);
            }
        }
        AnnotationKind::Redaction { rect } => draw_rect_fill(
            layer,
            *rect,
            annotation.style.fill.unwrap_or(Color::BLACK).into(),
        ),
        AnnotationKind::Pixelate { .. }
        | AnnotationKind::Measurement { .. }
        | AnnotationKind::Blur { .. }
        | AnnotationKind::Spotlight { .. }
        | AnnotationKind::RemoveFill { .. }
        | AnnotationKind::Image { .. }
        | AnnotationKind::Magnifier { .. } => unreachable!(),
    }
    Ok(())
}

type PixelBounds = (u32, u32, u32, u32);

fn annotation_layer_bounds(
    annotation: &Annotation,
    font: Option<&FontRef<'_>>,
    width: u32,
    height: u32,
) -> Option<PixelBounds> {
    if width == 0 || height == 0 {
        return None;
    }
    let full = || Some((0, 0, width, height));
    let finite = |values: &[f32]| values.iter().all(|value| value.is_finite());
    let stroke_padding = annotation.style.stroke_width.max(1.0) * 0.5 + 2.0;

    let rect = match &annotation.kind {
        AnnotationKind::Line { start, end } => {
            if !finite(&[start.x, start.y, end.x, end.y, stroke_padding]) {
                return full();
            }
            ImageRect::from_corners(*start, *end).expanded(stroke_padding)
        }
        AnnotationKind::Arrow { .. } | AnnotationKind::Measurement { .. } => return None,
        AnnotationKind::Rectangle { rect } | AnnotationKind::Ellipse { rect } => {
            if !finite(&[rect.x, rect.y, rect.width, rect.height]) {
                return full();
            }
            rect.expanded(2.0)
        }
        AnnotationKind::Text {
            origin,
            text,
            font_size,
        } => {
            if !finite(&[origin.x, origin.y, *font_size]) {
                return full();
            }
            let Some(font) = font else {
                return full();
            };
            let (text_width, text_height) =
                text_size(PxScale::from(font_size.max(1.0)), font, text);
            let padding = font_size.max(1.0) * 2.0 + 2.0;
            ImageRect::new(
                origin.x.round() - padding,
                origin.y.round() - padding,
                text_width as f32 + padding * 2.0,
                text_height as f32 + padding * 2.0,
            )
        }
        AnnotationKind::Counter {
            center,
            value,
            font_size,
        } => {
            if !finite(&[center.x, center.y, *font_size]) {
                return full();
            }
            let radius = (font_size * 0.65).max(8.0).round();
            let mut bounds = ImageRect::new(
                center.x.round() - radius - 1.0,
                center.y.round() - radius - 1.0,
                radius * 2.0 + 3.0,
                radius * 2.0 + 3.0,
            );
            if let Some(font) = font {
                let text = value.to_string();
                let scale = PxScale::from(font_size.max(1.0));
                let (text_width, text_height) = text_size(scale, font, &text);
                let padding = font_size.max(1.0) * 2.0 + 2.0;
                let text_bounds = ImageRect::new(
                    (center.x - text_width as f32 * 0.5).round() - padding,
                    (center.y - text_height as f32 * 0.5).round() - padding,
                    text_width as f32 + padding * 2.0,
                    text_height as f32 + padding * 2.0,
                );
                bounds = union_rect(bounds, text_bounds);
            }
            bounds
        }
        AnnotationKind::Magnifier { .. } => {
            let bounds = annotation.kind.bounds();
            if !finite(&[
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                stroke_padding,
            ]) {
                return full();
            }
            bounds.expanded(stroke_padding)
        }
        AnnotationKind::Highlight { rect } | AnnotationKind::Redaction { rect } => {
            if !finite(&[rect.x, rect.y, rect.width, rect.height]) {
                return full();
            }
            rect.expanded(2.0)
        }
        AnnotationKind::Freehand { points } => {
            if points.iter().any(|point| !finite(&[point.x, point.y])) {
                return full();
            }
            annotation.kind.bounds().expanded(stroke_padding)
        }
        AnnotationKind::Pixelate { .. }
        | AnnotationKind::Blur { .. }
        | AnnotationKind::Spotlight { .. }
        | AnnotationKind::RemoveFill { .. }
        | AnnotationKind::Image { .. } => return None,
    };
    rect.pixel_bounds(width, height)
}

fn composite_annotation_layer(
    canvas: &mut RgbaImage,
    layer: &mut RgbaImage,
    bounds: Option<PixelBounds>,
) {
    let Some((left, top, right, bottom)) = bounds else {
        return;
    };
    let row_width = canvas.width() as usize;
    let canvas = canvas.as_mut();
    let layer = layer.as_mut();
    for y in top as usize..bottom as usize {
        for x in left as usize..right as usize {
            let index = (y * row_width + x) * 4;
            let alpha = layer[index + 3];
            if alpha == 0 {
                continue;
            }
            if alpha == 255 {
                canvas[index..index + 4].copy_from_slice(&layer[index..index + 4]);
            } else {
                let mut destination = Rgba([
                    canvas[index],
                    canvas[index + 1],
                    canvas[index + 2],
                    canvas[index + 3],
                ]);
                destination.blend(&Rgba([
                    layer[index],
                    layer[index + 1],
                    layer[index + 2],
                    alpha,
                ]));
                canvas[index..index + 4].copy_from_slice(&destination.0);
            }
            layer[index..index + 4].fill(0);
        }
    }
}

fn draw_spotlights(canvas: &mut RgbaImage, annotations: &[&Annotation]) {
    let spotlights: Vec<_> = annotations
        .iter()
        .filter_map(|annotation| match &annotation.kind {
            AnnotationKind::Spotlight { rect } => Some((rect.normalized(), annotation.style)),
            _ => None,
        })
        .collect();
    let Some((_, style)) = spotlights.last() else {
        return;
    };
    let dim: Rgba<u8> = style.fill.unwrap_or(Color::new(0, 0, 0, 150)).into();
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
            if !spotlights.iter().any(|(rect, _)| rect.contains(point)) {
                canvas.get_pixel_mut(x, y).blend(&dim);
            }
        }
    }
}

fn draw_linked_magnifier(canvas: &mut RgbaImage, source: &RgbaImage, annotation: &Annotation) {
    let Some([sample, lens]) = annotation.kind.magnifier_circles() else {
        return;
    };
    let Some((left, top, right, bottom)) =
        annotation_layer_bounds(annotation, None, canvas.width(), canvas.height())
    else {
        return;
    };
    let Some(mut mask) = Pixmap::new(right - left, bottom - top) else {
        return;
    };
    let center =
        |rect: ImageRect| Point::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let lens_center = center(lens);
    let sample_center = center(sample);
    let zoom = lens.width / sample.width;
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    let mut path = PathBuilder::new();
    path.push_circle(
        lens_center.x - left as f32,
        lens_center.y - top as f32,
        lens.width * 0.5,
    );
    if let Some(path) = path.finish() {
        mask.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        for (index, pixel) in mask.pixels().iter().enumerate() {
            if pixel.alpha() == 0 {
                continue;
            }
            let x = left + index as u32 % mask.width();
            let y = top + index as u32 / mask.width();
            let sx = ((x as f32 + 0.5 - lens_center.x) / zoom + sample_center.x).floor();
            let sy = ((y as f32 + 0.5 - lens_center.y) / zoom + sample_center.y).floor();
            if sx < 0.0 || sy < 0.0 || sx >= source.width() as f32 || sy >= source.height() as f32 {
                continue;
            }
            let mut color = *source.get_pixel(sx as u32, sy as u32);
            color[3] = ((color[3] as u16 * pixel.alpha() as u16 + 127) / 255) as u8;
            canvas.put_pixel(x, y, color);
        }
    }
    mask.fill(tiny_skia::Color::TRANSPARENT);
    let mut path = PathBuilder::new();
    for circle in [sample, lens] {
        let center = center(circle);
        path.push_circle(
            center.x - left as f32,
            center.y - top as f32,
            circle.width * 0.5,
        );
    }
    if let Some([start, end]) = annotation.kind.magnifier_connector() {
        path.move_to(start.x - left as f32, start.y - top as f32);
        path.line_to(end.x - left as f32, end.y - top as f32);
    }
    if let Some(path) = path.finish() {
        mask.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: annotation.style.stroke_width.max(1.0),
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
        composite_mask(
            canvas,
            &mask,
            left,
            top,
            annotation.style.stroke.into(),
            CompositeMode::Over,
        );
    }
}

fn draw_magnifier(
    canvas: &mut RgbaImage,
    source: &RgbaImage,
    rect: ImageRect,
    zoom: f32,
    annotation: &Annotation,
) {
    let Some((left, top, right, bottom)) = rect.pixel_bounds(canvas.width(), canvas.height())
    else {
        return;
    };
    let destination_width = right - left;
    let destination_height = bottom - top;
    let zoom = if zoom.is_finite() {
        zoom.clamp(1.0, 16.0)
    } else {
        1.0
    };
    let sample_width = ((destination_width as f32 / zoom).round() as u32)
        .max(1)
        .min(source.width());
    let sample_height = ((destination_height as f32 / zoom).round() as u32)
        .max(1)
        .min(source.height());
    let center_x = left + destination_width / 2;
    let center_y = top + destination_height / 2;
    let sample_left = center_x
        .saturating_sub(sample_width / 2)
        .min(source.width() - sample_width);
    let sample_top = center_y
        .saturating_sub(sample_height / 2)
        .min(source.height() - sample_height);
    let region =
        image::imageops::crop_imm(source, sample_left, sample_top, sample_width, sample_height)
            .to_image();
    let enlarged = image::imageops::resize(
        &region,
        destination_width,
        destination_height,
        image::imageops::FilterType::CatmullRom,
    );
    let rx = destination_width as f32 * 0.5;
    let ry = destination_height as f32 * 0.5;
    for y in 0..destination_height {
        for x in 0..destination_width {
            let nx = (x as f32 + 0.5 - rx) / rx.max(0.5);
            let ny = (y as f32 + 0.5 - ry) / ry.max(0.5);
            if nx * nx + ny * ny <= 1.0 {
                canvas.put_pixel(left + x, top + y, *enlarged.get_pixel(x, y));
            }
        }
    }
    draw_ellipse(
        canvas,
        ImageRect::new(
            left as f32,
            top as f32,
            destination_width as f32,
            destination_height as f32,
        ),
        annotation.style.stroke_width,
        annotation.style.stroke.into(),
        None,
    );
}

fn draw_thick_line(image: &mut RgbaImage, start: Point, end: Point, width: f32, color: Rgba<u8>) {
    if image.width() == 0
        || image.height() == 0
        || !start.x.is_finite()
        || !start.y.is_finite()
        || !end.x.is_finite()
        || !end.y.is_finite()
        || !width.is_finite()
        || color[3] == 0
    {
        return;
    }

    let radius = width.max(1.0) * 0.5;
    let antialias_edge = radius + 0.5;
    let left = (start.x.min(end.x) - antialias_edge).floor().max(0.0) as u32;
    let top = (start.y.min(end.y) - antialias_edge).floor().max(0.0) as u32;
    let right = (start.x.max(end.x) + antialias_edge)
        .ceil()
        .min(image.width() as f32) as u32;
    let bottom = (start.y.max(end.y) + antialias_edge)
        .ceil()
        .min(image.height() as f32) as u32;

    let dx = end.x - start.x;
    let dy = end.y - start.y;
    for y in top..bottom {
        let (row_left, row_right) = if dy.abs() <= f32::EPSILON {
            (left, right)
        } else {
            let sample_y = y as f32 + 0.5;
            let first_t = ((sample_y - antialias_edge - start.y) / dy).clamp(0.0, 1.0);
            let second_t = ((sample_y + antialias_edge - start.y) / dy).clamp(0.0, 1.0);
            let first_x = start.x + first_t * dx;
            let second_x = start.x + second_t * dx;
            let row_left = (first_x.min(second_x) - antialias_edge - 1.0)
                .floor()
                .max(left as f32) as u32;
            let row_right = (first_x.max(second_x) + antialias_edge + 1.0)
                .ceil()
                .min(right as f32) as u32;
            (row_left, row_right)
        };
        for x in row_left..row_right {
            let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
            let distance = distance_to_segment(point, start, end);
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            let alpha = (color[3] as f32 * coverage).round() as u8;
            let pixel = image.get_pixel_mut(x, y);
            if alpha > pixel[3] {
                *pixel = Rgba([color[0], color[1], color[2], alpha]);
            }
        }
    }
}

fn draw_arrow(
    image: &mut RgbaImage,
    points: [Point; 3],
    width: f32,
    color: Rgba<u8>,
    variant: ArrowVariant,
    mode: CompositeMode,
) {
    let [start, middle, end] = points;
    match variant {
        ArrowVariant::Solid => draw_solid_arrow(image, start, middle, end, width, color, mode),
        ArrowVariant::HandDrawn | ArrowVariant::Thin | ArrowVariant::DoubleEnded => {
            draw_open_arrow(image, points, width, color, variant, mode)
        }
    }
}

#[derive(Clone, Copy)]
enum CompositeMode {
    Replace,
    Over,
}

fn draw_solid_arrow(
    image: &mut RgbaImage,
    start: Point,
    middle: Point,
    end: Point,
    width: f32,
    color: Rgba<u8>,
    mode: CompositeMode,
) {
    if image.width() == 0
        || image.height() == 0
        || !start.x.is_finite()
        || !start.y.is_finite()
        || !middle.x.is_finite()
        || !middle.y.is_finite()
        || !end.x.is_finite()
        || !end.y.is_finite()
        || !width.is_finite()
        || color[3] == 0
    {
        return;
    }

    let geometry = arrow_geometry(start, middle, end, width, ArrowVariant::Solid);
    let width = geometry.stroke_width;
    let control = geometry.control;
    let Some(ArrowHeadGeometry::Filled(head)) = geometry.end_head else {
        return;
    };
    let curve_bounds = quadratic_bounds(start, control, end);
    let mut left = curve_bounds.x;
    let mut top = curve_bounds.y;
    let mut right = curve_bounds.x + curve_bounds.width;
    let mut bottom = curve_bounds.y + curve_bounds.height;
    for point in head {
        left = left.min(point.x);
        top = top.min(point.y);
        right = right.max(point.x);
        bottom = bottom.max(point.y);
    }
    let padding = width * 0.5 + 1.0;
    let left = (left - padding).floor().max(0.0) as u32;
    let top = (top - padding).floor().max(0.0) as u32;
    let right = (right + padding).ceil().min(image.width() as f32) as u32;
    let bottom = (bottom + padding).ceil().min(image.height() as f32) as u32;
    if right <= left || bottom <= top {
        return;
    }

    let Some(mut mask) = Pixmap::new(right - left, bottom - top) else {
        return;
    };
    let offset = |point: Point| (point.x - left as f32, point.y - top as f32);
    let (start_x, start_y) = offset(start);
    let shaft_end_t = quadratic_t_before_end(start, control, end, width * 1.5);
    let shaft_control = Point::new(
        start.x + (control.x - start.x) * shaft_end_t,
        start.y + (control.y - start.y) * shaft_end_t,
    );
    let shaft_end = crate::geometry::quadratic_point(start, control, end, shaft_end_t);
    let (shaft_control_x, shaft_control_y) = offset(shaft_control);
    let (shaft_end_x, shaft_end_y) = offset(shaft_end);
    let mut curve = PathBuilder::new();
    curve.move_to(start_x, start_y);
    curve.quad_to(shaft_control_x, shaft_control_y, shaft_end_x, shaft_end_y);
    let Some(curve) = curve.finish() else {
        return;
    };

    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    paint.force_hq_pipeline = true;
    let stroke = Stroke {
        width,
        line_cap: LineCap::Butt,
        ..Stroke::default()
    };
    mask.stroke_path(&curve, &paint, &stroke, Transform::identity(), None);

    if head[0] != head[1] {
        let mut triangle = PathBuilder::new();
        let (tip_x, tip_y) = offset(head[0]);
        let (left_x, left_y) = offset(head[1]);
        let (right_x, right_y) = offset(head[2]);
        triangle.move_to(tip_x, tip_y);
        triangle.line_to(left_x, left_y);
        triangle.line_to(right_x, right_y);
        triangle.close();
        if let Some(triangle) = triangle.finish() {
            mask.fill_path(
                &triangle,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    for (index, pixel) in mask.pixels().iter().enumerate() {
        let coverage = pixel.alpha();
        if coverage == 0 {
            continue;
        }
        let x = left + index as u32 % mask.width();
        let y = top + index as u32 / mask.width();
        let alpha = ((color[3] as u16 * coverage as u16 + 127) / 255) as u8;
        composite_pixel(
            image,
            x,
            y,
            Rgba([color[0], color[1], color[2], alpha]),
            mode,
        );
    }
}

fn draw_open_arrow(
    image: &mut RgbaImage,
    points: [Point; 3],
    size: f32,
    color: Rgba<u8>,
    variant: ArrowVariant,
    mode: CompositeMode,
) {
    let [start, middle, end] = points;
    if image.width() == 0
        || image.height() == 0
        || !start.x.is_finite()
        || !start.y.is_finite()
        || !middle.x.is_finite()
        || !middle.y.is_finite()
        || !end.x.is_finite()
        || !end.y.is_finite()
        || !size.is_finite()
        || color[3] == 0
    {
        return;
    }

    let geometry = arrow_geometry(start, middle, end, size, variant);
    if start == end && start == geometry.control {
        if matches!(mode, CompositeMode::Replace) {
            draw_thick_line(image, start, start, geometry.stroke_width, color);
        } else {
            draw_thick_line_over(image, start, start, geometry.stroke_width, color);
        }
        return;
    }

    let mut bounds = quadratic_bounds(start, geometry.control, end);
    for head in geometry.heads() {
        let ArrowHeadGeometry::Open(arms) = head else {
            continue;
        };
        for arm in arms {
            let arm_bounds = quadratic_bounds(arm.start, arm.control, arm.end);
            bounds = union_rect(bounds, arm_bounds);
        }
    }
    let padding = geometry.stroke_width * 0.5 + 1.0;
    let left = (bounds.x - padding).floor().max(0.0) as u32;
    let top = (bounds.y - padding).floor().max(0.0) as u32;
    let right = (bounds.x + bounds.width + padding)
        .ceil()
        .min(image.width() as f32) as u32;
    let bottom = (bounds.y + bounds.height + padding)
        .ceil()
        .min(image.height() as f32) as u32;
    if right <= left || bottom <= top {
        return;
    }

    let Some(mut mask) = Pixmap::new(right - left, bottom - top) else {
        return;
    };
    let offset = |point: Point| Point::new(point.x - left as f32, point.y - top as f32);
    let mut path = PathBuilder::new();
    append_quadratic(
        &mut path,
        QuadraticCurve {
            start,
            control: geometry.control,
            end,
        },
        &offset,
    );
    for head in geometry.heads() {
        let ArrowHeadGeometry::Open(arms) = head else {
            continue;
        };
        for arm in arms {
            append_quadratic(&mut path, arm, &offset);
        }
    }
    let Some(path) = path.finish() else {
        return;
    };

    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    paint.force_hq_pipeline = true;
    let stroke = Stroke {
        width: geometry.stroke_width,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    mask.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    composite_mask(image, &mask, left, top, color, mode);
}

fn append_quadratic(
    path: &mut PathBuilder,
    curve: QuadraticCurve,
    offset: &impl Fn(Point) -> Point,
) {
    let start = offset(curve.start);
    let control = offset(curve.control);
    let end = offset(curve.end);
    path.move_to(start.x, start.y);
    path.quad_to(control.x, control.y, end.x, end.y);
}

fn union_rect(first: ImageRect, second: ImageRect) -> ImageRect {
    let left = first.x.min(second.x);
    let top = first.y.min(second.y);
    let right = (first.x + first.width).max(second.x + second.width);
    let bottom = (first.y + first.height).max(second.y + second.height);
    ImageRect::new(left, top, right - left, bottom - top)
}

fn composite_mask(
    image: &mut RgbaImage,
    mask: &Pixmap,
    left: u32,
    top: u32,
    color: Rgba<u8>,
    mode: CompositeMode,
) {
    for (index, pixel) in mask.pixels().iter().enumerate() {
        let coverage = pixel.alpha();
        if coverage == 0 {
            continue;
        }
        let x = left + index as u32 % mask.width();
        let y = top + index as u32 / mask.width();
        let alpha = ((color[3] as u16 * coverage as u16 + 127) / 255) as u8;
        composite_pixel(
            image,
            x,
            y,
            Rgba([color[0], color[1], color[2], alpha]),
            mode,
        );
    }
}

fn composite_pixel(image: &mut RgbaImage, x: u32, y: u32, source: Rgba<u8>, mode: CompositeMode) {
    match mode {
        CompositeMode::Replace => image.put_pixel(x, y, source),
        CompositeMode::Over => image.get_pixel_mut(x, y).blend(&source),
    }
}

fn draw_thick_line_over(
    image: &mut RgbaImage,
    start: Point,
    end: Point,
    width: f32,
    color: Rgba<u8>,
) {
    let mut layer = RgbaImage::new(image.width(), image.height());
    draw_thick_line(&mut layer, start, end, width, color);
    let bounds = ImageRect::from_corners(start, end)
        .expanded(width.max(1.0) * 0.5 + 2.0)
        .pixel_bounds(image.width(), image.height());
    composite_annotation_layer(image, &mut layer, bounds);
}

fn quadratic_t_before_end(start: Point, control: Point, end: Point, target_distance: f32) -> f32 {
    const STEPS: usize = 64;
    let mut distance = 0.0;
    let mut later_t = 1.0;
    let mut later = end;
    for index in (0..STEPS).rev() {
        let earlier_t = index as f32 / STEPS as f32;
        let earlier = crate::geometry::quadratic_point(start, control, end, earlier_t);
        let segment = earlier.distance_to(later);
        if distance + segment >= target_distance && segment > f32::EPSILON {
            let fraction = (target_distance - distance) / segment;
            return later_t + (earlier_t - later_t) * fraction;
        }
        distance += segment;
        later_t = earlier_t;
        later = earlier;
    }
    0.0
}

fn draw_rect_fill(image: &mut RgbaImage, rect: ImageRect, color: Rgba<u8>) {
    if let Some(rect) = draw_rect(rect, image.width(), image.height()) {
        draw_filled_rect_mut(image, rect, color);
    }
}

fn draw_rect_outline(image: &mut RgbaImage, rect: ImageRect, width: f32, color: Rgba<u8>) {
    let rect = rect.normalized();
    for inset in 0..width.ceil() as u32 {
        let inset = inset as f32;
        let inner = ImageRect::new(
            rect.x + inset,
            rect.y + inset,
            (rect.width - inset * 2.0).max(0.0),
            (rect.height - inset * 2.0).max(0.0),
        );
        if let Some(rect) = draw_rect(inner, image.width(), image.height()) {
            draw_hollow_rect_mut(image, rect, color);
        }
    }
}

fn draw_rect(rect: ImageRect, width: u32, height: u32) -> Option<Rect> {
    let (left, top, right, bottom) = rect.pixel_bounds(width, height)?;
    Some(Rect::at(left as i32, top as i32).of_size(right - left, bottom - top))
}

fn draw_ellipse(
    image: &mut RgbaImage,
    rect: ImageRect,
    width: f32,
    stroke: Rgba<u8>,
    fill: Option<Color>,
) {
    let rect = rect.normalized();
    let center = (
        (rect.x + rect.width * 0.5).round() as i32,
        (rect.y + rect.height * 0.5).round() as i32,
    );
    let rx = (rect.width * 0.5).round() as i32;
    let ry = (rect.height * 0.5).round() as i32;
    if let Some(fill) = fill {
        draw_filled_ellipse_mut(image, center, rx, ry, fill.into());
    }
    for inset in 0..width.ceil() as i32 {
        if rx - inset > 0 && ry - inset > 0 {
            draw_hollow_ellipse_mut(image, center, rx - inset, ry - inset, stroke);
        }
    }
}

fn draw_label(
    image: &mut RgbaImage,
    origin: Point,
    text: &str,
    font_size: f32,
    color: Rgba<u8>,
    font: &FontRef<'_>,
) {
    draw_text_mut(
        image,
        color,
        origin.x.round() as i32,
        origin.y.round() as i32,
        PxScale::from(font_size.max(1.0)),
        font,
        text,
    );
}

fn draw_counter(
    image: &mut RgbaImage,
    center: Point,
    value: u32,
    font_size: f32,
    background: Rgba<u8>,
    foreground: Rgba<u8>,
    font: &FontRef<'_>,
) {
    let radius = (font_size * 0.65).max(8.0).round() as i32;
    draw_filled_circle_mut(
        image,
        (center.x.round() as i32, center.y.round() as i32),
        radius,
        background,
    );
    let text = value.to_string();
    let scale = PxScale::from(font_size.max(1.0));
    let (width, height) = text_size(scale, font, &text);
    draw_text_mut(
        image,
        foreground,
        (center.x - width as f32 * 0.5).round() as i32,
        (center.y - height as f32 * 0.5).round() as i32,
        scale,
        font,
        &text,
    );
}

fn pixelate(image: &mut RgbaImage, rect: ImageRect, block_size: u32) {
    let Some((left, top, right, bottom)) = rect.pixel_bounds(image.width(), image.height()) else {
        return;
    };
    for block_y in (top..bottom).step_by(block_size as usize) {
        for block_x in (left..right).step_by(block_size as usize) {
            let block_right = (block_x + block_size).min(right);
            let block_bottom = (block_y + block_size).min(bottom);
            let mut sum = [0_u64; 4];
            let count = (block_right - block_x) as u64 * (block_bottom - block_y) as u64;
            for y in block_y..block_bottom {
                for x in block_x..block_right {
                    for (sum, channel) in sum.iter_mut().zip(image.get_pixel(x, y).0) {
                        *sum += channel as u64;
                    }
                }
            }
            let average = Rgba(sum.map(|channel| (channel / count) as u8));
            for y in block_y..block_bottom {
                for x in block_x..block_right {
                    image.put_pixel(x, y, average);
                }
            }
        }
    }
}

fn blur(image: &mut RgbaImage, rect: ImageRect, radius: f32) {
    let Some((left, top, right, bottom)) = rect.pixel_bounds(image.width(), image.height()) else {
        return;
    };
    let region = image::imageops::crop_imm(image, left, top, right - left, bottom - top).to_image();
    let blurred = image::imageops::blur(&region, radius);
    image::imageops::replace(image, &blurred, left as i64, top as i64);
}

fn sample(image: &RgbaImage, point: Point) -> Option<Color> {
    let x = point.x.floor() as i64;
    let y = point.y.floor() as i64;
    (x >= 0 && y >= 0 && x < image.width() as i64 && y < image.height() as i64)
        .then(|| Color::from(*image.get_pixel(x as u32, y as u32)))
}

fn average_border(image: &RgbaImage, rect: ImageRect) -> Color {
    let rect = rect.normalized();
    let points = [
        Point::new(rect.x - 1.0, rect.y - 1.0),
        Point::new(rect.x + rect.width, rect.y - 1.0),
        Point::new(rect.x - 1.0, rect.y + rect.height),
        Point::new(rect.x + rect.width, rect.y + rect.height),
    ];
    let colors: Vec<_> = points
        .into_iter()
        .filter_map(|point| sample(image, point))
        .collect();
    if colors.is_empty() {
        return Color::WHITE;
    }
    let count = colors.len() as u32;
    let (r, g, b, a) = colors.into_iter().fold((0, 0, 0, 0), |sum, color| {
        (
            sum.0 + color.r as u32,
            sum.1 + color.g as u32,
            sum.2 + color.b as u32,
            sum.3 + color.a as u32,
        )
    });
    Color::new(
        (r / count) as u8,
        (g / count) as u8,
        (b / count) as u8,
        (a / count) as u8,
    )
}

fn backdrop_is_identity(backdrop: Backdrop) -> bool {
    backdrop.padding == 0 && backdrop.corner_radius == 0 && backdrop.shadow.is_none()
}

fn apply_backdrop(source: &RgbaImage, backdrop: Backdrop) -> RgbaImage {
    let width = source
        .width()
        .saturating_add(backdrop.padding.saturating_mul(2));
    let height = source
        .height()
        .saturating_add(backdrop.padding.saturating_mul(2));
    let mut result = background(width, height, backdrop.background);
    if let Some(shadow) = backdrop.shadow {
        let layer = shadow_layer(width, height, source, backdrop, shadow);
        image::imageops::overlay(&mut result, &layer, 0, 0);
    }
    let mut layer = RgbaImage::new(width, height);
    let radius = backdrop
        .corner_radius
        .min(source.width().min(source.height()) / 2);
    for y in 0..source.height() {
        for x in 0..source.width() {
            if inside_rounded(x, y, source.width(), source.height(), radius) {
                layer.put_pixel(
                    x + backdrop.padding,
                    y + backdrop.padding,
                    *source.get_pixel(x, y),
                );
            }
        }
    }
    image::imageops::overlay(&mut result, &layer, 0, 0);
    result
}

fn background(width: u32, height: u32, background: Background) -> RgbaImage {
    match background {
        Background::Solid { color } => RgbaImage::from_pixel(width, height, color.into()),
        Background::LinearGradient {
            start,
            end,
            angle_degrees,
        } => {
            let angle = angle_degrees.to_radians();
            let direction = (angle.cos(), angle.sin());
            let extent = width as f32 * direction.0.abs() + height as f32 * direction.1.abs();
            RgbaImage::from_fn(width, height, |x, y| {
                let projection = (x as f32 - width as f32 * 0.5) * direction.0
                    + (y as f32 - height as f32 * 0.5) * direction.1;
                let t = if extent <= f32::EPSILON {
                    0.5
                } else {
                    (projection / extent + 0.5).clamp(0.0, 1.0)
                };
                Rgba([
                    lerp(start.r, end.r, t),
                    lerp(start.g, end.g, t),
                    lerp(start.b, end.b, t),
                    lerp(start.a, end.a, t),
                ])
            })
        }
    }
}

fn shadow_layer(
    width: u32,
    height: u32,
    source: &RgbaImage,
    backdrop: Backdrop,
    shadow: Shadow,
) -> RgbaImage {
    let mut layer = RgbaImage::new(width, height);
    let left = backdrop.padding as i64 + shadow.offset.x.round() as i64;
    let top = backdrop.padding as i64 + shadow.offset.y.round() as i64;
    let radius = backdrop
        .corner_radius
        .min(source.width().min(source.height()) / 2);
    for y in 0..source.height() {
        for x in 0..source.width() {
            let destination_x = left + x as i64;
            let destination_y = top + y as i64;
            if destination_x >= 0
                && destination_y >= 0
                && destination_x < width as i64
                && destination_y < height as i64
                && inside_rounded(x, y, source.width(), source.height(), radius)
            {
                layer.put_pixel(
                    destination_x as u32,
                    destination_y as u32,
                    shadow.color.into(),
                );
            }
        }
    }
    if shadow.blur_radius > 0.0 {
        image::imageops::blur(&layer, shadow.blur_radius)
    } else {
        layer
    }
}

fn inside_rounded(x: u32, y: u32, width: u32, height: u32, radius: u32) -> bool {
    if radius == 0 {
        return true;
    }
    let radius = radius as f32;
    let test_corner = |cx: f32, cy: f32| {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        dx * dx + dy * dy <= radius * radius
    };
    if x < radius as u32 && y < radius as u32 {
        test_corner(radius, radius)
    } else if x >= width - radius as u32 && y < radius as u32 {
        test_corner(width as f32 - radius, radius)
    } else if x < radius as u32 && y >= height - radius as u32 {
        test_corner(radius, height as f32 - radius)
    } else if x >= width - radius as u32 && y >= height - radius as u32 {
        test_corner(width as f32 - radius, height as f32 - radius)
    } else {
        true
    }
}

fn lerp(start: u8, end: u8, t: f32) -> u8 {
    (start as f32 + (end as f32 - start as f32) * t).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnnotationStyle;
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

    const TEST_FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSans.ttf");

    fn full_canvas_reference(
        document: &Document,
        options: &RenderOptions<'_>,
    ) -> Result<RgbaImage> {
        let font = match options.font_bytes {
            Some(bytes) => {
                Some(FontRef::try_from_slice(bytes).map_err(|_| SnipletError::InvalidFont)?)
            }
            None => None,
        };
        let mut canvas = RgbaImage::new(document.width(), document.height());
        image::imageops::overlay(&mut canvas, document.original(), 0, 0);
        let annotations: Vec<_> = document
            .annotations()
            .iter()
            .chain(options.extra_annotations.iter())
            .collect();
        // Keep the full-canvas allocation path independent of the optimized
        // renderer, with the same required base/image/shade/drawing order.
        for layer in [
            CompositeLayer::Raster,
            CompositeLayer::Image,
            CompositeLayer::Spotlight,
            CompositeLayer::Drawing,
        ] {
            if layer == CompositeLayer::Spotlight {
                legacy_draw_spotlights(&mut canvas, &annotations);
                continue;
            }
            for annotation in annotations
                .iter()
                .filter(|annotation| annotation.kind.composite_layer() == layer)
            {
                legacy_draw_annotation(
                    &mut canvas,
                    document.original(),
                    annotation,
                    font.as_ref(),
                )?;
            }
        }
        if options.apply_crop
            && let Some(crop) = document.crop()
            && let Some((left, top, right, bottom)) =
                crop.pixel_bounds(canvas.width(), canvas.height())
        {
            canvas = image::imageops::crop_imm(&canvas, left, top, right - left, bottom - top)
                .to_image();
        }
        if options.apply_backdrop {
            canvas = apply_backdrop(&canvas, document.backdrop());
        }
        Ok(canvas)
    }

    fn legacy_draw_annotation(
        canvas: &mut RgbaImage,
        source: &RgbaImage,
        annotation: &Annotation,
        font: Option<&FontRef<'_>>,
    ) -> Result<()> {
        match &annotation.kind {
            AnnotationKind::Measurement { measurement } => {
                let (bounds, overlay) =
                    measurement.overlay_with_font(font.ok_or(SnipletError::MissingFont)?)?;
                image::imageops::overlay(canvas, &overlay, bounds.x as i64, bounds.y as i64);
            }
            AnnotationKind::Pixelate { rect, block_size } => {
                pixelate(canvas, *rect, (*block_size).max(2));
            }
            AnnotationKind::Blur { rect, radius } => blur(canvas, *rect, (*radius).max(0.1)),
            AnnotationKind::RemoveFill {
                rect,
                sample: sampled_point,
            } => {
                let fill = sampled_point
                    .and_then(|point| sample(source, point))
                    .unwrap_or_else(|| average_border(source, *rect));
                draw_rect_fill(canvas, *rect, fill.into());
            }
            AnnotationKind::Image {
                origin,
                png_bytes,
                size,
            } => {
                let pasted = image::load_from_memory(png_bytes)?.to_rgba8();
                let pasted = match size {
                    Some(size) if size.width > 0 && size.height > 0 => image::imageops::resize(
                        &pasted,
                        size.width,
                        size.height,
                        image::imageops::FilterType::CatmullRom,
                    ),
                    _ => pasted,
                };
                image::imageops::overlay(
                    canvas,
                    &pasted,
                    origin.x.round() as i64,
                    origin.y.round() as i64,
                );
            }
            AnnotationKind::Magnifier { rect, zoom, .. } => {
                let mut layer = RgbaImage::new(canvas.width(), canvas.height());
                if annotation.kind.magnifier_circles().is_some() {
                    draw_linked_magnifier(&mut layer, source, annotation);
                } else {
                    draw_magnifier(&mut layer, source, *rect, *zoom, annotation);
                }
                image::imageops::overlay(canvas, &layer, 0, 0);
            }
            AnnotationKind::Spotlight { .. } => {}
            _ => {
                let mut layer = RgbaImage::new(canvas.width(), canvas.height());
                draw_layer_annotation(&mut layer, annotation, font)?;
                image::imageops::overlay(canvas, &layer, 0, 0);
            }
        }
        Ok(())
    }

    fn legacy_draw_spotlights(canvas: &mut RgbaImage, annotations: &[&Annotation]) {
        let spotlights: Vec<_> = annotations
            .iter()
            .filter_map(|annotation| match &annotation.kind {
                AnnotationKind::Spotlight { rect } => Some((rect.normalized(), annotation.style)),
                _ => None,
            })
            .collect();
        let Some((_, style)) = spotlights.last() else {
            return;
        };
        let dim: Rgba<u8> = style.fill.unwrap_or(Color::new(0, 0, 0, 150)).into();
        let mut layer = RgbaImage::new(canvas.width(), canvas.height());
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
                if !spotlights.iter().any(|(rect, _)| rect.contains(point)) {
                    layer.put_pixel(x, y, dim);
                }
            }
        }
        image::imageops::overlay(canvas, &layer, 0, 0);
    }

    fn reference_draw_thick_line(
        image: &mut RgbaImage,
        start: Point,
        end: Point,
        width: f32,
        color: Rgba<u8>,
    ) {
        if image.width() == 0
            || image.height() == 0
            || !start.x.is_finite()
            || !start.y.is_finite()
            || !end.x.is_finite()
            || !end.y.is_finite()
            || !width.is_finite()
            || color[3] == 0
        {
            return;
        }
        let radius = width.max(1.0) * 0.5;
        let antialias_edge = radius + 0.5;
        let left = (start.x.min(end.x) - antialias_edge).floor().max(0.0) as u32;
        let top = (start.y.min(end.y) - antialias_edge).floor().max(0.0) as u32;
        let right = (start.x.max(end.x) + antialias_edge)
            .ceil()
            .min(image.width() as f32) as u32;
        let bottom = (start.y.max(end.y) + antialias_edge)
            .ceil()
            .min(image.height() as f32) as u32;
        for y in top..bottom {
            for x in left..right {
                let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
                let distance = distance_to_segment(point, start, end);
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                let alpha = (color[3] as f32 * coverage).round() as u8;
                let pixel = image.get_pixel_mut(x, y);
                if alpha > pixel[3] {
                    *pixel = Rgba([color[0], color[1], color[2], alpha]);
                }
            }
        }
    }

    #[test]
    fn row_bounded_thick_lines_match_the_exhaustive_reference() {
        let fixed_cases = [
            (Point::new(4.5, 20.5), Point::new(118.5, 20.5), 3.0),
            (Point::new(63.5, 2.5), Point::new(63.5, 88.5), 7.0),
            (Point::new(3.25, 4.75), Point::new(121.5, 86.25), 5.5),
            (Point::new(121.5, 86.25), Point::new(3.25, 4.75), 5.5),
            (Point::new(42.5, 37.5), Point::new(42.5, 37.5), 11.0),
            (Point::new(-30.0, -8.0), Point::new(150.0, 105.0), 4.0),
            (Point::new(8.0, 82.0), Point::new(119.0, 5.0), 30.0),
        ];
        for (case, (start, end, width)) in fixed_cases.into_iter().enumerate() {
            let mut expected = RgbaImage::new(127, 91);
            let mut actual = expected.clone();
            let color = Rgba([29, 113, 227, 171]);
            reference_draw_thick_line(&mut expected, start, end, width, color);
            draw_thick_line(&mut actual, start, end, width, color);
            assert_eq!(actual, expected, "fixed line case {case} differed");
        }

        let mut seed = 0x8bad_f00d_u32;
        for case in 0..200 {
            let mut next = || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                seed
            };
            let coordinate =
                |value: u32, extent: f32| value as f32 / u32::MAX as f32 * (extent + 40.0) - 20.0;
            let start = Point::new(coordinate(next(), 127.0), coordinate(next(), 91.0));
            let end = Point::new(coordinate(next(), 127.0), coordinate(next(), 91.0));
            let width = 1.0 + (next() % 2_900) as f32 / 100.0;
            let color = Rgba([
                next() as u8,
                next() as u8,
                next() as u8,
                1 + (next() % 255) as u8,
            ]);
            let original = RgbaImage::from_fn(127, 91, |x, y| {
                let alpha = ((x * 17 + y * 31 + case) % 23) as u8;
                Rgba([11, 22, 33, alpha])
            });
            let mut expected = original.clone();
            let mut actual = original;
            reference_draw_thick_line(&mut expected, start, end, width, color);
            draw_thick_line(&mut actual, start, end, width, color);
            assert_eq!(actual, expected, "line case {case} differed");
        }
    }

    #[test]
    fn optimized_render_matches_the_full_canvas_reference_for_every_annotation_kind() {
        let mut seed = 0x1234_5678_u32;
        let source = RgbaImage::from_fn(96, 72, |x, y| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let alpha = [0, 37, 128, 255][((x + y * 3) % 4) as usize];
            Rgba([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, alpha])
        });
        let mut document = Document::new(source);
        document.expand_canvas_to(112, 84);
        let style = AnnotationStyle {
            stroke: Color::new(231, 48, 42, 173),
            fill: Some(Color::new(31, 127, 220, 91)),
            stroke_width: 5.5,
        };
        let kinds = [
            AnnotationKind::Measurement {
                measurement: Measurement {
                    start: Point::new(2.0, 2.0),
                    end: Point::new(38.0, 2.0),
                    pixels_per_unit: 2.0,
                    scale_factor: 2.0,
                },
            },
            AnnotationKind::Line {
                start: Point::new(2.25, 4.75),
                end: Point::new(83.5, 62.25),
            },
            AnnotationKind::Arrow {
                start: Point::new(8.5, 60.0),
                end: Point::new(88.25, 12.5),
                bend: Some(Point::new(46.5, 5.25)),
                variant: ArrowVariant::Solid,
            },
            AnnotationKind::Arrow {
                start: Point::new(90.0, 69.0),
                end: Point::new(22.0, 14.0),
                bend: Some(Point::new(72.0, 18.0)),
                variant: ArrowVariant::HandDrawn,
            },
            AnnotationKind::Arrow {
                start: Point::new(5.0, 35.0),
                end: Point::new(100.0, 36.0),
                bend: None,
                variant: ArrowVariant::Thin,
            },
            AnnotationKind::Arrow {
                start: Point::new(12.0, 18.0),
                end: Point::new(95.0, 55.0),
                bend: Some(Point::new(55.0, 70.0)),
                variant: ArrowVariant::DoubleEnded,
            },
            AnnotationKind::Rectangle {
                rect: ImageRect::new(5.2, 6.7, 31.4, 20.8),
            },
            AnnotationKind::Ellipse {
                rect: ImageRect::new(54.3, 39.4, 33.7, 24.2),
            },
            AnnotationKind::Text {
                origin: Point::new(4.4, 48.6),
                text: "Sniplet Åß".into(),
                font_size: 13.5,
            },
            AnnotationKind::Counter {
                center: Point::new(92.4, 18.7),
                value: 42,
                font_size: 15.5,
            },
            AnnotationKind::Pixelate {
                rect: ImageRect::new(9.2, 9.8, 19.4, 14.1),
                block_size: 5,
            },
            AnnotationKind::Blur {
                rect: ImageRect::new(30.6, 7.2, 22.3, 15.7),
                radius: 2.25,
            },
            AnnotationKind::Highlight {
                rect: ImageRect::new(15.2, 30.6, 42.2, 8.9),
            },
            AnnotationKind::Spotlight {
                rect: ImageRect::new(18.3, 10.7, 61.6, 49.2),
            },
            AnnotationKind::Freehand {
                points: vec![
                    Point::new(3.2, 70.1),
                    Point::new(18.7, 55.6),
                    Point::new(37.9, 68.4),
                    Point::new(61.3, 52.2),
                ],
            },
            AnnotationKind::Redaction {
                rect: ImageRect::new(65.5, 63.2, 24.1, 9.3),
            },
            AnnotationKind::RemoveFill {
                rect: ImageRect::new(74.4, 26.8, 15.9, 11.1),
                sample: Some(Point::new(2.0, 2.0)),
            },
            AnnotationKind::Magnifier {
                rect: ImageRect::new(39.2, 23.8, 25.7, 22.4),
                zoom: 2.5,
                source: None,
            },
            AnnotationKind::Magnifier {
                rect: ImageRect::new(76.0, -4.0, 40.0, 40.0),
                zoom: 3.0,
                source: Some(Point::new(4.0, 5.0)),
            },
        ];
        for kind in kinds {
            document.add_annotation(kind, style);
        }

        let pasted = RgbaImage::from_fn(3, 2, |x, y| {
            Rgba([20 + x as u8 * 30, 90 + y as u8 * 40, 210, 127])
        });
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(pasted.as_raw(), 3, 2, ExtendedColorType::Rgba8)
            .unwrap();
        document.add_annotation(
            AnnotationKind::Image {
                origin: Point::new(97.2, 67.6),
                png_bytes: png,
                size: Some(crate::ImageSize {
                    width: 11,
                    height: 8,
                }),
            },
            style,
        );
        document.set_crop(ImageRect::new(1.3, 2.7, 108.2, 78.1));
        document.set_backdrop(Backdrop {
            padding: 3,
            corner_radius: 5,
            background: Background::LinearGradient {
                start: Color::new(8, 16, 32, 255),
                end: Color::new(60, 20, 80, 255),
                angle_degrees: 31.0,
            },
            shadow: Some(Shadow {
                offset: Point::new(1.0, 2.0),
                blur_radius: 1.5,
                color: Color::new(0, 0, 0, 100),
            }),
        });
        let preview = Annotation {
            id: crate::AnnotationId(u64::MAX),
            kind: AnnotationKind::Ellipse {
                rect: ImageRect::new(79.7, 3.2, 25.1, 18.4),
            },
            style,
        };
        let options = RenderOptions {
            font_bytes: Some(TEST_FONT),
            apply_crop: true,
            apply_backdrop: true,
            extra_annotations: std::slice::from_ref(&preview),
        };

        let expected = full_canvas_reference(&document, &options).unwrap();
        let actual = document.render(&options).unwrap();
        assert_eq!(actual.dimensions(), expected.dimensions());
        for (x, y, pixel) in actual.enumerate_pixels() {
            assert_eq!(
                pixel,
                expected.get_pixel(x, y),
                "optimized output differed at ({x}, {y})"
            );
        }
    }

    #[test]
    fn diagonal_capsule_has_no_holes_correct_width_round_caps_and_union_alpha() {
        let start = Point::new(16.5, 16.5);
        let end = Point::new(49.5, 41.5);
        let width = 10.0;
        let radius = width * 0.5;
        let color = Rgba([231, 48, 42, 255]);
        let mut image = RgbaImage::new(68, 58);

        draw_thick_line(&mut image, start, end, width, color);

        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let length_squared = dx * dx + dy * dy;
        let mut opaque_interior = 0;
        let mut antialiased_edge = 0;
        let mut before_start = 0;
        let mut after_end = 0;
        let mut middle_max_distance = 0.0_f32;

        for (x, y, pixel) in image.enumerate_pixels() {
            let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
            let distance = distance_to_segment(point, start, end);
            let projection = ((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared;

            if distance <= radius - 0.5 {
                assert_eq!(*pixel, color, "interior hole at ({x}, {y})");
                opaque_interior += 1;
            }
            if pixel[3] > 0 {
                assert!(
                    distance < radius + 0.5,
                    "stroke exceeded its antialiased width at ({x}, {y})"
                );
                if (0.4..=0.6).contains(&projection) {
                    middle_max_distance = middle_max_distance.max(distance);
                }
                before_start += usize::from(projection < 0.0);
                after_end += usize::from(projection > 1.0);
            }
            antialiased_edge += usize::from(pixel[3] > 0 && pixel[3] < 255);
        }

        assert!(
            opaque_interior > 250,
            "diagonal interior was unexpectedly sparse"
        );
        assert!(antialiased_edge > 40, "stroke edge was not antialiased");
        assert!(
            (radius - 0.5..radius + 0.5).contains(&middle_max_distance),
            "rendered half-width was {middle_max_distance}, expected about {radius}"
        );
        assert!(before_start > 0 && after_end > 0, "round caps were missing");

        let translucent = Rgba([20, 120, 240, 128]);
        let mut overlapping = RgbaImage::new(68, 58);
        draw_thick_line(&mut overlapping, start, end, width, translucent);
        let once = overlapping.clone();
        draw_thick_line(&mut overlapping, start, end, width, translucent);
        assert_eq!(
            overlapping, once,
            "overlapping stroke geometry accumulated alpha"
        );
    }
}
