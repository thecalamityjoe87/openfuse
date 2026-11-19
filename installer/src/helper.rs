//! Client for `openfuse-setup`, the privileged helper shipped in the
//! `openfuse-dkms` package.
//!
//! Interface (see docs/kernel.md §8): one verb per call; `key=value` lines on
//! stdout; `status` needs no root, the other verbs run through `pkexec` (polkit
//! action `org.openfuse.setup`, `auth_admin_keep`).

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const HELPER: &str = "/usr/libexec/openfuse/openfuse-setup";

/// Parsed `key=value` output of one helper call.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub code: i32,
    pub values: HashMap<String, String>,
    pub stderr: String,
}

impl Output {
    pub fn get(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or("")
    }

    /// Everything the helper said, for the "Show details" box.
    pub fn details(&self) -> String {
        let mut keys: Vec<_> = self.values.iter().collect();
        keys.sort();
        let mut s: String = keys.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        s.push_str(&format!("exit={}\n", self.code));
        if !self.stderr.trim().is_empty() {
            s.push_str(self.stderr.trim());
        }
        s
    }
}

pub fn installed() -> bool {
    Path::new(HELPER).is_file()
}

fn parse(stdout: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn run(privileged: bool, verb: &str, stdin: Option<&str>) -> std::io::Result<Output> {
    let mut cmd = if privileged {
        let mut c = Command::new("pkexec");
        c.arg(HELPER);
        c
    } else {
        Command::new(HELPER)
    };
    cmd.arg(verb).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = cmd.spawn()?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok(Output {
        code: out.status.code().unwrap_or(-1),
        values: parse(&out.stdout),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Machine state; needs no privileges.
pub fn status() -> std::io::Result<Output> {
    run(false, "status", None)
}

/// Queue the signing key with `password`, then build and install the driver.
pub fn enroll(password: &str) -> std::io::Result<Output> {
    // Password twice, newline-separated, on stdin only.
    run(true, "enroll", Some(&format!("{password}\n{password}\n")))
}

/// (Re)build and install the driver for the running kernel.
pub fn build() -> std::io::Result<Output> {
    run(true, "build", None)
}

/// Remove the driver for the running kernel so the standard one is used again.
pub fn restore_stock() -> std::io::Result<Output> {
    run(true, "restore-stock", None)
}

/// pkexec's own exit codes when authentication was dismissed or refused.
pub fn denied(code: i32) -> bool {
    code == 126 || code == 127
}

/// Plain-language explanation for a helper `reason=` value.
pub fn reason_text(reason: &str) -> &'static str {
    match reason {
        "no-headers" => "The build files for your kernel are missing.",
        "key-not-enrolled" => "The Secure Boot key has not been approved yet.",
        "key-blocked" => "Your firmware refused the Secure Boot key.",
        "no-sources" => "The driver sources for your kernel could not be found, even offline.",
        "no-mokutil" => "The Secure Boot tool (mokutil) is missing.",
        "no-key" => "No signing key could be created.",
        "bad-password" => "The password was not accepted.",
        "mokutil-import" | "not-pending-after-import" => "The Secure Boot key could not be queued.",
        "dkms-add" | "dkms-install" => "The driver could not be built for your kernel.",
        _ => "Something went wrong while setting up the driver.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_value_lines_only() {
        let v = parse(b"version=0.1.0\nkey_state=pending\nnoise line\nBad=x\nlog=/var/lib/dkms/a=b\n");
        assert_eq!(v.get("version").unwrap(), "0.1.0");
        assert_eq!(v.get("key_state").unwrap(), "pending");
        assert_eq!(v.get("log").unwrap(), "/var/lib/dkms/a=b");
        assert!(!v.contains_key("Bad"));
        assert_eq!(v.len(), 3);
    }
}
