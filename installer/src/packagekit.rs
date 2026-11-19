//! Minimal blocking client for the PackageKit D-Bus API.
//!
//! PackageKit does the privileged work itself and asks the user for
//! authorization through polkit (the desktop's normal password dialog), so the
//! installer never runs as root for package operations.

use std::collections::HashMap;

use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type as MsgType;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::MatchRule;

const SERVICE: &str = "org.freedesktop.PackageKit";
const ROOT_PATH: &str = "/org/freedesktop/PackageKit";
const ROOT_IFACE: &str = "org.freedesktop.PackageKit";
const TX_IFACE: &str = "org.freedesktop.PackageKit.Transaction";

/// `PK_FILTER_ENUM_INSTALLED` as a bitfield.
pub const FILTER_INSTALLED: u64 = 1 << 2;
/// `PK_FILTER_ENUM_NOT_INSTALLED` as a bitfield.
pub const FILTER_NOT_INSTALLED: u64 = 1 << 3;
/// No transaction flags. Local `.deb` files are not repository-signed, so
/// `ONLY_TRUSTED` must not be set for them (PackageKit then asks for the
/// "install untrusted" authorization).
pub const TX_FLAGS_NONE: u64 = 0;

const EXIT_SUCCESS: u32 = 1;

#[derive(Debug, Clone)]
pub enum Event {
    /// 0–100, or `None` when PackageKit cannot estimate.
    Progress(Option<u8>),
    Status(&'static str),
    Package { id: String },
}

#[derive(Debug, Clone)]
pub struct Package {
    pub info: u32,
    pub id: String,
    pub summary: String,
}

impl Package {
    /// Package name part of a PackageKit id (`name;version;arch;repo`).
    pub fn name(&self) -> &str {
        self.id.split(';').next().unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct Error {
    pub message: String,
    pub details: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.details.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.message, self.details)
        }
    }
}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Self {
        let message = match &e {
            zbus::Error::MethodError(name, _, _) if name.as_str().contains("NotAuthorized")
                || name.as_str().contains("Refused") =>
            {
                "Permission was not granted".to_string()
            }
            _ => "Could not talk to the software installation service".to_string(),
        };
        Error { message, details: e.to_string() }
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(e: zbus::fdo::Error) -> Self {
        zbus::Error::from(e).into()
    }
}

/// Human-readable text for the PackageKit status values the user will see.
pub fn status_text(code: u32) -> &'static str {
    match code {
        2 => "Preparing",
        3..=5 => "Working",
        6 => "Removing software",
        7 | 20 | 21 | 22 | 27 => "Checking available software",
        8 => "Downloading",
        9 => "Installing",
        11 => "Cleaning up",
        13 => "Resolving dependencies",
        14 => "Checking signatures",
        15 | 16 => "Applying changes",
        30 => "Waiting for another software installation to finish",
        31 => "Waiting for permission",
        35 | 36 => "Setting up (building the driver can take a minute)",
        _ => "Working",
    }
}

pub struct Client {
    conn: Connection,
    locale: String,
}

impl Client {
    pub fn connect() -> Result<Self, Error> {
        let conn = Connection::system()?;
        let locale = std::env::var("LANG").unwrap_or_else(|_| "C".into());
        Ok(Client { conn, locale })
    }

    /// Run one PackageKit transaction: `call` invokes the method on the
    /// transaction proxy; signals are reported through `on_event` until the
    /// transaction finishes.
    fn transaction(
        &self,
        on_event: &mut dyn FnMut(Event),
        call: &dyn Fn(&Proxy<'_>) -> zbus::Result<()>,
    ) -> Result<Vec<Package>, Error> {
        let root = Proxy::new(&self.conn, SERVICE, ROOT_PATH, ROOT_IFACE)?;
        let path: OwnedObjectPath = root.call("CreateTransaction", &())?;

        // Subscribe before starting so no signal is missed.
        let rule = MatchRule::builder()
            .msg_type(MsgType::Signal)
            .sender(SERVICE)?
            .path(path.clone())?
            .build();
        let messages = MessageIterator::for_match_rule(rule, &self.conn, Some(256))?;

        let tx = Proxy::new(&self.conn, SERVICE, path.as_str(), TX_IFACE)?;
        let hints = vec!["interactive=true".to_string(), format!("locale={}", self.locale)];
        tx.call::<_, _, ()>("SetHints", &(hints,))?;
        call(&tx)?;

        let mut packages = Vec::new();
        let mut error: Option<Error> = None;
        for msg in messages {
            let msg = msg?;
            let header = msg.header();
            let member = header.member().map(|m| m.as_str().to_string()).unwrap_or_default();
            let body = msg.body();
            match member.as_str() {
                "Package" => {
                    if let Ok((info, id, summary)) = body.deserialize::<(u32, String, String)>() {
                        on_event(Event::Package { id: id.clone() });
                        packages.push(Package { info, id, summary });
                    }
                }
                "ErrorCode" => {
                    if let Ok((_code, details)) = body.deserialize::<(u32, String)>() {
                        error = Some(Error { message: "The software installation failed".into(), details });
                    }
                }
                "PropertiesChanged" => {
                    if let Ok((_iface, changed, _inv)) =
                        body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                    {
                        if let Some(v) = changed.get("Percentage").and_then(|v| u32::try_from(v).ok()) {
                            on_event(Event::Progress(if v <= 100 { Some(v as u8) } else { None }));
                        }
                        if let Some(v) = changed.get("Status").and_then(|v| u32::try_from(v).ok()) {
                            on_event(Event::Status(status_text(v)));
                        }
                    }
                }
                "Finished" => {
                    let exit = body.deserialize::<(u32, u32)>().map(|(e, _)| e).unwrap_or(0);
                    if exit == EXIT_SUCCESS {
                        return Ok(packages);
                    }
                    return Err(error.unwrap_or(Error {
                        message: if exit == 3 { "The installation was cancelled".into() } else {
                            "The software installation failed".into()
                        },
                        details: String::new(),
                    }));
                }
                _ => {}
            }
        }
        Err(Error { message: "Lost contact with the software installation service".into(), details: String::new() })
    }

    /// Look up packages by name. `filter` is a PackageKit filter bitfield.
    pub fn resolve(&self, names: &[&str], filter: u64) -> Result<Vec<Package>, Error> {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        self.transaction(&mut |_| {}, &|tx| tx.call("Resolve", &(filter, names.clone())))
    }

    /// Install local package files (e.g. `.deb`), pulling dependencies from the distribution.
    pub fn install_files(&self, files: &[String], on_event: &mut dyn FnMut(Event)) -> Result<(), Error> {
        let files = files.to_vec();
        self.transaction(on_event, &|tx| tx.call("InstallFiles", &(TX_FLAGS_NONE, files.clone())))
            .map(|_| ())
    }

    /// Install repository packages by PackageKit id.
    pub fn install_packages(&self, ids: &[String], on_event: &mut dyn FnMut(Event)) -> Result<(), Error> {
        let ids = ids.to_vec();
        self.transaction(on_event, &|tx| tx.call("InstallPackages", &(TX_FLAGS_NONE, ids.clone())))
            .map(|_| ())
    }

    /// Remove installed packages by PackageKit id.
    pub fn remove_packages(&self, ids: &[String], on_event: &mut dyn FnMut(Event)) -> Result<(), Error> {
        let ids = ids.to_vec();
        self.transaction(on_event, &|tx| tx.call("RemovePackages", &(TX_FLAGS_NONE, ids.clone(), true, false)))
            .map(|_| ())
    }
}
