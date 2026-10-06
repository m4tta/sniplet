use std::path::{Path, PathBuf};

use directories::UserDirs;
use image::{DynamicImage, RgbaImage};

use crate::{ExportFormat, PlatformError, Result};

pub fn default_export_directory() -> Result<PathBuf> {
    let user = UserDirs::new().ok_or(PlatformError::UserDirectoryUnavailable)?;
    let base = user.picture_dir().unwrap_or_else(|| user.home_dir());
    Ok(base.join("Sniplet"))
}

/// Saving at 1x leaves the editable source and clipboard image at full resolution.
pub fn image_at_1x(image: RgbaImage, scale: f32) -> RgbaImage {
    if !scale.is_finite() || scale <= 1.0 {
        return image;
    }
    let width = (image.width() as f32 / scale).round().max(1.0) as u32;
    let height = (image.height() as f32 / scale).round().max(1.0) as u32;
    image::imageops::resize(&image, width, height, image::imageops::FilterType::Lanczos3)
}

/// Removes path separators, control characters, reserved Windows characters,
/// traversal components, and problematic trailing characters from a filename stem.
pub fn sanitize_filename(input: &str) -> String {
    let mut result = String::with_capacity(input.len().min(120));
    let mut previous_was_space = false;
    for character in input.chars().take(120) {
        let safe = !character.is_control()
            && !matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            );
        if !safe {
            continue;
        }
        if character.is_whitespace() {
            if !previous_was_space && !result.is_empty() {
                result.push(' ');
            }
            previous_was_space = true;
        } else {
            result.push(character);
            previous_was_space = false;
        }
    }
    let result = result.trim_matches([' ', '.']).to_owned();
    if result.is_empty() || result == "." || result == ".." || is_windows_reserved(&result) {
        "Sniplet Screenshot".to_owned()
    } else {
        result
    }
}

fn is_windows_reserved(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || base
            .strip_prefix("COM")
            .or_else(|| base.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

/// Returns a currently unused path under `directory`; it never accepts a path
/// component from `stem`, so user-entered names cannot escape the destination.
pub fn unique_export_path(directory: &Path, stem: &str, format: ExportFormat) -> Result<PathBuf> {
    if directory.exists() && !directory.is_dir() {
        return Err(PlatformError::InvalidExportDirectory(directory.to_owned()));
    }
    let stem = sanitize_filename(stem);
    for suffix in 0..10_000_u32 {
        let filename = if suffix == 0 {
            format!("{stem}.{}", format.extension())
        } else {
            format!("{stem} {suffix}.{}", format.extension())
        };
        let path = directory.join(filename);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(PlatformError::ExportNameExhausted(directory.to_owned()))
}

pub fn export_image(image: &RgbaImage, path: &Path, format: ExportFormat) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| PlatformError::WriteFile {
        path: parent.to_owned(),
        source,
    })?;
    let dynamic = match format {
        ExportFormat::Jpeg => {
            DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image.clone()).into_rgb8())
        }
        ExportFormat::Png | ExportFormat::Webp => DynamicImage::ImageRgba8(image.clone()),
    };
    let image_format = match format {
        ExportFormat::Png => image::ImageFormat::Png,
        ExportFormat::Jpeg => image::ImageFormat::Jpeg,
        ExportFormat::Webp => image::ImageFormat::WebP,
    };
    dynamic
        .save_with_format(path, image_format)
        .map_err(PlatformError::Image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_at_1x_uses_the_capture_scale_and_preserves_standard_images() {
        let image = RgbaImage::from_pixel(600, 400, image::Rgba([20, 40, 60, 255]));
        assert_eq!(image_at_1x(image.clone(), 2.0).dimensions(), (300, 200));
        assert_eq!(image_at_1x(image.clone(), 1.5).dimensions(), (400, 267));
        for scale in [1.0, 0.0, f32::NAN, f32::INFINITY] {
            assert_eq!(image_at_1x(image.clone(), scale), image);
        }
    }

    #[test]
    fn sanitizes_untrusted_filename_stems() {
        assert_eq!(
            sanitize_filename(r#"  report: <final>/..\\?.  "#),
            "report final"
        );
        assert_eq!(sanitize_filename("../"), "Sniplet Screenshot");
        assert_eq!(sanitize_filename("CON"), "Sniplet Screenshot");
        assert_eq!(sanitize_filename("a\t  b"), "a b");
    }

    #[test]
    fn unique_path_stays_in_the_requested_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = unique_export_path(directory.path(), "../../escape", ExportFormat::Png).unwrap();
        assert_eq!(path.parent(), Some(directory.path()));
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("png")
        );
    }

    #[test]
    fn unique_path_does_not_overwrite_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("Shot.png"), []).unwrap();
        let path = unique_export_path(directory.path(), "Shot", ExportFormat::Png).unwrap();
        assert_eq!(
            path.file_name().and_then(|value| value.to_str()),
            Some("Shot 1.png")
        );
    }
}
