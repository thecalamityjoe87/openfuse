//! Start at login: the desktop entry is created and removed under
//! $XDG_CONFIG_HOME (one test in this binary, so setting the variable is safe).

use openfuse::autostart;

#[test]
fn autostart_entry_create_and_remove() {
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.path()) };
    let path = tmp.path().join("autostart").join("openfuse.desktop");
    assert_eq!(autostart::autostart_path(), path);
    assert!(!autostart::autostart_enabled());

    autostart::set_autostart(true).unwrap();
    assert!(autostart::autostart_enabled());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("[Desktop Entry]\n") && text.contains("Type=Application") && text.contains("Name=OpenFuse"));
    let args = autostart::exec_args(&path);
    assert_eq!(args.last().map(String::as_str), Some("--tray"));
    assert_eq!(std::path::Path::new(&args[0]), std::env::current_exe().unwrap());
    let icon = text.lines().find_map(|l| l.strip_prefix("Icon=")).unwrap();
    assert!(std::path::Path::new(icon).is_file(), "icon written next to the settings");

    autostart::set_autostart(true).unwrap(); // idempotent
    autostart::set_autostart(false).unwrap();
    assert!(!autostart::autostart_enabled() && !path.exists());
    autostart::set_autostart(false).unwrap(); // removing twice is fine
}
