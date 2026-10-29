//! The OS owns the startup preference. Do not keep a second copy in settings.json.

/// Explicit requests to open content or settings take priority over a hidden launch.
pub fn start_minimized(args: &[String], login_launch: bool) -> bool {
    (login_launch || args.iter().any(|arg| arg == "--minimized"))
        && !args
            .iter()
            .any(|arg| arg == "--demo" || arg == "--settings" || !arg.starts_with('-'))
}

#[cfg(target_os = "macos")]
pub fn enabled() -> anyhow::Result<bool> {
    Ok(crate::macos::launch_at_startup())
}

#[cfg(target_os = "macos")]
pub fn set_enabled(enabled: bool) -> anyhow::Result<bool> {
    crate::macos::set_launch_at_startup(enabled).map_err(anyhow::Error::msg)
}

#[cfg(target_os = "windows")]
mod windows {
    use std::path::Path;

    use windows_registry::{CURRENT_USER, Key};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "Sniplet";
    // HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND), for an absent key or value.
    const NOT_FOUND: u32 = 0x8007_0002;

    pub fn enabled() -> anyhow::Result<bool> {
        match CURRENT_USER.open(RUN_KEY) {
            Ok(key) => registered(&key),
            Err(error) if error.code().0 as u32 == NOT_FOUND => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn registered(key: &Key) -> anyhow::Result<bool> {
        match key.get_string(VALUE_NAME) {
            Ok(command) => Ok(!command.is_empty()),
            Err(error) if error.code().0 as u32 == NOT_FOUND => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn command(executable: &Path) -> anyhow::Result<String> {
        let path = executable
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("The app path is not valid Unicode"))?;
        let command = format!("\"{path}\" --minimized");
        // Windows limits Run values to 260 characters, including arguments.
        anyhow::ensure!(
            command.encode_utf16().count() <= 260,
            "The app path is too long for Windows startup. Move Sniplet to a shorter path."
        );
        Ok(command)
    }

    fn update(key: &Key, executable: &Path, enabled: bool) -> anyhow::Result<bool> {
        if enabled {
            key.set_string(VALUE_NAME, command(executable)?)?;
        } else {
            match key.remove_value(VALUE_NAME) {
                Ok(()) => {}
                Err(error) if error.code().0 as u32 == NOT_FOUND => {}
                Err(error) => return Err(error.into()),
            }
        }
        registered(key)
    }

    pub fn set_enabled(enabled: bool) -> anyhow::Result<bool> {
        let key = CURRENT_USER.create(RUN_KEY)?;
        update(&key, &std::env::current_exe()?, enabled)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn registration_quotes_the_path_and_removes_only_its_own_value() {
            // Exercise the real registry API without changing the user's Run key.
            let path = format!(r"Software\SnipletTests\Startup-{}", std::process::id());
            let key = CURRENT_USER.create(&path).unwrap();
            key.set_string("OtherApp", "keep me").unwrap();
            let executable = Path::new(r"C:\Apps\Sniplet 测试\sniplet.exe");
            assert!(!registered(&key).unwrap());
            assert!(update(&key, executable, true).unwrap());
            assert_eq!(
                key.get_string(VALUE_NAME).unwrap(),
                r#""C:\Apps\Sniplet 测试\sniplet.exe" --minimized"#
            );
            assert!(update(&key, executable, true).unwrap());
            assert!(!update(&key, executable, false).unwrap());
            assert!(!update(&key, executable, false).unwrap());
            assert_eq!(key.get_string("OtherApp").unwrap(), "keep me");
            drop(key);
            CURRENT_USER.remove_tree(&path).unwrap();
        }

        #[test]
        fn a_run_command_that_exceeds_the_windows_limit_is_rejected() {
            assert!(command(Path::new(&format!("C:\\{}\\sniplet.exe", "x".repeat(250)))).is_err());
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows::{enabled, set_enabled};

#[cfg(any(target_os = "linux", test))]
mod linux {
    use std::{fs, path::Path};

    #[cfg(target_os = "linux")]
    fn entry_path() -> anyhow::Result<std::path::PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
            .filter(|path| path.is_absolute())
            .ok_or_else(|| anyhow::anyhow!("The user configuration directory is unavailable"))?;
        Ok(config
            .join("autostart")
            .join(format!("{}.desktop", sniplet_platform::APP_ID)))
    }

    fn desktop_entry(executable: &Path) -> anyhow::Result<String> {
        let path = executable
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("The app path is not valid UTF-8"))?;
        anyhow::ensure!(
            !path.contains(['\n', '\r', '=']),
            "The app path contains characters that Linux startup does not support"
        );
        // Desktop-entry string escaping is applied before Exec argument quoting.
        let mut quoted = String::new();
        for character in path.chars() {
            match character {
                '\\' => quoted.push_str(r"\\\\"),
                '"' | '`' | '$' => {
                    quoted.push_str(r"\\");
                    quoted.push(character);
                }
                '%' => quoted.push_str("%%"),
                _ => quoted.push(character),
            }
        }
        Ok(format!(
            "[Desktop Entry]\nType=Application\nName=Sniplet\nExec=\"{quoted}\" --minimized\nTerminal=false\n"
        ))
    }

    fn registered(path: &Path) -> anyhow::Result<bool> {
        let entry = match fs::read_to_string(path) {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let mut in_entry = false;
        let mut has_command = false;
        for line in entry.lines().map(str::trim) {
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
            } else if in_entry && let Some((key, value)) = line.split_once('=') {
                let (key, value) = (key.trim(), value.trim());
                if (key == "Hidden" && value == "true")
                    || (key == "X-GNOME-Autostart-enabled" && value == "false")
                {
                    return Ok(false);
                }
                if key == "Exec" {
                    has_command = !value.is_empty();
                }
            }
        }
        Ok(has_command)
    }

    fn update(path: &Path, executable: &Path, enabled: bool) -> anyhow::Result<bool> {
        if enabled {
            let entry = desktop_entry(executable)?;
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, entry)?;
        } else {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        registered(path)
    }

    #[cfg(target_os = "linux")]
    pub fn enabled() -> anyhow::Result<bool> {
        registered(&entry_path()?)
    }

    #[cfg(target_os = "linux")]
    pub fn set_enabled(enabled: bool) -> anyhow::Result<bool> {
        update(&entry_path()?, &std::env::current_exe()?, enabled)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn desktop_commands_escape_spaces_and_reserved_characters() {
            let entry = desktop_entry(Path::new("/opt/Sniplet $`\"\\%/sniplet")).unwrap();
            assert!(entry.contains(r#"Exec="/opt/Sniplet \\$\\`\\"\\\\%%/sniplet" --minimized"#));
            assert!(desktop_entry(Path::new("/opt/invalid\npath/sniplet")).is_err());
        }

        #[test]
        fn registration_can_be_enabled_disabled_and_removed() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("autostart/sniplet.desktop");
            let executable = Path::new("/opt/Sniplet/sniplet");
            assert!(!registered(&path).unwrap());
            assert!(update(&path, executable, true).unwrap());
            assert!(update(&path, executable, true).unwrap());
            for disabled in [
                "Hidden=true",
                "Hidden = true",
                "X-GNOME-Autostart-enabled=false",
            ] {
                fs::write(
                    &path,
                    format!("{}\n{disabled}\n", desktop_entry(executable).unwrap()),
                )
                .unwrap();
                assert!(!registered(&path).unwrap());
            }
            assert!(!update(&path, executable, false).unwrap());
            assert!(!path.exists());
            assert!(!update(&path, executable, false).unwrap());
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{enabled, set_enabled};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_background_launches_start_minimized() {
        let args = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        assert!(!start_minimized(&[], false));
        assert!(start_minimized(&[], true));
        assert!(start_minimized(&args(&["--minimized"]), false));
        for explicit in ["--demo", "--settings", "image.png", "project.sniplet"] {
            assert!(!start_minimized(&args(&["--minimized", explicit]), false));
            assert!(!start_minimized(&args(&[explicit]), true));
        }
    }
}
