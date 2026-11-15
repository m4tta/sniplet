//! Cross-platform screenshot document and raster editing engine for Sniplet.
//!
//! [`Document`] never mutates its source pixels. Cropping and annotations are
//! retained as an editable project and composited only by [`Scene::render`]. A
//! document derefs to its [`Scene`], and [`Document::snapshot`] hands one to a worker.

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
pub use document::{Backdrop, Background, Document, ImageSize, Project, Scene, Shadow};
pub use error::{Result, SnipletError};
pub use export::{ExportFormat, decode_image, open_image};
pub use geometry::{ImageRect, Point};
pub use measurement::{Measurement, MeasurementAxis, measure_at};
pub use render::{FONT, RenderOptions};
pub use stitch::{StitchOptions, Stitcher, stitch_vertical, stitch_vertical_refs};
pub use transform::ViewportTransform;
