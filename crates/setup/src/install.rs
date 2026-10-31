//! Running the embedded NSIS installer and the bits around it.
//!
//! Built without an installer (no `payload` cfg, or not on Windows) this is
//! a simulation, so the window can be developed and tested anywhere.

use serde::{Deserialize, Serialize};

/// The two choices the window offers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub desktop_shortcut: bool,
    pub start_with_windows: bool,
}

pub struct Installed {
    pub version: String,
    #[cfg_attr(not(all(windows, payload)), allow(dead_code))]
    pub exe: std::path::PathBuf,
}

pub const HAS_PAYLOAD: bool = cfg!(all(windows, payload));

#[cfg(all(windows, payload))]
pub use real::*;

#[cfg(not(all(windows, payload)))]
pub use demo::*;

#[cfg(all(windows, payload))]
mod real {
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE};

    use super::{Installed, Options};

    static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.exe"));

    /// Must match the app's productName: NSIS names its registry keys and
    /// shortcuts after it, and the app's start-at-login entry uses it too.
    const PRODUCT: &str = "Lyrics Overlay";
    const UNINSTALL_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Lyrics Overlay";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const STARTUP_APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    /// What the app's start-at-login entry passes, so it starts hidden.
    const AT_LOGIN_ARG: &str = "--autostart";
    const WEBVIEW2_CLIENT: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn installed() -> Option<Installed> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(UNINSTALL_KEY, KEY_READ)
            .ok()?;
        let version: String = key.get_value("DisplayVersion").ok()?;
        let icon: String = key.get_value("DisplayIcon").ok()?;
        Some(Installed {
            version,
            exe: PathBuf::from(icon.trim_matches('"')),
        })
    }

    /// For a reinstall or update, keep what the user has now.
    pub fn current_options(installed: bool) -> Options {
        Options {
            desktop_shortcut: !installed || desktop_shortcut().is_some_and(|p| p.exists()),
            start_with_windows: run_entry().is_some(),
        }
    }

    pub fn run(options: &Options) -> Result<(), String> {
        let path = std::env::temp_dir().join(format!(
            "LyricsOverlay-{}-installer.exe",
            env!("CARGO_PKG_VERSION")
        ));
        std::fs::write(&path, PAYLOAD)
            .map_err(|e| format!("Couldn't unpack the installer: {e}"))?;
        // /S: silent. It closes a running copy of the app itself.
        let status = Command::new(&path)
            .arg("/S")
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        let _ = std::fs::remove_file(&path);
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                return Err(format!(
                    "The installer stopped with code {}.",
                    s.code().unwrap_or(-1)
                ));
            }
            Err(e) => return Err(format!("Couldn't start the installer: {e}")),
        }

        let exe = installed()
            .map(|i| i.exe)
            .ok_or("The installer finished, but Lyrics Overlay isn't registered as installed.")?;
        // NSIS adds the desktop shortcut in silent mode; its "no shortcuts"
        // switch would drop the Start menu entry too, so remove it after.
        if !options.desktop_shortcut {
            if let Some(link) = desktop_shortcut() {
                let _ = std::fs::remove_file(link);
            }
        }
        set_start_with_windows(options.start_with_windows, &exe)
            .map_err(|e| format!("Installed, but couldn't set Start with Windows: {e}"))
    }

    pub fn launch() -> Result<(), String> {
        let exe = installed()
            .map(|i| i.exe)
            .ok_or("Lyrics Overlay isn't installed.")?;
        Command::new(&exe)
            .current_dir(exe.parent().unwrap_or(Path::new(".")))
            .spawn()
            .map(drop)
            .map_err(|e| format!("Couldn't start Lyrics Overlay: {e}"))
    }

    /// Runs the classic installer, with its own window, when this one can't
    /// be shown. Returns whether it did (and the setup should exit).
    pub fn fall_back_without_webview() -> bool {
        if webview2_installed() {
            return false;
        }
        let path = std::env::temp_dir().join("LyricsOverlay-installer.exe");
        if std::fs::write(&path, PAYLOAD).is_ok() {
            let _ = Command::new(&path).status();
            let _ = std::fs::remove_file(&path);
        }
        true
    }

    fn webview2_installed() -> bool {
        let read = |root, key: &str| {
            RegKey::predef(root)
                .open_subkey_with_flags(key, KEY_READ)
                .and_then(|k| k.get_value::<String, _>("pv"))
                .is_ok_and(|v| !v.is_empty() && v != "0.0.0.0")
        };
        let client =
            |prefix: &str| format!(r"{prefix}Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT}");
        read(HKEY_LOCAL_MACHINE, &client(r"SOFTWARE\WOW6432Node\"))
            || read(HKEY_LOCAL_MACHINE, &client(r"SOFTWARE\"))
            || read(HKEY_CURRENT_USER, &client(r"Software\"))
    }

    fn desktop_shortcut() -> Option<PathBuf> {
        dirs::desktop_dir().map(|d| d.join(format!("{PRODUCT}.lnk")))
    }

    fn run_entry() -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_READ)
            .and_then(|k| k.get_value(PRODUCT))
            .ok()
    }

    /// Writes the same entry the app's own "Start with Windows" toggle does
    /// (tauri-plugin-autostart), so the toggle shows it as on.
    fn set_start_with_windows(on: bool, exe: &Path) -> std::io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
        if !on {
            return match run.delete_value(PRODUCT) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        run.set_value(PRODUCT, &format!("{} {AT_LOGIN_ARG}", exe.display()))?;
        // Clears a "disabled in Task Manager" mark, as the plugin does.
        if let Ok(approved) = hkcu.open_subkey_with_flags(STARTUP_APPROVED_KEY, KEY_SET_VALUE) {
            let enabled = winreg::RegValue {
                vtype: winreg::enums::RegType::REG_BINARY,
                bytes: vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into(),
            };
            approved.set_raw_value(PRODUCT, &enabled)?;
        }
        Ok(())
    }
}

/// Stand-in for development: nothing installed, a few seconds of
/// "installing", then success (or failure with `LYRICS_SETUP_DEMO_FAIL`).
#[cfg(not(all(windows, payload)))]
mod demo {
    use std::time::Duration;

    use super::{Installed, Options};

    pub fn installed() -> Option<Installed> {
        std::env::var("LYRICS_SETUP_DEMO_INSTALLED")
            .ok()
            .map(|version| Installed {
                version,
                exe: Default::default(),
            })
    }

    pub fn current_options(installed: bool) -> Options {
        Options {
            desktop_shortcut: !installed,
            start_with_windows: false,
        }
    }

    pub fn run(_options: &Options) -> Result<(), String> {
        std::thread::sleep(Duration::from_millis(3500));
        match std::env::var("LYRICS_SETUP_DEMO_FAIL") {
            Ok(_) => Err("The installer stopped with code 2.".into()),
            Err(_) => Ok(()),
        }
    }

    pub fn launch() -> Result<(), String> {
        Ok(())
    }

    pub fn fall_back_without_webview() -> bool {
        false
    }
}
