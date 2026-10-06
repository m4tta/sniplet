use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::{CloudUploadConfig, PlatformError, Result, WindowCaptureStyle};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
}

impl ExportFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Default for AnnotationColor {
    fn default() -> Self {
        Self {
            red: 255,
            green: 59,
            blue: 48,
            alpha: 255,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySettings {
    pub capture_area: String,
    pub capture_screen: String,
    pub capture_window: String,
    pub scrolling_capture: String,
    pub repeat_area: String,
    pub active_window: String,
    pub capture_ocr: String,
    pub show_editor: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            capture_area: "CommandOrControl+Shift+2".to_owned(),
            capture_screen: "CommandOrControl+Shift+1".to_owned(),
            capture_window: "CommandOrControl+Shift+3".to_owned(),
            scrolling_capture: "CommandOrControl+Shift+4".to_owned(),
            repeat_area: "CommandOrControl+Shift+5".to_owned(),
            active_window: "CommandOrControl+Shift+6".to_owned(),
            capture_ocr: "CommandOrControl+Shift+7".to_owned(),
            show_editor: "CommandOrControl+Shift+8".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkeys: HotkeySettings,
    pub theme: ThemePreference,
    pub format: ExportFormat,
    pub auto_copy: bool,
    pub hide_after_export: bool,
    pub always_on_top: bool,
    pub screenshot_directory: Option<PathBuf>,
    pub downscale_on_save: bool,
    pub window_capture: WindowCaptureStyle,
    pub scroll_max_frames: usize,
    pub scroll_settle_ms: u64,
    pub annotation_color: AnnotationColor,
    /// Upload destination metadata. Credentials are loaded from the environment
    /// at upload time and are never persisted here.
    pub cloud_upload: Option<CloudUploadConfig>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkeys: HotkeySettings::default(),
            theme: ThemePreference::System,
            format: ExportFormat::Png,
            auto_copy: true,
            hide_after_export: true,
            always_on_top: false,
            screenshot_directory: None,
            downscale_on_save: false,
            window_capture: WindowCaptureStyle::default(),
            scroll_max_frames: 40,
            scroll_settle_ms: 250,
            annotation_color: AnnotationColor::default(),
            cloud_upload: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn for_app() -> Result<Self> {
        let project = ProjectDirs::from("io.github", "m4tta", "sniplet")
            .ok_or(PlatformError::SettingsDirectoryUnavailable)?;
        Ok(Self {
            path: project.config_dir().join("settings.json"),
        })
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Settings> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(source) => {
                return Err(PlatformError::ReadFile {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        serde_json::from_slice(&bytes).map_err(|source| PlatformError::InvalidSettings {
            path: self.path.clone(),
            source,
        })
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| PlatformError::WriteFile {
            path: parent.to_owned(),
            source,
        })?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| PlatformError::WriteFile {
                path: parent.to_owned(),
                source,
            })?;
        serde_json::to_writer_pretty(&mut temporary, settings)
            .map_err(PlatformError::EncodeSettings)?;
        temporary
            .write_all(b"\n")
            .and_then(|_| temporary.flush())
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|source| PlatformError::WriteFile {
                path: self.path.clone(),
                source,
            })?;
        temporary
            .persist(&self.path)
            .map_err(|error| PlatformError::WriteFile {
                path: self.path.clone(),
                source: error.error,
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_use_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn settings_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("nested/settings.json"));
        let settings = Settings {
            theme: ThemePreference::Dark,
            format: ExportFormat::Webp,
            auto_copy: false,
            always_on_top: false,
            annotation_color: AnnotationColor {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 4,
            },
            screenshot_directory: Some(directory.path().join("Screenshots")),
            downscale_on_save: true,
            window_capture: WindowCaptureStyle {
                background: crate::WindowBackground::Solid,
                padding: 64,
                color: [20, 30, 40],
                wallpaper: Some(directory.path().join("background.png")),
            },
            scroll_max_frames: 80,
            scroll_settle_ms: 500,
            ..Settings::default()
        };
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn saving_replaces_existing_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        store.save(&Settings::default()).unwrap();
        let replacement = Settings {
            auto_copy: false,
            ..Settings::default()
        };
        store.save(&replacement).unwrap();
        assert_eq!(store.load().unwrap(), replacement);
    }

    #[test]
    fn malformed_settings_are_reported_without_rewriting_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, b"{").unwrap();
        let store = SettingsStore::at(path.clone());

        assert!(matches!(
            store.load(),
            Err(PlatformError::InvalidSettings { path: invalid_path, .. }) if invalid_path == path
        ));
        assert_eq!(fs::read(path).unwrap(), b"{");
    }

    #[test]
    fn newer_fields_are_filled_from_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"auto_copy":false}"#).unwrap();
        assert!(!settings.auto_copy);
        assert_eq!(settings.theme, ThemePreference::System);
        assert_eq!(settings.format, ExportFormat::Png);
        assert_eq!(settings.hotkeys, HotkeySettings::default());
        assert!(settings.screenshot_directory.is_none());
        assert!(!settings.downscale_on_save);
        assert_eq!(settings.window_capture, WindowCaptureStyle::default());
        assert_eq!(settings.scroll_max_frames, 40);
        assert_eq!(settings.scroll_settle_ms, 250);
    }

    #[test]
    fn theme_preferences_use_lowercase_json() {
        for (preference, json) in [
            (ThemePreference::System, r#""system""#),
            (ThemePreference::Light, r#""light""#),
            (ThemePreference::Dark, r#""dark""#),
        ] {
            assert_eq!(serde_json::to_string(&preference).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<ThemePreference>(json).unwrap(),
                preference
            );
        }
    }
}
