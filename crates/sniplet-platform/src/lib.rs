//! Cross-platform operating-system services used by Sniplet.
//!
//! This crate deliberately keeps capture, clipboard, OCR, QR decoding, settings,
//! and export policy outside the GPUI layer. The public image boundary is
//! [`image::RgbaImage`], which can be passed directly to
//! `sniplet_core::Document::new`.

pub const APP_ID: &str = "io.github.m4tta.sniplet";

mod capture;
mod clipboard;
mod error;
mod export;
mod ocr;
mod qr;
mod scroll;
mod settings;
mod upload;

pub use capture::{
    CaptureSource, CapturedFrame, MonitorInfo, ScreenPoint, WindowInfo, capture_active_window,
    capture_monitor, capture_monitor_region, capture_window, list_monitors, list_windows,
    window_at_point,
};
pub use clipboard::Clipboard;
pub use error::{PlatformError, Result};
pub use export::{default_export_directory, export_image, sanitize_filename, unique_export_path};
pub use image::RgbaImage;
#[cfg(windows)]
pub use ocr::recognize_text_native_windows;
pub use ocr::{OcrOptions, recognize_text, recognize_text_tesseract};
pub use qr::{QrCode, QrPoint, scan_qr_codes};
pub use scroll::{
    AutomaticScrollCapture, AutomaticScrollOptions, AutomaticScrollStop, ScrollCancellation,
    ScrollCaptureTarget, ScrollSession, capture_scrolling_region,
};
pub use settings::{
    AnnotationColor, ExportFormat, HotkeySettings, Settings, SettingsStore, ThemePreference,
};
pub use upload::{CloudUploadConfig, UploadedImage, upload_image};
