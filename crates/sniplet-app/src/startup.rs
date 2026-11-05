//! The OS owns the startup preference. Do not keep a second copy in settings.json.

/// Explicit requests to open content or settings take priority over a hidden launch.
pub fn start_minimized(args: &[String], login_launch: bool) -> bool {
    (login_launch || args.iter().any(|arg| arg == "--minimized"))
        && !args
            .iter()
            .any(|arg| arg == "--demo" || arg == "--settings" || !arg.starts_with('-'))
}

/// tray-icon creates a Linux tray even when no StatusNotifier host can show it.
/// Ask D-Bus for a host so that a hidden start always leaves a way back in.
#[cfg(target_os = "linux")]
pub fn tray_host_available() -> bool {
    let check = || -> zbus::Result<bool> {
        let connection = zbus::blocking::Connection::session()?;
        zbus::blocking::Proxy::new(
            &connection,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )?
        .get_property("IsStatusNotifierHostRegistered")
    };
    check().unwrap_or(false)
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
    // Task Manager and Settings > Apps > Startup turn an entry off here and keep its Run value.
    const APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    const VALUE_NAME: &str = "Sniplet";
    // HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND), for an absent value.
    const NOT_FOUND: u32 = 0x8007_0002;

    pub fn enabled() -> anyhow::Result<bool> {
        registered(
            &CURRENT_USER.create(RUN_KEY)?,
            &CURRENT_USER.create(APPROVED_KEY)?,
        )
    }

    fn registered(run: &Key, approved: &Key) -> anyhow::Result<bool> {
        let command = match run.get_string(VALUE_NAME) {
            Ok(command) => !command.is_empty(),
            Err(error) if error.code().0 as u32 == NOT_FOUND => false,
            Err(error) => return Err(error.into()),
        };
        // An odd first byte means that the user turned the entry off.
        let off = match approved.get_value(VALUE_NAME) {
            Ok(state) => state.first().is_some_and(|byte| byte & 1 == 1),
            Err(error) if error.code().0 as u32 == NOT_FOUND => false,
            Err(error) => return Err(error.into()),
        };
        Ok(command && !off)
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

    fn remove(key: &Key) -> anyhow::Result<()> {
        match key.remove_value(VALUE_NAME) {
            Ok(()) => Ok(()),
            Err(error) if error.code().0 as u32 == NOT_FOUND => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn update(run: &Key, approved: &Key, executable: &Path, enabled: bool) -> anyhow::Result<bool> {
        if enabled {
            run.set_string(VALUE_NAME, command(executable)?)?;
        } else {
            remove(run)?;
        }
        // The Sniplet switch replaces an earlier choice in system settings.
        remove(approved)?;
        registered(run, approved)
    }

    pub fn set_enabled(enabled: bool) -> anyhow::Result<bool> {
        update(
            &CURRENT_USER.create(RUN_KEY)?,
            &CURRENT_USER.create(APPROVED_KEY)?,
            &std::env::current_exe()?,
            enabled,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn registration_quotes_the_path_and_removes_only_its_own_value() {
            // Exercise the real registry API without changing the user's startup keys.
            let path = format!(r"Software\SnipletTests\Startup-{}", std::process::id());
            let run = CURRENT_USER.create(format!(r"{path}\Run")).unwrap();
            let approved = CURRENT_USER.create(format!(r"{path}\Approved")).unwrap();
            run.set_string("OtherApp", "keep me").unwrap();
            let executable = Path::new(r"C:\Apps\Sniplet 测试\sniplet.exe");
            assert!(!registered(&run, &approved).unwrap());
            assert!(update(&run, &approved, executable, true).unwrap());
            assert_eq!(
                run.get_string(VALUE_NAME).unwrap(),
                r#""C:\Apps\Sniplet 测试\sniplet.exe" --minimized"#
            );
            // Task Manager writes 03 to turn an entry off and 02 to turn it on again.
            let state = |first| [first, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            approved
                .set_bytes(VALUE_NAME, windows_registry::Type::Bytes, &state(3))
                .unwrap();
            assert!(!registered(&run, &approved).unwrap());
            approved
                .set_bytes(VALUE_NAME, windows_registry::Type::Bytes, &state(2))
                .unwrap();
            assert!(registered(&run, &approved).unwrap());
            approved
                .set_bytes(VALUE_NAME, windows_registry::Type::Bytes, &state(3))
                .unwrap();
            assert!(update(&run, &approved, executable, true).unwrap());
            assert!(!update(&run, &approved, executable, false).unwrap());
            assert!(!update(&run, &approved, executable, false).unwrap());
            assert_eq!(run.get_string("OtherApp").unwrap(), "keep me");
            drop((run, approved));
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
