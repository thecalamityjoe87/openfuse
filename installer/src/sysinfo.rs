//! Read-only inspection of the system the installer runs on.
//!
//! Nothing here needs root or changes anything.

use std::fs;
use std::path::Path;
use std::process::Command;

pub const ARTURIA_VID: &str = "1c75";
/// Models OpenFuse can drive: (USB product id, name). Extend as models are characterised.
pub const SUPPORTED_MODELS: &[(&str, &str)] = &[("af90", "MiniFuse 2")];

pub const PACKAGE_DKMS: &str = "openfuse-dkms";
pub const PACKAGE_APP: &str = "openfuse";

/// Where DKMS keeps its module-signing certificate. Ubuntu uses shim-signed's
/// location, upstream DKMS its own; `/etc/dkms/framework.conf` can override both.
const MOK_CERT_CANDIDATES: &[&str] = &["/var/lib/shim-signed/mok/MOK.der", "/var/lib/dkms/mok.pub"];

/// Outcome of one check shown on the welcome page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Good,
    /// Not a problem, but something the user should know (e.g. an extra step).
    Note,
    /// Installation cannot continue.
    Blocker,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub label: &'static str,
    pub value: String,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct SystemInfo {
    pub distro_id: String,
    pub distro_like: Vec<String>,
    pub distro_name: String,
    pub kernel: String,
    /// `None`: not an EFI system / unknown.
    pub secure_boot: Option<bool>,
    pub packagekit: bool,
    pub pkexec: bool,
    pub mokutil: bool,
    pub headers: bool,
    pub devices: Vec<(String, String)>,
    pub installed_dkms: Option<String>,
    pub installed_app: Option<String>,
    pub controls_present: bool,
    pub mok_cert: Option<String>,
    /// `None`: no key yet, or cannot tell.
    pub mok_enrolled: Option<bool>,
}

impl SystemInfo {
    pub fn debian_like(&self) -> bool {
        matches!(self.distro_id.as_str(), "ubuntu" | "debian")
            || self.distro_like.iter().any(|d| d == "debian")
    }

    pub fn supported_distro(&self) -> bool {
        self.debian_like()
    }

    pub fn installed(&self) -> bool {
        self.installed_dkms.is_some() || self.installed_app.is_some()
    }

    /// Secure Boot is on and the DKMS signing key is not (known to be) enrolled.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn needs_key_enrollment(&self) -> bool {
        self.secure_boot == Some(true) && self.mok_enrolled != Some(true)
    }

    pub fn blockers(&self) -> Vec<Check> {
        self.checks().into_iter().filter(|c| c.status == Status::Blocker).collect()
    }

    pub fn checks(&self) -> Vec<Check> {
        let mut c = Vec::new();
        let distro = if self.distro_name.is_empty() { "Unknown Linux" } else { self.distro_name.as_str() };
        c.push(Check {
            label: "System",
            value: format!("{distro} · kernel {}", self.kernel.split('-').next().unwrap_or(&self.kernel)),
            status: if self.supported_distro() { Status::Good } else { Status::Blocker },
            detail: if self.supported_distro() {
                String::new()
            } else {
                "This installer currently supports Ubuntu and Debian based systems.".into()
            },
        });
        // Only worth a line when it is missing.
        if !self.packagekit {
            c.push(Check {
                label: "Software installation service",
                value: "missing".into(),
                status: Status::Blocker,
                detail: "Install the \"packagekit\" package, then run the installer again.".into(),
            });
        }
        c.push(Check {
            label: "Kernel build files",
            value: if self.headers { "installed" } else { "will be installed" }.into(),
            status: if self.headers { Status::Good } else { Status::Note },
            detail: String::new(),
        });
        match self.secure_boot {
            Some(true) => {
                let enrolled = self.mok_enrolled == Some(true);
                c.push(Check {
                    label: "Secure Boot",
                    value: "on".into(),
                    status: if enrolled { Status::Good } else { Status::Note },
                    detail: if enrolled {
                        "The signing key is already set up.".into()
                    } else {
                        "A signing key will be set up. This needs one restart.".into()
                    },
                });
                if !self.mokutil || !self.pkexec {
                    c.push(Check {
                        label: "Secure Boot tools",
                        value: "missing".into(),
                        status: Status::Blocker,
                        detail: "\"mokutil\" and \"pkexec\" are needed to set up the signing key.".into(),
                    });
                }
            }
            Some(false) => c.push(Check { label: "Secure Boot", value: "off".into(), status: Status::Good, detail: String::new() }),
            None => c.push(Check { label: "Secure Boot", value: "not used".into(), status: Status::Good, detail: String::new() }),
        }
        if self.devices.is_empty() {
            c.push(Check {
                label: "Audio interface",
                value: "not connected".into(),
                status: Status::Note,
                detail: "You can install now and plug it in later.".into(),
            });
        } else {
            let names: Vec<&str> = self.devices.iter().map(|(_, n)| n.as_str()).collect();
            c.push(Check {
                label: "Audio interface",
                value: format!("{} connected", names.join(", ")),
                status: Status::Good,
                detail: String::new(),
            });
        }
        c
    }
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

fn os_release() -> Vec<(String, String)> {
    let text = read("/etc/os-release").or_else(|| read("/usr/lib/os-release")).unwrap_or_default();
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.trim().trim_matches('"').to_string()))
        .collect()
}

fn secure_boot() -> Option<bool> {
    if !Path::new("/sys/firmware/efi").is_dir() {
        return None;
    }
    let dir = fs::read_dir("/sys/firmware/efi/efivars").ok()?;
    for entry in dir.flatten() {
        if entry.file_name().to_string_lossy().starts_with("SecureBoot-")
            && let Ok(raw) = fs::read(entry.path()) {
                // 4 bytes of attributes, then the 1-byte value.
                return Some(raw.len() >= 5 && raw[4] == 1);
            }
    }
    Some(false)
}

fn uname_release() -> String {
    read("/proc/sys/kernel/osrelease").unwrap_or_default()
}

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(cmd).is_file()))
        .unwrap_or(false)
}

fn run(cmd: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = Command::new(cmd).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

pub fn dpkg_version(pkg: &str) -> Option<String> {
    let (ok, out) = run("dpkg-query", &["-W", "-f=${Status}|${Version}", pkg])?;
    if !ok || !out.contains("install ok installed") {
        return None;
    }
    out.split_once('|').map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn packagekit_available() -> bool {
    Path::new("/usr/share/dbus-1/system-services/org.freedesktop.PackageKit.service").exists()
}

/// Connected supported devices: (USB product id, model name). Cheap; safe to poll.
pub fn usb_devices() -> Vec<(String, String)> {
    let mut found = Vec::new();
    let Ok(dir) = fs::read_dir("/sys/bus/usb/devices") else { return found };
    for entry in dir.flatten() {
        let p = entry.path();
        if read(p.join("idVendor")).as_deref() != Some(ARTURIA_VID) {
            continue;
        }
        let pid = read(p.join("idProduct")).unwrap_or_default();
        if let Some((_, name)) = SUPPORTED_MODELS.iter().find(|(id, _)| *id == pid) {
            found.push((pid, (*name).to_string()));
        }
    }
    found
}

pub fn mok_cert() -> Option<String> {
    let conf = read("/etc/dkms/framework.conf").unwrap_or_default();
    for line in conf.lines() {
        if let Some(v) = line.trim().strip_prefix("mok_certificate=") {
            let path = v.trim().trim_matches('"');
            if Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }
    MOK_CERT_CANDIDATES.iter().find(|p| Path::new(p).exists()).map(|p| p.to_string())
}

/// `Some(true/false)` when mokutil can tell, `None` when there is no key or no mokutil.
pub fn mok_enrolled(cert: Option<&str>) -> Option<bool> {
    let cert = cert?;
    let (_, out) = run("mokutil", &["--test-key", cert])?;
    let out = out.to_lowercase();
    if out.contains("is already enrolled") {
        Some(true)
    } else if out.contains("is not enrolled") {
        Some(false)
    } else {
        None
    }
}

/// Whether a supported card exposes the OpenFuse ALSA controls (module active).
pub fn openfuse_controls_present() -> bool {
    let Ok(dir) = fs::read_dir("/proc/asound") else { return false };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(num) = name.strip_prefix("card") else { continue };
        let usbid = read(entry.path().join("usbid")).unwrap_or_default().to_lowercase();
        if !usbid.starts_with(&format!("{ARTURIA_VID}:")) {
            continue;
        }
        if let Some((_, out)) = run("amixer", &["-c", num, "controls"])
            && out.contains("Phantom Power") {
                return true;
            }
    }
    false
}

pub fn gather() -> SystemInfo {
    let rel = os_release();
    let get = |k: &str| rel.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let kernel = uname_release();
    let mut info = SystemInfo {
        distro_id: get("ID"),
        distro_like: get("ID_LIKE").split_whitespace().map(String::from).collect(),
        distro_name: get("PRETTY_NAME"),
        headers: Path::new(&format!("/lib/modules/{kernel}/build")).is_dir(),
        kernel,
        secure_boot: secure_boot(),
        packagekit: packagekit_available(),
        pkexec: which("pkexec"),
        mokutil: which("mokutil"),
        devices: usb_devices(),
        installed_dkms: dpkg_version(PACKAGE_DKMS),
        installed_app: dpkg_version(PACKAGE_APP),
        ..Default::default()
    };
    info.controls_present = openfuse_controls_present();
    info.mok_cert = mok_cert();
    if info.secure_boot == Some(true) {
        info.mok_enrolled = mok_enrolled(info.mok_cert.as_deref());
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SystemInfo {
        SystemInfo {
            distro_id: "ubuntu".into(),
            distro_name: "Ubuntu 26.10".into(),
            kernel: "7.3.0-10-generic".into(),
            packagekit: true,
            pkexec: true,
            mokutil: true,
            headers: true,
            secure_boot: Some(false),
            ..Default::default()
        }
    }

    #[test]
    fn ubuntu_is_supported_and_unblocked() {
        let i = base();
        assert!(i.supported_distro());
        assert!(i.blockers().is_empty());
        assert!(!i.needs_key_enrollment());
    }

    #[test]
    fn debian_derivative_via_id_like() {
        let i = SystemInfo { distro_id: "linuxmint".into(), distro_like: vec!["ubuntu".into(), "debian".into()], ..base() };
        assert!(i.supported_distro());
    }

    #[test]
    fn fedora_is_blocked() {
        let i = SystemInfo { distro_id: "fedora".into(), distro_like: vec![], ..base() };
        assert_eq!(i.blockers().len(), 1);
    }

    #[test]
    fn missing_packagekit_blocks() {
        let i = SystemInfo { packagekit: false, ..base() };
        assert!(i.blockers().iter().any(|c| c.label == "Software installation service"));
    }

    #[test]
    fn secure_boot_needs_enrollment_until_enrolled() {
        let mut i = SystemInfo { secure_boot: Some(true), ..base() };
        assert!(i.needs_key_enrollment());
        i.mok_enrolled = Some(false);
        assert!(i.needs_key_enrollment());
        i.mok_enrolled = Some(true);
        assert!(!i.needs_key_enrollment());
    }

    #[test]
    fn secure_boot_without_tools_blocks() {
        let i = SystemInfo { secure_boot: Some(true), mokutil: false, ..base() };
        assert!(i.blockers().iter().any(|c| c.label == "Secure Boot tools"));
    }
}
