// SPDX-License-Identifier: GPL-3.0-or-later

//! Desktop integration: an app menu launcher (to reopen the app while it runs in the
//! background) and the entry that starts it at login.

use std::io;
use std::path::{Path, PathBuf};

use gtk::glib;

use crate::APP_ID;

/// Command-line flag for starting without a window, as used at login.
pub const BACKGROUND_FLAG: &str = "--background";

fn launcher_path() -> PathBuf {
    glib::user_data_dir().join("applications").join(format!("{APP_ID}.desktop"))
}

fn autostart_path() -> PathBuf {
    glib::user_config_dir().join("autostart").join(format!("{APP_ID}.desktop"))
}

/// A desktop entry that runs `exe` with `args`.
fn entry(exe: &Path, args: &str) -> io::Result<String> {
    let exe = exe.to_str().ok_or_else(|| io::Error::other("the program's path isn't valid text"))?;
    // Keep quoting simple: refuse the few characters that would need escaping in Exec.
    if exe.chars().any(|c| matches!(c, '"' | '`' | '$' | '\\') || c.is_control()) {
        return Err(io::Error::other("the program's path contains characters a launcher can't hold"));
    }
    let exec = format!("\"{}\"{}{args}", exe.replace('%', "%%"), if args.is_empty() { "" } else { " " });
    Ok(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=OpenSteelStudio\n\
         Comment=Lighting and screen editor for SteelSeries keyboards\n\
         Exec={exec}\n\
         Icon=input-keyboard\n\
         Terminal=false\n\
         Categories=Utility;Settings;HardwareSettings;\n\
         StartupNotify=true\n"
    ))
}

fn write(path: &Path, contents: &str) -> io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap_or(path))?;
    std::fs::write(path, contents)
}

/// The program to start: the AppImage file when running from one (its contents live in a
/// temporary folder that changes every run), otherwise this executable.
fn program() -> io::Result<PathBuf> {
    match std::env::var_os("APPIMAGE") {
        Some(appimage) => Ok(appimage.into()),
        None => std::env::current_exe(),
    }
}

/// Whether a package already put OpenSteelStudio in the app menu.
fn installed_system_wide() -> bool {
    glib::system_data_dirs().iter().any(|dir| dir.join("applications").join(format!("{APP_ID}.desktop")).exists())
}

/// Adds OpenSteelStudio to the app menu, pointing at the program that is running now,
/// unless a package already did.
pub fn install_launcher() -> io::Result<()> {
    if installed_system_wide() {
        return Ok(());
    }
    write(&launcher_path(), &entry(&program()?, "")?)
}

pub fn autostart_enabled() -> bool {
    autostart_path().exists()
}

/// Starts OpenSteelStudio without a window at login, or stops doing so.
pub fn set_autostart(on: bool) -> io::Result<()> {
    let path = autostart_path();
    if on {
        write(&path, &entry(&program()?, BACKGROUND_FLAG)?)
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_quotes_the_program() {
        let text = entry(Path::new("/home/me/My Apps/open-steel-studio"), BACKGROUND_FLAG).unwrap();
        assert!(text.contains("Exec=\"/home/me/My Apps/open-steel-studio\" --background\n"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        let plain = entry(Path::new("/usr/bin/open-steel-studio"), "").unwrap();
        assert!(plain.contains("Exec=\"/usr/bin/open-steel-studio\"\n"));
    }

    #[test]
    fn entry_escapes_or_refuses_special_characters() {
        assert!(entry(Path::new("/tmp/100%/app"), "").unwrap().contains("/tmp/100%%/app"));
        assert!(entry(Path::new("/tmp/$HOME/app"), "").is_err());
    }
}
