use std::path::{Path, PathBuf};
use std::process::Command;

use image::RgbaImage;

use crate::{PlatformError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrOptions {
    pub executable: PathBuf,
    pub language: String,
    pub page_segmentation_mode: u8,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("tesseract"),
            language: "eng".to_owned(),
            page_segmentation_mode: 6,
        }
    }
}

/// Recognizes text entirely offline. Windows uses its built-in OCR engine first
/// and falls back to the configured Tesseract executable if WinRT OCR fails. When
/// Tesseract is missing too, the Windows error is the one that explains the failure.
/// macOS and Linux use Tesseract directly.
pub fn recognize_text(image: &RgbaImage, options: &OcrOptions) -> Result<String> {
    #[cfg(windows)]
    let native = match recognize_text_native_windows(image, None) {
        Ok(text) => return Ok(text),
        Err(error) => Some(error),
    };
    #[cfg(not(windows))]
    let native = None;
    match (recognize_text_tesseract(image, options), native) {
        (Err(PlatformError::TesseractMissing { .. }), Some(native)) => Err(native),
        (result, _) => result,
    }
}

/// Runs the installed Tesseract command locally. No image or text leaves the machine.
pub fn recognize_text_tesseract(image: &RgbaImage, options: &OcrOptions) -> Result<String> {
    let input = tempfile::Builder::new()
        .prefix("sniplet-ocr-")
        .suffix(".png")
        .tempfile()
        .map_err(|source| PlatformError::WriteFile {
            path: std::env::temp_dir(),
            source,
        })?;
    save_ocr_input(image, input.path())?;

    let output = Command::new(&options.executable)
        .arg(input.path())
        .arg("stdout")
        .arg("-l")
        .arg(&options.language)
        .arg("--psm")
        .arg(options.page_segmentation_mode.to_string())
        .output()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                PlatformError::TesseractMissing {
                    executable: options.executable.clone(),
                }
            } else {
                PlatformError::ReadFile {
                    path: options.executable.clone(),
                    source,
                }
            }
        })?;

    if !output.status.success() {
        return Err(PlatformError::OcrFailed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

fn save_ocr_input(image: &RgbaImage, path: &Path) -> Result<()> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(PlatformError::Image)
}

/// Uses the OCR engine included with Windows 10 and later. The bitmap remains
/// in memory and recognition is performed entirely by the operating system.
#[cfg(windows)]
pub fn recognize_text_native_windows(
    image: &RgbaImage,
    language_tag: Option<&str>,
) -> Result<String> {
    use std::sync::Mutex;

    use windows::{
        Globalization::Language,
        Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
        core::HSTRING,
    };

    fn windows_ocr_error(
        operation: &'static str,
    ) -> impl FnOnce(windows::core::Error) -> PlatformError {
        move |source| PlatformError::WindowsOcr { operation, source }
    }

    /// The engine for the user's languages, made once.
    static USER_ENGINE: Mutex<Option<OcrEngine>> = Mutex::new(None);

    // The bitmap takes the pixels directly, as BGRA.
    let mut bgra = image.as_raw()[..image.width() as usize * image.height() as usize * 4].to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(windows_ocr_error("create an image writer"))?;
    writer
        .WriteBytes(&bgra)
        .map_err(windows_ocr_error("write the image"))?;
    let buffer = writer
        .DetachBuffer()
        .map_err(windows_ocr_error("store the image"))?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        image.width() as i32,
        image.height() as i32,
    )
    .map_err(windows_ocr_error("read image pixels"))?;
    let engine = if let Some(language_tag) = language_tag {
        let language = Language::CreateLanguage(&HSTRING::from(language_tag))
            .map_err(windows_ocr_error("select the OCR language"))?;
        OcrEngine::TryCreateFromLanguage(&language)
            .map_err(windows_ocr_error("create the OCR engine"))?
    } else {
        let mut cached = USER_ENGINE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match (*cached).clone() {
            Some(engine) => engine,
            None => {
                let engine = OcrEngine::TryCreateFromUserProfileLanguages()
                    .map_err(windows_ocr_error("create the OCR engine"))?;
                *cached = Some(engine.clone());
                engine
            }
        }
    };
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(windows_ocr_error("start recognition"))?
        .join()
        .map_err(windows_ocr_error("recognize text"))?;
    result
        .Text()
        .map(|text| text.to_string_lossy())
        .map_err(windows_ocr_error("read recognized text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tesseract_has_specific_error() {
        let options = OcrOptions {
            executable: PathBuf::from("sniplet-this-executable-does-not-exist"),
            ..OcrOptions::default()
        };
        let error = recognize_text_tesseract(&RgbaImage::new(1, 1), &options).unwrap_err();
        assert!(matches!(error, PlatformError::TesseractMissing { .. }));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an installed Windows OCR language pack"]
    fn native_windows_ocr_reads_rendered_text() {
        use image::Rgba;
        use sniplet_core::{
            AnnotationKind, AnnotationStyle, Color, Document, Point, RenderOptions,
        };

        let mut document =
            Document::new(RgbaImage::from_pixel(900, 180, Rgba([255, 255, 255, 255])));
        document.add_annotation(
            AnnotationKind::Text {
                origin: Point::new(24.0, 35.0),
                text: "SNIPLET OCR TEST 7391".to_owned(),
                font_size: 64.0,
            },
            AnnotationStyle {
                stroke: Color::BLACK,
                ..AnnotationStyle::default()
            },
        );
        let image = document
            .render(&RenderOptions {
                font_bytes: Some(include_bytes!("../../../assets/fonts/NotoSans.ttf")),
                ..RenderOptions::default()
            })
            .unwrap();
        let options = OcrOptions {
            executable: PathBuf::from("sniplet-tesseract-must-not-run"),
            ..OcrOptions::default()
        };
        let text = recognize_text(&image, &options).unwrap();
        assert!(
            text.to_ascii_uppercase().contains("SNIPLET OCR TEST"),
            "{text:?}"
        );
        assert!(text.contains("7391"), "{text:?}");
    }
}
