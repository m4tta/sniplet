use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use directories::UserDirs;
use image::{RgbImage, RgbaImage};

use crate::{ExportFormat, PlatformError, Result};

pub fn default_export_directory() -> Result<PathBuf> {
    let user = UserDirs::new().ok_or(PlatformError::UserDirectoryUnavailable)?;
    let base = user.picture_dir().unwrap_or_else(|| user.home_dir());
    Ok(base.join("Sniplet"))
}

/// Saving at 1x leaves the editable source and clipboard image at full resolution. An image
/// already at 1x is borrowed, not copied.
pub fn image_at_1x(image: &RgbaImage, scale: f32) -> Cow<'_, RgbaImage> {
    if !scale.is_finite() || scale <= 1.0 {
        return Cow::Borrowed(image);
    }
    let width = (image.width() as f32 / scale).round().max(1.0) as u32;
    let height = (image.height() as f32 / scale).round().max(1.0) as u32;
    let filter = image::imageops::FilterType::Lanczos3;
    Cow::Owned(image::imageops::resize(image, width, height, filter))
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

pub fn export_image(image: &RgbaImage, path: &Path, format: ExportFormat) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| PlatformError::WriteFile {
        path: parent.to_owned(),
        source,
    })?;
    let saved = match format {
        // JPEG has no alpha: drop it, as `DynamicImage::into_rgb8` does.
        ExportFormat::Jpeg => {
            let rgb = image
                .as_raw()
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[red, green, blue, _]| [red, green, blue])
                .collect();
            RgbImage::from_raw(image.width(), image.height(), rgb)
                .expect("three channels for every pixel")
                .save_with_format(path, image::ImageFormat::Jpeg)
        }
        ExportFormat::Png => image.save_with_format(path, image::ImageFormat::Png),
        ExportFormat::Webp => image.save_with_format(path, image::ImageFormat::WebP),
    };
    saved.map_err(PlatformError::Image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_at_1x_uses_the_capture_scale_and_preserves_standard_images() {
        let image = RgbaImage::from_pixel(600, 400, image::Rgba([20, 40, 60, 255]));
        assert_eq!(image_at_1x(&image, 2.0).dimensions(), (300, 200));
        assert_eq!(image_at_1x(&image, 1.5).dimensions(), (400, 267));
        for scale in [1.0, 0.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(image_at_1x(&image, scale), Cow::Borrowed(same) if *same == image));
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
}
