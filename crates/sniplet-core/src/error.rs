use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SnipletError {
    #[error("image operation failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("I/O operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("project JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project version {actual} is unsupported; expected version {expected}")]
    UnsupportedProjectVersion { expected: u32, actual: u32 },
    #[error("annotation {0} does not exist")]
    UnknownAnnotation(u64),
    #[error("annotation bounds must have finite coordinates and positive dimensions")]
    InvalidAnnotationBounds,
    #[error("an edit group is already open")]
    EditGroupAlreadyOpen,
    #[error("there is no open edit group")]
    NoEditGroup,
    #[error("text rendering needs font bytes supplied by the caller")]
    MissingFont,
    #[error("the supplied font data is invalid")]
    InvalidFont,
    #[error(
        "project expects a {expected_width}x{expected_height} image, but {path:?} is {actual_width}x{actual_height}"
    )]
    ProjectImageMismatch {
        path: PathBuf,
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    #[error("stitching needs at least one frame")]
    NoStitchFrames,
    #[error("frame {index} has width {actual}; expected {expected}")]
    StitchWidthMismatch {
        index: usize,
        expected: u32,
        actual: u32,
    },
    #[error("frame {index} has no acceptable vertical overlap (best error {best_error:.2})")]
    StitchOverlapNotFound { index: usize, best_error: f32 },
    #[error("JPEG quality must be in 1..=100")]
    InvalidJpegQuality,
}

pub type Result<T> = std::result::Result<T, SnipletError>;
