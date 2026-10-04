use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("could not enumerate {kind}: {source}")]
    Enumeration {
        kind: &'static str,
        #[source]
        source: Box<xcap::XCapError>,
    },

    #[error("could not read {kind} property `{property}`: {source}")]
    CaptureProperty {
        kind: &'static str,
        property: &'static str,
        #[source]
        source: Box<xcap::XCapError>,
    },

    #[error(
        "screen capture backend is unavailable while trying to {operation}: {reason}. On Linux, use a compositor with a supported X11 or Wayland capture protocol"
    )]
    CaptureBackendUnavailable {
        operation: &'static str,
        reason: String,
    },

    #[error("monitor index {index} does not exist (found {available} monitors)")]
    MonitorNotFound { index: usize, available: usize },

    #[error(
        "capture region ({x}, {y}, {width}, {height}) is outside the {monitor_width}x{monitor_height} monitor"
    )]
    InvalidCaptureRegion {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        monitor_width: u32,
        monitor_height: u32,
    },

    #[error("window id {id} does not exist")]
    WindowNotFound { id: u32 },

    #[error("no focused window is available for capture")]
    ActiveWindowNotFound,

    #[error("window id {id} is minimized and cannot be captured")]
    WindowMinimized { id: u32 },

    #[error("screen capture failed: {0}")]
    Capture(#[source] Box<xcap::XCapError>),

    #[error("scrolling capture could not be stitched: {0}")]
    Stitch(#[source] sniplet_core::SnipletError),

    #[error("automatic scrolling options are invalid: {0}")]
    InvalidScrollOptions(&'static str),

    #[error("could not initialize automatic input: {0}")]
    InputInitialization(#[source] enigo::NewConError),

    #[error("could not send automatic input: {0}")]
    Input(#[source] enigo::InputError),

    #[error("clipboard initialization failed: {0}")]
    ClipboardUnavailable(#[source] arboard::Error),

    #[error("clipboard operation failed: {0}")]
    Clipboard(#[source] arboard::Error),

    #[error("invalid RGBA buffer: expected {expected} bytes, received {actual}")]
    InvalidRgbaBuffer { expected: usize, actual: usize },

    #[error("Tesseract executable `{executable}` was not found")]
    TesseractMissing { executable: PathBuf },

    #[error("Tesseract failed with exit code {status:?}: {stderr}")]
    OcrFailed { status: Option<i32>, stderr: String },

    #[cfg(windows)]
    #[error("Windows OCR failed while trying to {operation}: {source}")]
    WindowsOcr {
        operation: &'static str,
        #[source]
        source: windows::core::Error,
    },

    #[error("QR code decoding failed: {0}")]
    QrDecode(#[source] rqrr::DeQRError),

    #[error("Sniplet's settings directory is not available on this operating system")]
    SettingsDirectoryUnavailable,

    #[error("the user's home directory is not available on this operating system")]
    UserDirectoryUnavailable,

    #[error("could not read `{path}`: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not write `{path}`: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("settings in `{path}` are invalid: {source}")]
    InvalidSettings {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("could not encode settings: {0}")]
    EncodeSettings(#[source] serde_json::Error),

    #[error("could not encode image: {0}")]
    Image(#[source] image::ImageError),

    #[error("cloud upload configuration is invalid: {0}")]
    InvalidCloudUploadConfig(String),

    #[error("the upload server returned HTTP {status}")]
    UploadHttpStatus { status: u16 },

    #[error("the upload request could not reach the configured server")]
    UploadTransport,

    #[error("S3 upload failed: {0}")]
    S3(#[source] Box<s3::Error>),

    #[error("export destination must be a directory: `{0}`")]
    InvalidExportDirectory(PathBuf),

    #[error("could not allocate a unique filename in `{0}`")]
    ExportNameExhausted(PathBuf),
}

pub type Result<T> = std::result::Result<T, PlatformError>;
