//! The actual work: install, enroll the Secure Boot key, uninstall, restart.
//!
//! Everything here blocks; the UI runs it on a worker thread and receives
//! progress through the `emit` callback. Every operation has a simulated
//! variant (`sim = true`) that changes nothing on the system, for trying the
//! installer out and for screenshots.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use crate::packagekit::{self, Client, FILTER_INSTALLED, FILTER_NOT_INSTALLED};
use crate::helper;
use crate::sysinfo::{self, PACKAGE_APP, PACKAGE_DKMS};

/// Steps shown in the progress list, in order.
pub const INSTALL_STEPS: &[&str] = &[
    "Kernel build files",
    "OpenFuse driver and app",
    "Check the driver",
];
pub const UNINSTALL_STEPS: &[&str] = &["Remove OpenFuse"];

#[derive(Debug, Clone)]
pub enum Event {
    StepStarted(usize),
    StepDone(usize),
    StepSkipped(usize),
    Status(String),
    Progress(Option<u8>),
    Log(String),
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub message: String,
    pub details: String,
}

impl Failure {
    fn new(message: impl Into<String>, details: impl Into<String>) -> Self {
        Failure { message: message.into(), details: details.into() }
    }
}

impl From<packagekit::Error> for Failure {
    fn from(e: packagekit::Error) -> Self {
        Failure::new(e.message, e.details)
    }
}

#[derive(Debug, Clone)]
pub struct InstallOutcome {
    /// Secure Boot is on and the signing key still has to be enrolled.
    pub needs_key: bool,
    pub cert: Option<String>,
}

// ---------------------------------------------------------------- payload

/// The OpenFuse packages to install. They ship next to the installer, in plain
/// sight, so anyone can see (and install by hand) exactly what it installs.
pub struct Payload {
    pub files: Vec<PathBuf>,
}

/// Where the packages are: `--payload DIR`, else the folder the installer itself is in.
pub fn package_dir(dir: Option<&Path>) -> Option<PathBuf> {
    match dir {
        Some(d) => Some(d.to_path_buf()),
        None => std::env::current_exe().ok()?.parent().map(Path::to_path_buf),
    }
}

/// Is this one of ours ("openfuse_<ver>_<arch>.deb", "openfuse-dkms_<ver>_all.deb")?
fn is_ours(name: &str) -> bool {
    name.ends_with(".deb") && (name.starts_with(&format!("{PACKAGE_APP}_")) || name.starts_with(&format!("{PACKAGE_DKMS}_")))
}

impl Payload {
    pub fn locate(dir: Option<&Path>) -> Result<Payload, Failure> {
        let missing = || {
            Failure::new(
                "No OpenFuse packages were found next to the installer",
                "Keep openfuse-installer in its folder, next to openfuse_*.deb and openfuse-dkms_*.deb: unpack the \
                 whole download and run it from there.",
            )
        };
        let dir = package_dir(dir).ok_or_else(missing)?;
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| Failure::new("The package folder could not be read", format!("{}: {e}", dir.display())))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| is_ours(&n.to_string_lossy())))
            .collect();
        files.sort();
        if files.is_empty() {
            return Err(missing());
        }
        Ok(Payload { files })
    }
}

/// The app version this installer would install (from its package's file name next to it).
pub fn bundled_version(dir: Option<&Path>) -> Option<String> {
    let dir = package_dir(dir)?;
    fs::read_dir(dir).ok()?.flatten().find_map(|e| package_version(&e.file_name().to_string_lossy(), PACKAGE_APP))
}

/// "openfuse_0.1.0_amd64.deb" -> "0.1.0" for package "openfuse".
fn package_version(file: &str, package: &str) -> Option<String> {
    let rest = file.strip_suffix(".deb")?.strip_prefix(package)?.strip_prefix('_')?;
    rest.split('_').next().filter(|v| !v.is_empty()).map(String::from)
}

/// Order two package versions ("0.1.0", "0.10.2-1"): numeric parts as numbers, others as text.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<String> { s.split(['.', '-', '+', '~']).map(String::from).collect() };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i).map_or("0", String::as_str), pb.get(i).map_or("0", String::as_str));
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if o.is_ne() {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

// ---------------------------------------------------------------- install

fn pk_forward(emit: &mut dyn FnMut(Event)) -> impl FnMut(packagekit::Event) + '_ {
    move |ev| match ev {
        packagekit::Event::Progress(p) => emit(Event::Progress(p)),
        packagekit::Event::Status(s) => emit(Event::Status(s.to_string())),
        packagekit::Event::Package { id } => {
            let name = id.split(';').next().unwrap_or("").to_string();
            if !name.is_empty() {
                emit(Event::Log(name));
            }
        }
    }
}

pub fn install(payload_dir: Option<PathBuf>, sim: bool, emit: &mut dyn FnMut(Event)) -> Result<InstallOutcome, Failure> {
    if sim {
        return simulate_install(emit);
    }
    let info = sysinfo::gather();
    let client = Client::connect()?;

    // 1. Kernel headers for the running kernel (the driver is built against them).
    emit(Event::StepStarted(0));
    if info.headers {
        emit(Event::StepSkipped(0));
    } else {
        emit(Event::Status("Looking up the kernel build files".into()));
        let name = format!("linux-headers-{}", info.kernel);
        let found = client.resolve(&[name.as_str()], FILTER_NOT_INSTALLED)?;
        let ids: Vec<String> = found.iter().filter(|p| p.name() == name).map(|p| p.id.clone()).collect();
        if ids.is_empty() {
            return Err(Failure::new(
                "The build files for your kernel are not available",
                format!("Package {name} was not found. Update your system, restart, and run the installer again."),
            ));
        }
        client.install_packages(&ids, &mut pk_forward(emit))?;
        emit(Event::StepDone(0));
    }

    // 2. OpenFuse packages (PackageKit pulls in dkms and other dependencies).
    emit(Event::StepStarted(1));
    emit(Event::Progress(None));
    emit(Event::Status("Waiting for permission".into()));
    let payload = Payload::locate(payload_dir.as_deref())?;
    let files: Vec<String> = payload.files.iter().map(|p| p.display().to_string()).collect();
    client.install_files(&files, &mut pk_forward(emit))?;
    emit(Event::StepDone(1));

    // 3. Driver for the running kernel. With Secure Boot on and the key not yet
    //    approved, the package deliberately skips the build; the key step does it.
    emit(Event::StepStarted(2));
    emit(Event::Progress(None));
    emit(Event::Status("Checking the driver".into()));
    if !helper::installed() {
        return Err(Failure::new(
            "The OpenFuse driver package is incomplete",
            format!("{} is missing after installing {PACKAGE_DKMS}.", helper::HELPER),
        ));
    }
    let st = helper::status().map_err(|e| Failure::new("Could not check the driver", e.to_string()))?;
    emit(Event::Log(format!("dkms_state={} key_state={}", st.get("dkms_state"), st.get("key_state"))));
    let needs_key = st.get("secure_boot") == "enabled" && st.get("key_state") != "enrolled";
    if needs_key {
        emit(Event::StepDone(2));
        return Ok(InstallOutcome { needs_key: true, cert: Some(st.get("cert").to_string()) });
    }
    if st.get("dkms_state") != "installed" {
        emit(Event::Status("Building the driver".into()));
        let out = helper::build().map_err(|e| Failure::new("Could not build the driver", e.to_string()))?;
        if helper::denied(out.code) {
            return Err(Failure::new("Permission was not granted, so the driver was not built", out.details()));
        }
        if out.get("build") != "installed" {
            return Err(Failure::new(helper::reason_text(out.get("reason")), out.details()));
        }
    }
    emit(Event::StepDone(2));
    Ok(InstallOutcome { needs_key: false, cert: None })
}

fn simulate_install(emit: &mut dyn FnMut(Event)) -> Result<InstallOutcome, Failure> {
    let pause = |ms| sleep(Duration::from_millis(ms));
    emit(Event::StepStarted(0));
    pause(500);
    emit(Event::StepSkipped(0));
    emit(Event::StepStarted(1));
    emit(Event::Status("Waiting for permission".into()));
    emit(Event::Progress(None));
    pause(1200);
    for (pct, status, pkg) in [
        (10, "Resolving dependencies", "dkms"),
        (35, "Downloading", "dkms"),
        (55, "Installing", "openfuse-dkms"),
        (70, "Setting up (building the driver can take a minute)", "openfuse-dkms"),
        (90, "Installing", "openfuse"),
        (100, "Applying changes", "openfuse"),
    ] {
        emit(Event::Status(status.into()));
        emit(Event::Progress(Some(pct)));
        emit(Event::Log(pkg.into()));
        pause(700);
    }
    emit(Event::StepDone(1));
    emit(Event::StepStarted(2));
    emit(Event::Status("Checking the driver".into()));
    emit(Event::Progress(None));
    pause(800);
    emit(Event::StepDone(2));
    Ok(InstallOutcome { needs_key: true, cert: Some("/var/lib/shim-signed/mok/MOK.der".into()) })
}

// ---------------------------------------------------------------- secure boot

/// Queue the signing key for approval at the next start, and build the driver,
/// in one privileged step (one password prompt).
pub fn enroll_key(password: &str, sim: bool) -> Result<(), Failure> {
    if sim {
        sleep(Duration::from_millis(1200));
        return Ok(());
    }
    let out = helper::enroll(password).map_err(|e| Failure::new("Could not set up the key", e.to_string()))?;
    if helper::denied(out.code) {
        return Err(Failure::new("Permission was not granted, so the key was not set up", out.details()));
    }
    let queued = matches!(out.get("enroll"), "imported" | "re-imported" | "already-pending" | "already-enrolled" | "not-needed");
    if !queued {
        return Err(Failure::new(helper::reason_text(out.get("reason")), out.details()));
    }
    if out.get("build") != "installed" {
        // The key is queued, but nothing was installed: say why.
        return Err(Failure::new(
            helper::reason_text(out.get("reason")),
            format!("The key is queued for the next start, but the driver was not built.\n{}", out.details()),
        ));
    }
    Ok(())
}

/// Go back to the standard Linux driver for the running kernel.
pub fn restore_stock(sim: bool) -> Result<(), Failure> {
    if sim {
        sleep(Duration::from_millis(800));
        return Ok(());
    }
    let out = helper::restore_stock().map_err(|e| Failure::new("Could not restore the standard driver", e.to_string()))?;
    if helper::denied(out.code) {
        return Err(Failure::new("Permission was not granted", out.details()));
    }
    if out.get("restore") == "failed" || out.code != 0 {
        return Err(Failure::new("The standard driver could not be restored", out.details()));
    }
    Ok(())
}

// ---------------------------------------------------------------- uninstall

pub fn uninstall(sim: bool, emit: &mut dyn FnMut(Event)) -> Result<(), Failure> {
    emit(Event::StepStarted(0));
    emit(Event::Progress(None));
    if sim {
        for (pct, status) in [(20, "Waiting for permission"), (60, "Removing software"), (100, "Applying changes")] {
            emit(Event::Status(status.into()));
            emit(Event::Progress(Some(pct)));
            sleep(Duration::from_millis(800));
        }
        emit(Event::StepDone(0));
        return Ok(());
    }
    let client = Client::connect()?;
    let found = client.resolve(&[PACKAGE_APP, PACKAGE_DKMS], FILTER_INSTALLED)?;
    let ids: Vec<String> = found
        .iter()
        .filter(|p| p.name() == PACKAGE_APP || p.name() == PACKAGE_DKMS)
        .map(|p| p.id.clone())
        .collect();
    if !ids.is_empty() {
        client.remove_packages(&ids, &mut pk_forward(emit))?;
    }
    emit(Event::StepDone(0));
    Ok(())
}

// ---------------------------------------------------------------- after install

/// Ask logind to restart. `interactive` lets polkit ask if other users are logged in.
pub fn restart(sim: bool) -> Result<(), Failure> {
    if sim {
        return Ok(());
    }
    let conn = zbus::blocking::Connection::system().map_err(|e| Failure::new("Could not restart", e.to_string()))?;
    conn.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        "Reboot",
        &(true,),
    )
    .map_err(|e| Failure::new("Could not restart", e.to_string()))?;
    Ok(())
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
}

const RESUME_DESKTOP: &str = "openfuse-installer-resume.desktop";
const APP_DESKTOP: &str = "openfuse.desktop";

/// Reopen the installer after the restart to finish (and confirm) the setup.
/// The installer copies itself to a stable place first, in case the download
/// is moved or deleted.
pub fn schedule_resume(sim: bool) -> Result<(), Failure> {
    if sim {
        // Simulation changes nothing on the system, not even the user's autostart.
        return Ok(());
    }
    let err = |e: std::io::Error| Failure::new("Could not prepare the restart", e.to_string());
    let src = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or(std::env::current_exe().map_err(err)?);
    let dir = data_home().join("openfuse-installer");
    fs::create_dir_all(&dir).map_err(err)?;
    let dst = dir.join("openfuse-installer");
    if src != dst {
        fs::copy(&src, &dst).map_err(err)?;
    }
    let autostart = config_home().join("autostart");
    fs::create_dir_all(&autostart).map_err(err)?;
    fs::write(
        autostart.join(RESUME_DESKTOP),
        format!(
            "[Desktop Entry]\nType=Application\nName=OpenFuse Installer\n\
             Comment=Finish setting up OpenFuse\nExec=\"{}\" --resume\n\
             Icon=audio-card\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
            dst.display()
        ),
    )
    .map_err(err)?;
    Ok(())
}

pub fn clear_resume() {
    let _ = fs::remove_file(config_home().join("autostart").join(RESUME_DESKTOP));
    let _ = fs::remove_dir_all(data_home().join("openfuse-installer"));
}

/// The app's own "start in the tray at login" entry (same file the app manages).
pub fn set_app_autostart(enabled: bool) -> std::io::Result<()> {
    let dir = config_home().join("autostart");
    let path = dir.join(APP_DESKTOP);
    if enabled {
        fs::create_dir_all(&dir)?;
        fs::write(
            path,
            "[Desktop Entry]\nType=Application\nName=OpenFuse\n\
             Comment=Control your Arturia MiniFuse\nExec=openfuse --tray\nIcon=openfuse\n\
             X-GNOME-Autostart-enabled=true\n",
        )
    } else {
        match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

pub fn launch_app(sim: bool) -> Result<(), Failure> {
    if sim {
        return Ok(());
    }
    Command::new("openfuse")
        .spawn()
        .map(|_| ())
        .map_err(|e| Failure::new("Could not start OpenFuse", e.to_string()))
}

/// What is still missing after the restart, in words for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnosis {
    Ready,
    NotPluggedIn,
    KeyNotConfirmed,
    DriverNotActive,
}

pub fn diagnose(sim: bool) -> Diagnosis {
    if sim {
        // Simulation installs nothing, but the device detection is real.
        return if sysinfo::usb_devices().is_empty() { Diagnosis::NotPluggedIn } else { Diagnosis::Ready };
    }
    if sysinfo::openfuse_controls_present() {
        return Diagnosis::Ready;
    }
    let st = helper::status().unwrap_or_default();
    // Driver installed but refused at load because the key was never approved:
    // no USB audio driver is running at all.
    if st.get("signature_enforced") == "yes" && st.get("key_state") != "enrolled" && st.get("dkms_state") == "installed" {
        return Diagnosis::KeyNotConfirmed;
    }
    if sysinfo::usb_devices().is_empty() {
        Diagnosis::NotPluggedIn
    } else {
        Diagnosis::DriverNotActive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_version_comes_from_the_app_package_name() {
        assert_eq!(package_version("openfuse_0.2.0_amd64.deb", PACKAGE_APP).as_deref(), Some("0.2.0"));
        assert_eq!(package_version("openfuse_0.1.0-2_amd64.deb", PACKAGE_APP).as_deref(), Some("0.1.0-2"));
        assert_eq!(package_version("openfuse-dkms_0.2.0_all.deb", PACKAGE_APP), None, "the driver package is not the app");
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("openfuse-dkms_0.3.1_all.deb"), "").unwrap();
        std::fs::write(d.path().join("openfuse_0.3.1_amd64.deb"), "").unwrap();
        assert_eq!(bundled_version(Some(d.path())).as_deref(), Some("0.3.1"));
    }

    #[test]
    fn versions_compare_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("0.10.0", "0.9.0"), Greater);
        assert_eq!(compare_versions("0.1.0", "0.1.0"), Equal);
        assert_eq!(compare_versions("0.1", "0.1.0"), Equal);
        assert_eq!(compare_versions("0.1.0", "0.2.0"), Less);
        // Debian revisions: a rebuild of the same upstream version is newer
        assert_eq!(compare_versions("0.1.0-1", "0.1.0"), Greater);
        assert_eq!(compare_versions("0.1.0-2", "0.1.0-1"), Greater);
        assert_eq!(compare_versions("0.1.0-2", "0.1.1-1"), Less);
        assert_eq!(compare_versions("1.0.0-2", "1.0.0-1"), Greater);
    }

    #[test]
    fn app_autostart_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module run in one thread for this variable.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.path()) };
        set_app_autostart(true).unwrap();
        let file = tmp.path().join("autostart").join(APP_DESKTOP);
        assert!(fs::read_to_string(&file).unwrap().contains("Exec=openfuse --tray"));
        set_app_autostart(false).unwrap();
        assert!(!file.exists());
        set_app_autostart(false).unwrap(); // removing twice is fine
    }

    #[test]
    fn simulated_install_reports_all_steps() {
        let mut events = Vec::new();
        let out = install(None, true, &mut |e| events.push(e)).unwrap();
        assert!(out.needs_key);
        let done = events.iter().filter(|e| matches!(e, Event::StepDone(_) | Event::StepSkipped(_))).count();
        assert_eq!(done, INSTALL_STEPS.len());
    }

    #[test]
    fn missing_payload_dir_is_a_clear_failure() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("something-else_1.0_all.deb"), "").unwrap();
        let err = Payload::locate(Some(tmp.path())).err().unwrap();
        assert!(err.message.contains("No OpenFuse packages"), "other packages are never picked up");
        std::fs::write(tmp.path().join("openfuse-dkms_0.1.0_all.deb"), "").unwrap();
        std::fs::write(tmp.path().join("openfuse_0.1.0_amd64.deb"), "").unwrap();
        assert_eq!(Payload::locate(Some(tmp.path())).unwrap().files.len(), 2);
    }
}
