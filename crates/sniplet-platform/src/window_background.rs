use std::path::{Path, PathBuf};

use image::{GrayImage, Luma, Rgba, RgbaImage, imageops};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowBackground {
    Wallpaper,
    #[default]
    Transparent,
    Solid,
    TrimShadow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowCaptureStyle {
    pub background: WindowBackground,
    /// Padding in logical pixels; multiplied by the capture's display scale.
    pub padding: u32,
    pub color: [u8; 3],
    /// None uses the desktop wallpaper, without capturing other windows.
    pub wallpaper: Option<PathBuf>,
}

impl Default for WindowCaptureStyle {
    fn default() -> Self {
        Self {
            background: WindowBackground::Transparent,
            padding: 32,
            color: [242, 242, 247],
            wallpaper: None,
        }
    }
}

/// Compose a tight native window capture once, before loading the editor. This
/// keeps its pixels sharp and uses the same result for editing, copy and save.
/// xcap captures the window bounds without the desktop's external shadow; trim
/// mode keeps those bounds unchanged. Other modes add a soft alpha-mask shadow.
pub fn compose_window_capture(
    source: RgbaImage,
    style: &WindowCaptureStyle,
    scale_factor: f32,
    wallpaper: Option<&RgbaImage>,
) -> RgbaImage {
    if style.background == WindowBackground::TrimShadow || source.is_empty() {
        return source;
    }
    let scale = if scale_factor.is_finite() {
        scale_factor.clamp(1.0, 8.0)
    } else {
        1.0
    };
    let padding = (style.padding.min(120) as f32 * scale).round() as u32;
    let width = source.width() + padding * 2;
    let height = source.height() + padding * 2;
    let color = Rgba([style.color[0], style.color[1], style.color[2], 255]);
    let mut canvas = match style.background {
        WindowBackground::Transparent => RgbaImage::new(width, height),
        WindowBackground::Wallpaper => match wallpaper.filter(|image| !image.is_empty()) {
            Some(image) => wallpaper_cover(image, width, height),
            None => RgbaImage::from_pixel(width, height, color),
        },
        WindowBackground::Solid => RgbaImage::from_pixel(width, height, color),
        WindowBackground::TrimShadow => unreachable!(),
    };
    if padding > 0 {
        // Blur a single channel with a linear-time box approximation. No
        // window pixels are filtered, and this work runs off the UI thread.
        let mut mask = GrayImage::new(width, height);
        let offset = padding + (4.0 * scale).round() as u32;
        for (x, y, pixel) in source.enumerate_pixels() {
            if y + offset < height {
                mask.put_pixel(x + padding, y + offset, Luma([pixel[3]]));
            }
        }
        let mask = imageops::fast_blur(&mask, 8.0 * scale);
        let shadow = RgbaImage::from_fn(width, height, |x, y| {
            Rgba([
                0,
                0,
                0,
                (u16::from(mask.get_pixel(x, y)[0]) * 72 / 255) as u8,
            ])
        });
        imageops::overlay(&mut canvas, &shadow, 0, 0);
    }
    imageops::overlay(&mut canvas, &source, i64::from(padding), i64::from(padding));
    canvas
}

fn wallpaper_cover(source: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    // Crop before resizing so a very wide/tall wallpaper doesn't allocate an
    // oversized intermediate image. Wallpaper is fitted to the output frame.
    let factor = (width as f64 / source.width() as f64).max(height as f64 / source.height() as f64);
    let crop_width = ((width as f64 / factor).round() as u32).clamp(1, source.width());
    let crop_height = ((height as f64 / factor).round() as u32).clamp(1, source.height());
    let crop = imageops::crop_imm(
        source,
        (source.width() - crop_width) / 2,
        (source.height() - crop_height) / 2,
        crop_width,
        crop_height,
    );
    imageops::resize(
        &crop.to_image(),
        width,
        height,
        imageops::FilterType::Triangle,
    )
}

pub(crate) fn load_wallpaper(style: &WindowCaptureStyle) -> Result<RgbaImage, String> {
    let path = match &style.wallpaper {
        Some(path) => path.clone(),
        None => desktop_wallpaper()?,
    };
    load_wallpaper_image(&path)
}

#[cfg(windows)]
fn desktop_wallpaper() -> Result<PathBuf, String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut buffer = vec![0_u16; 32768];
    // SPI_GETDESKWALLPAPER writes a file path; it doesn't change desktop state.
    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|error| error.to_string())?;
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    if length == 0 {
        return Err("The desktop has no wallpaper image".into());
    }
    use std::os::windows::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}

#[cfg(not(windows))]
fn desktop_wallpaper() -> Result<PathBuf, String> {
    let path = wallpaper::get().map_err(|error| error.to_string())?;
    if path.trim().is_empty() {
        return Err("The desktop has no wallpaper image".into());
    }
    if path.starts_with("file:") {
        return url::Url::parse(&path)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .ok_or_else(|| "The desktop wallpaper path is invalid".into());
    }
    Ok(PathBuf::from(path))
}

pub fn load_wallpaper_image(path: &Path) -> Result<RgbaImage, String> {
    match image::open(path) {
        Ok(image) => Ok(image.to_rgba8()),
        Err(error) => {
            // macOS desktop wallpapers are often HEIC. Use the OS decoder for
            // formats not supported by image, without changing the wallpaper.
            #[cfg(target_os = "macos")]
            {
                let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
                let output_path = directory.path().join("wallpaper.png");
                let output = std::process::Command::new("/usr/bin/sips")
                    .args(["-s", "format", "png"])
                    .arg(path)
                    .arg("--out")
                    .arg(&output_path)
                    .output()
                    .map_err(|error| error.to_string())?;
                if output.status.success() {
                    return image::open(output_path)
                        .map(|image| image.to_rgba8())
                        .map_err(|error| error.to_string());
                }
            }
            Err(format!("Could not read wallpaper: {error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_padding_preserves_window_pixels_and_has_a_soft_shadow() {
        let source = RgbaImage::from_pixel(80, 60, Rgba([210, 90, 40, 255]));
        let result = compose_window_capture(source.clone(), &Default::default(), 1.0, None);
        assert_eq!(result.dimensions(), (144, 124));
        assert_eq!(
            imageops::crop_imm(&result, 32, 32, 80, 60).to_image(),
            source
        );
        assert_eq!(result.get_pixel(0, 0)[3], 0);
        let near = result.get_pixel(30, 60)[3];
        let far = result.get_pixel(12, 60)[3];
        assert!(near > far && near > 0 && near < 72);
    }

    #[test]
    fn solid_and_wallpaper_fill_the_padding_and_preserve_content() {
        let source = RgbaImage::from_pixel(40, 30, Rgba([230, 40, 80, 255]));
        let mut style = WindowCaptureStyle {
            background: WindowBackground::Solid,
            color: [25, 40, 60],
            ..Default::default()
        };
        let result = compose_window_capture(source.clone(), &style, 1.0, None);
        assert_eq!(*result.get_pixel(0, 0), Rgba([25, 40, 60, 255]));
        assert!(result.pixels().all(|pixel| pixel[3] == 255));
        let wallpaper = RgbaImage::from_fn(300, 100, |x, _| {
            if (80..220).contains(&x) {
                Rgba([20, 180, 140, 255])
            } else {
                Rgba([255, 0, 0, 255])
            }
        });
        style.background = WindowBackground::Wallpaper;
        let result = compose_window_capture(source.clone(), &style, 1.0, Some(&wallpaper));
        assert_eq!(*result.get_pixel(0, 0), Rgba([20, 180, 140, 255]));
        assert_eq!(
            imageops::crop_imm(&result, 32, 32, 40, 30).to_image(),
            source
        );
        assert_eq!(
            compose_window_capture(source, &style, 1.0, None).get_pixel(0, 0)[0],
            25
        );
    }

    #[test]
    fn trim_is_exact_and_padding_scales_with_retina_without_resampling_content() {
        let source = RgbaImage::from_pixel(20, 10, Rgba([20, 40, 80, 255]));
        let style = WindowCaptureStyle {
            background: WindowBackground::TrimShadow,
            ..Default::default()
        };
        assert_eq!(
            compose_window_capture(source.clone(), &style, 2.0, None),
            source
        );
        let style = WindowCaptureStyle::default();
        assert_eq!(
            compose_window_capture(source.clone(), &style, 2.0, None).dimensions(),
            (148, 138)
        );
        for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(
                compose_window_capture(source.clone(), &style, scale, None).dimensions(),
                (84, 74)
            );
        }
        let style = WindowCaptureStyle {
            padding: 0,
            ..Default::default()
        };
        assert_eq!(
            compose_window_capture(source.clone(), &style, 1.0, None),
            source
        );
        assert_eq!(
            compose_window_capture(RgbaImage::new(0, 0), &style, 1.0, None).dimensions(),
            (0, 0)
        );
    }

    #[test]
    fn transparent_corners_and_png_alpha_survive_compositing_and_export() {
        let mut source = RgbaImage::from_pixel(20, 20, Rgba([220, 230, 240, 255]));
        source.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        let result = compose_window_capture(source, &Default::default(), 1.0, None);
        assert!(result.get_pixel(32, 32)[3] < 255);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("transparent.png");
        result.save(&path).unwrap();
        assert_eq!(image::open(path).unwrap().to_rgba8(), result);
    }

    #[test]
    fn custom_wallpaper_reads_files_and_reports_missing_images() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wallpaper.png");
        let image = RgbaImage::from_pixel(2, 3, Rgba([70, 120, 180, 255]));
        image.save(&path).unwrap();
        let style = WindowCaptureStyle {
            wallpaper: Some(path),
            ..Default::default()
        };
        assert_eq!(load_wallpaper(&style).unwrap(), image);
        assert!(load_wallpaper_image(&directory.path().join("missing.png")).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a visible Sniplet settings window and a desktop wallpaper"]
    fn native_window_backgrounds_capture_and_export_all_four_modes() {
        let window = crate::list_windows()
            .unwrap()
            .into_iter()
            .find(|window| window.title == "Sniplet Settings" && !window.is_minimized)
            .expect("Open Sniplet with --demo --settings --normal-window before this test");
        let raw = crate::capture_window(window.id).unwrap();
        let output = std::env::var_os("SNIPLET_BACKGROUND_TEST_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("sniplet-window-background-test"));
        std::fs::create_dir_all(&output).unwrap();
        raw.image.save(output.join("window-original.png")).unwrap();
        for (name, background) in [
            ("wallpaper", WindowBackground::Wallpaper),
            ("transparent", WindowBackground::Transparent),
            ("solid", WindowBackground::Solid),
            ("trim", WindowBackground::TrimShadow),
        ] {
            let style = WindowCaptureStyle {
                background,
                ..Default::default()
            };
            let started = std::time::Instant::now();
            let (frame, warning) =
                crate::capture_window_with_background(window.id, &style).unwrap();
            assert!(warning.is_none(), "{warning:?}");
            let padding = if background == WindowBackground::TrimShadow {
                0
            } else {
                (32.0 * frame.scale_factor.clamp(1.0, 8.0)).round() as u32
            };
            assert_eq!(
                frame.image.dimensions(),
                (
                    raw.image.width() + padding * 2,
                    raw.image.height() + padding * 2
                )
            );
            assert_eq!(frame.source, raw.source);
            if background == WindowBackground::Transparent {
                assert_eq!(frame.image.get_pixel(0, 0)[3], 0);
                assert!(
                    frame
                        .image
                        .pixels()
                        .any(|pixel| pixel[3] > 0 && pixel[3] < 255)
                );
            } else if background != WindowBackground::TrimShadow {
                assert!(frame.image.pixels().all(|pixel| pixel[3] == 255));
            }
            let file = output.join(format!("window-{name}.png"));
            frame.image.save(&file).unwrap();
            assert_eq!(image::open(&file).unwrap().to_rgba8(), frame.image);
            eprintln!(
                "{name}: {:?}, capture/compose/export {:?}",
                frame.image.dimensions(),
                started.elapsed()
            );
        }
    }
}
