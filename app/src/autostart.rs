//! XDG autostart entry ("Start at login") and small persistent settings.
//!
//! Start at login = `$XDG_CONFIG_HOME/autostart/openfuse.desktop` (default
//! `~/.config`), launching this binary with `--tray`. Settings live in
//! `$XDG_CONFIG_HOME/openfuse/settings.ini`. Paths are resolved on every call
//! so tests can point `XDG_CONFIG_HOME` at a temp dir.

use std::io;
use std::path::{Path, PathBuf};

pub const DESKTOP_NAME: &str = "openfuse.desktop";

pub fn config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(p) if !p.is_empty() && Path::new(&p).is_absolute() => PathBuf::from(p),
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(".config"),
    }
}

pub fn autostart_path() -> PathBuf {
    config_home().join("autostart").join(DESKTOP_NAME)
}

pub fn app_config_dir() -> PathBuf {
    config_home().join("openfuse")
}

/// The binary to start at login: the AppImage if we run from one, else this executable.
pub fn exe_path() -> PathBuf {
    if let Some(p) = std::env::var_os("APPIMAGE") {
        return PathBuf::from(p);
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("openfuse"))
}

/// Quote one Exec argument (Desktop Entry spec, "The Exec key").
fn exec_quote(s: &str) -> String {
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    let q = if s.chars().any(reserved) {
        let mut o = String::from("\"");
        for c in s.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                o.push('\\');
            }
            o.push(c);
        }
        o.push('"');
        o
    } else {
        s.to_string()
    };
    // field codes start with %; then string-level escaping of backslashes
    q.replace('%', "%%").replace('\\', "\\\\")
}

pub fn desktop_entry(exe: &Path, icon: &Path) -> String {
    let exec = [exe.to_string_lossy().as_ref(), "--tray"].iter().map(|a| exec_quote(a)).collect::<Vec<_>>().join(" ");
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=OpenFuse\n\
         Comment=Control panel for Arturia MiniFuse audio interfaces\n\
         Exec={exec}\n\
         Icon={}\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;Settings;\n\
         X-GNOME-Autostart-enabled=true\n",
        icon.to_string_lossy()
    )
}

pub fn autostart_enabled() -> bool {
    autostart_path().is_file()
}

/// Create or remove the autostart entry (atomic write).
pub fn set_autostart(on: bool) -> io::Result<()> {
    let path = autostart_path();
    if on {
        let icon = app_config_dir().join("openfuse.svg");
        std::fs::create_dir_all(app_config_dir())?;
        std::fs::write(&icon, crate::icons::icon_svg(false, false))?;
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        let tmp = path.with_extension("desktop.tmp");
        std::fs::write(&tmp, desktop_entry(&exe_path(), &icon))?;
        std::fs::rename(&tmp, &path)?;
    } else if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Parse the Exec= line of an entry back into arguments (used by tests).
pub fn exec_args(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    let Some(line) = text.lines().find_map(|l| l.strip_prefix("Exec=")) else { return vec![] };
    let line = line.replace("\\\\", "\\").replace("%%", "%");
    let mut out = vec![];
    let mut cur = String::new();
    let (mut in_q, mut any) = (false, false);
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_q = !in_q;
                any = true;
            }
            '\\' if in_q => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ' ' if !in_q => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Tiny key=value settings file.
#[derive(Clone)]
pub struct Settings {
    path: PathBuf,
}

impl Settings {
    pub fn open() -> Settings {
        Settings { path: app_config_dir().join("settings.ini") }
    }
    pub fn at(path: PathBuf) -> Settings {
        Settings { path }
    }
    fn read(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect()
    }
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        // missing file, missing key or an unreadable value: the default
        self.read()
            .into_iter()
            .rev()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v.as_str() {
                "1" | "true" | "yes" => Some(true),
                "0" | "false" | "no" => Some(false),
                _ => None,
            })
            .unwrap_or(default)
    }
    pub fn set_bool(&self, key: &str, value: bool) -> io::Result<()> {
        self.set_raw(key, Some(value.to_string()))
    }
    /// A number, or `None` when the key is missing or unreadable.
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.read().into_iter().rev().find(|(k, _)| k == key).and_then(|(_, v)| v.parse().ok()).filter(|v: &f64| v.is_finite())
    }
    /// Store a number; `None` removes the key.
    pub fn set_f64(&self, key: &str, value: Option<f64>) -> io::Result<()> {
        self.set_raw(key, value.map(|v| v.to_string()))
    }
    fn set_raw(&self, key: &str, value: Option<String>) -> io::Result<()> {
        let mut kv = self.read();
        kv.retain(|(k, _)| k != key);
        if let Some(v) = value {
            kv.push((key.to_string(), v));
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text: String = kv.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        std::fs::write(&self.path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_quoting_round_trips() {
        let t = tempfile::tempdir().unwrap();
        let exe = Path::new("/opt/My Apps/open$fuse \"x\"\\bin/openfuse%1");
        let p = t.path().join("e.desktop");
        std::fs::write(&p, desktop_entry(exe, Path::new("/i.svg"))).unwrap();
        assert_eq!(exec_args(&p), vec![exe.to_string_lossy().to_string(), "--tray".to_string()]);
        let plain = Path::new("/usr/bin/openfuse");
        std::fs::write(&p, desktop_entry(plain, Path::new("/i.svg"))).unwrap();
        assert_eq!(exec_args(&p), vec!["/usr/bin/openfuse", "--tray"]);
    }

    #[test]
    fn settings_store_bools() {
        let t = tempfile::tempdir().unwrap();
        let s = Settings::at(t.path().join("x/settings.ini"));
        assert!(!s.get_bool("a", false));
        s.set_bool("a", true).unwrap();
        s.set_bool("b", false).unwrap();
        assert!(s.get_bool("a", false) && !s.get_bool("b", true));
        s.set_bool("a", false).unwrap();
        assert!(!s.get_bool("a", true));
    }
}
