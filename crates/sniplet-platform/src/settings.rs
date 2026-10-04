use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::{CloudUploadConfig, PlatformError, Result};

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
            capture_area: "CommandOrControl+Shift+1".to_owned(),
            capture_screen: "CommandOrControl+Shift+2".to_owned(),
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
            annotation_color: AnnotationColor::default(),
            cloud_upload: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
    legacy_path: Option<PathBuf>,
}

impl SettingsStore {
    pub fn for_app() -> Result<Self> {
        let project = ProjectDirs::from("fish", "Box Jelly", "Sniplet")
            .ok_or(PlatformError::SettingsDirectoryUnavailable)?;
        let legacy_path = ProjectDirs::from("fish", "Box Jelly", "Clippy")
            .map(|project| project.config_dir().join("settings.json"));
        Ok(Self {
            path: project.config_dir().join("settings.json"),
            legacy_path,
        })
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy_path: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Settings> {
        if let Some(settings) = Self::load_from(&self.path)? {
            return Ok(settings);
        }
        let Some(legacy_path) = &self.legacy_path else {
            return Ok(Settings::default());
        };
        let Some(settings) = Self::load_from(legacy_path)? else {
            return Ok(Settings::default());
        };

        let temporary = self.temporary_file(&settings)?;
        match temporary.persist_noclobber(&self.path) {
            Ok(_) => Ok(settings),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                Ok(Self::load_from(&self.path)?.unwrap_or(settings))
            }
            Err(error) => Err(PlatformError::WriteFile {
                path: self.path.clone(),
                source: error.error,
            }),
        }
    }

    fn load_from(path: &Path) -> Result<Option<Settings>> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(source) => {
                return Err(PlatformError::ReadFile {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| PlatformError::InvalidSettings {
                path: path.to_owned(),
                source,
            })
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        let temporary = self.temporary_file(settings)?;
        temporary
            .persist(&self.path)
            .map_err(|error| PlatformError::WriteFile {
                path: self.path.clone(),
                source: error.error,
            })?;
        Ok(())
    }

    fn temporary_file(&self, settings: &Settings) -> Result<tempfile::NamedTempFile> {
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
        Ok(temporary)
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
    fn legacy_settings_are_migrated_when_current_settings_are_missing() {
        let directory = tempfile::tempdir().unwrap();
        let legacy_path = directory.path().join("clippy/settings.json");
        let path = directory.path().join("sniplet/settings.json");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        let legacy = Settings {
            theme: ThemePreference::Dark,
            auto_copy: false,
            ..Settings::default()
        };
        fs::write(&legacy_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let store = SettingsStore {
            path: path.clone(),
            legacy_path: Some(legacy_path),
        };

        assert_eq!(store.load().unwrap(), legacy);
        assert_eq!(SettingsStore::at(path).load().unwrap(), legacy);
    }

    #[test]
    fn current_settings_win_over_legacy_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sniplet/settings.json");
        let legacy_path = directory.path().join("clippy/settings.json");
        let current = Settings {
            theme: ThemePreference::Light,
            ..Settings::default()
        };
        let legacy = Settings {
            theme: ThemePreference::Dark,
            ..Settings::default()
        };
        let store = SettingsStore {
            path,
            legacy_path: Some(legacy_path.clone()),
        };
        store.save(&current).unwrap();
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(legacy_path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        assert_eq!(store.load().unwrap(), current);
    }

    #[test]
    fn malformed_legacy_settings_are_not_migrated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sniplet/settings.json");
        let legacy_path = directory.path().join("clippy/settings.json");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, b"{").unwrap();
        let store = SettingsStore {
            path: path.clone(),
            legacy_path: Some(legacy_path.clone()),
        };

        assert!(matches!(
            store.load(),
            Err(PlatformError::InvalidSettings { path, .. }) if path == legacy_path
        ));
        assert!(!path.exists());
    }

    #[test]
    fn newer_fields_are_filled_from_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"auto_copy":false}"#).unwrap();
        assert!(!settings.auto_copy);
        assert_eq!(settings.theme, ThemePreference::System);
        assert_eq!(settings.format, ExportFormat::Png);
        assert_eq!(settings.hotkeys, HotkeySettings::default());
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
