//! Cross-platform screenshot document and raster editing engine for Sniplet.
//!
//! [`Document`] never mutates its source pixels. Cropping and annotations are
//! retained as an editable project and composited only by [`Document::render`].

mod annotation;
mod color;
mod demo;
mod document;
mod error;
mod export;
mod geometry;
mod measurement;
mod render;
mod stitch;
mod transform;

pub use annotation::{
    Annotation, AnnotationId, AnnotationKind, AnnotationStyle, ArrowVariant, MagnifierPart,
};
pub use color::{Color, ColorFormat};
pub use demo::demo_image;
pub use document::{Backdrop, Background, Document, ImageSize, Project, Shadow};
pub use error::{Result, SnipletError};
pub use export::{ExportFormat, decode_image};
pub use geometry::{ImageRect, Point};
pub use measurement::{Measurement, MeasurementAxis, measure_at};
pub use render::RenderOptions;
pub use stitch::{StitchOptions, stitch_vertical};
pub use transform::ViewportTransform;
