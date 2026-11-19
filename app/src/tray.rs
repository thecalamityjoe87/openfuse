//! System tray icon (StatusNotifierItem over D-Bus, via `ksni`).
//!
//! The menu mirrors the device: one checkable entry per live switch, updated
//! from the app whenever the device state changes (so hardware presses show
//! up too). Entries never flip themselves: they send a request to the app,
//! which goes through the same path as the cards, including the 48 V
//! confirmation.

use crate::autostart;
use crate::backend::Status;
use crate::icons;
use crate::ui::{App, External};
use iced::futures::channel::mpsc::UnboundedSender;
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip};

pub fn toggle_label(key: &str) -> Option<&'static str> {
    Some(match key {
        "in1_inst" => "INST 1 (Hi-Z)",
        "in2_inst" => "INST 2 (Hi-Z)",
        "phantom" => "48 V phantom power",
        "direct_mono" => "Direct Mono",
        "direct_monitor" => "Direct Monitor",
        "loopback" => "Loopback",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrayToggle {
    pub key: String,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
}

/// Everything the tray shows, computed from the app state.
#[derive(Debug, Clone, PartialEq)]
pub struct TraySnapshot {
    pub status_line: String,
    pub toggles: Vec<TrayToggle>,
    pub autostart: bool,
    pub notify_phantom: bool,
    /// "Ask before turning on 48 V"
    pub ask_phantom: bool,
    /// icon variant: 48 V on
    pub phantom: bool,
    /// icon variant: not connected / simulated / controls missing
    pub offline: bool,
    /// meters with a latched clip ("Input 1" ...)
    pub clipping: Vec<String>,
    /// "48 kHz · 64 frames · 1.3 ms · PipeWire", as on the main page's Audio button
    pub audio_line: Option<String>,
}

impl TraySnapshot {
    pub fn from_app(app: &App) -> TraySnapshot {
        let model = app.model;
        let mut keys: Vec<&str> = model.toggles.to_vec();
        if model.loopback.is_some() {
            keys.push("loopback");
        }
        let toggles: Vec<TrayToggle> = keys
            .iter()
            .map(|k| TrayToggle {
                key: k.to_string(),
                label: toggle_label(k).map(String::from).unwrap_or_else(|| model.label_for(k)),
                enabled: app.available.contains(*k) && !app.no_device(),
                checked: app.value_bool(k),
            })
            .collect();
        TraySnapshot {
            status_line: if app.status == Status::NoDevice {
                "No MiniFuse connected".to_string()
            } else {
                format!("{} — {}", model.name, app.status.text())
            },
            phantom: toggles.iter().any(|t| t.key == "phantom" && t.enabled && t.checked),
            offline: app.status != Status::Connected,
            clipping: app.clipping_labels().into_iter().map(String::from).collect(),
            audio_line: app.audio_summary().map(|s| {
                let set = if s.forced { " (set by OpenFuse)" } else { "" };
                format!("{} · {}{set}", s.text, s.source)
            }),
            toggles,
            autostart: autostart::autostart_enabled(),
            notify_phantom: app.settings.get_bool("notify_phantom_hardware", false),
            ask_phantom: app.ask_phantom(),
        }
    }

    pub fn tooltip(&self) -> String {
        let parts: Vec<String> = self
            .toggles
            .iter()
            .filter(|t| t.enabled)
            .map(|t| {
                let label = if t.key == "phantom" { "48 V" } else { t.label.split(" (").next().unwrap_or(&t.label) };
                format!("{label} {}", if t.checked { "on" } else { "off" })
            })
            .collect();
        let mut s = format!("{}\n{}", self.status_line, parts.join(" · "));
        if let Some(a) = &self.audio_line {
            s.push_str(&format!("\n{a}"));
        }
        if !self.clipping.is_empty() {
            s.push_str(&format!("\nclipping on {}", self.clipping.join(", ")));
        }
        s
    }
}

struct OpenFuseTray {
    snap: TraySnapshot,
    tx: UnboundedSender<External>,
    /// [plain, phantom, offline, phantom+offline]
    icons: [Vec<Icon>; 4],
}

impl OpenFuseTray {
    fn send(&self, e: External) {
        let _ = self.tx.unbounded_send(e);
    }
}

const INK_LIGHT: &str = "#ffffff";
const INK_DARK: &str = "#24292f";

/// Color of the monochrome tray glyph: it has to contrast with the panel,
/// which the tray protocol does not tell us. GNOME's top bar is always dark;
/// elsewhere the panel follows the desktop's light/dark preference.
/// `OPENFUSE_TRAY_ICON=light|dark` overrides (the glyph is light / dark).
fn tray_ink() -> &'static str {
    ink_for(
        std::env::var("OPENFUSE_TRAY_ICON").ok().as_deref(),
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        crate::ui::prefers_dark,
    )
}

fn ink_for(force: Option<&str>, desktop: Option<&str>, prefers_dark: impl FnOnce() -> bool) -> &'static str {
    match force {
        Some("light") => return INK_LIGHT,
        Some("dark") => return INK_DARK,
        _ => {}
    }
    let desktop = desktop.unwrap_or("").to_ascii_uppercase();
    if desktop.split(':').any(|d| d == "GNOME" || d == "UBUNTU") && !desktop.contains("KDE") {
        INK_LIGHT
    } else if prefers_dark() {
        INK_LIGHT
    } else {
        INK_DARK
    }
}

fn icon_set(phantom: bool, offline: bool, ink: &str) -> Vec<Icon> {
    [16, 22, 24, 32, 48, 64]
        .into_iter()
        .map(|s| {
            let r = icons::render_tray_icon(phantom, offline, s, ink);
            Icon { width: s as i32, height: s as i32, data: icons::argb32(&r) }
        })
        .collect()
}

impl ksni::Tray for OpenFuseTray {
    fn id(&self) -> String {
        "openfuse".into()
    }
    fn title(&self) -> String {
        "OpenFuse".into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }
    fn icon_pixmap(&self) -> Vec<Icon> {
        let i = usize::from(self.snap.phantom) + 2 * usize::from(self.snap.offline);
        self.icons[i].clone()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "OpenFuse".into(), description: self.snap.tooltip(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(External::ToggleWindow);
    }
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        // the panel went away: make sure the window can still be reached
        self.send(External::TrayOffline);
        true
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut m: Vec<MenuItem<Self>> =
            vec![StandardItem { label: self.snap.status_line.clone(), enabled: false, ..Default::default() }.into()];
        if let Some(a) = &self.snap.audio_line {
            m.push(StandardItem { label: a.clone(), enabled: false, ..Default::default() }.into());
        }
        m.push(MenuItem::Separator);
        for t in &self.snap.toggles {
            let key = t.key.clone();
            m.push(
                CheckmarkItem {
                    label: t.label.clone(),
                    enabled: t.enabled,
                    checked: t.checked,
                    activate: Box::new(move |this: &mut Self| this.send(External::Toggle(key.clone()))),
                    ..Default::default()
                }
                .into(),
            );
        }
        m.push(MenuItem::Separator);
        m.push(
            StandardItem {
                label: "Open OpenFuse".into(),
                activate: Box::new(|this: &mut Self| this.send(External::Open)),
                ..Default::default()
            }
            .into(),
        );
        m.push(
            StandardItem {
                label: "Audio settings…".into(),
                activate: Box::new(|this: &mut Self| this.send(External::OpenAudio)),
                ..Default::default()
            }
            .into(),
        );
        m.push(MenuItem::Separator);
        let auto = self.snap.autostart;
        m.push(
            CheckmarkItem {
                label: "Start at login".into(),
                checked: auto,
                activate: Box::new(move |this: &mut Self| this.send(External::SetAutostart(!auto))),
                ..Default::default()
            }
            .into(),
        );
        let ask = self.snap.ask_phantom;
        m.push(
            CheckmarkItem {
                label: "Ask before turning on 48 V".into(),
                checked: ask,
                activate: Box::new(move |this: &mut Self| this.send(External::SetAskPhantom(!ask))),
                ..Default::default()
            }
            .into(),
        );
        let notify = self.snap.notify_phantom;
        m.push(
            CheckmarkItem {
                label: "Notify when 48 V is switched on the device".into(),
                checked: notify,
                activate: Box::new(move |this: &mut Self| this.send(External::SetNotify(!notify))),
                ..Default::default()
            }
            .into(),
        );
        m.push(MenuItem::Separator);
        m.push(
            StandardItem {
                label: "Quit OpenFuse".into(),
                activate: Box::new(|this: &mut Self| this.send(External::Quit)),
                ..Default::default()
            }
            .into(),
        );
        m
    }
}

/// A running tray icon.
pub struct TrayLink {
    handle: ksni::blocking::Handle<OpenFuseTray>,
}

impl TrayLink {
    /// Register the tray icon. Fails when no StatusNotifierWatcher/Host is
    /// running (the desktop has no tray): the caller then keeps the window
    /// open and closing it quits.
    pub fn spawn(snap: TraySnapshot, tx: UnboundedSender<External>) -> Result<TrayLink, String> {
        let ink = tray_ink();
        let tray = OpenFuseTray {
            snap,
            tx,
            icons: [icon_set(false, false, ink), icon_set(true, false, ink), icon_set(false, true, ink), icon_set(true, true, ink)],
        };
        tray.spawn().map(|handle| TrayLink { handle }).map_err(|e| e.to_string())
    }

    pub fn update(&self, snap: TraySnapshot) {
        self.handle.update(move |t| t.snap = snap);
    }

    pub fn shutdown(&self) {
        let _ = self.handle.shutdown();
    }
}

/// Desktop notification (org.freedesktop.Notifications), fire and forget.
pub fn desktop_notification(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    let _ = std::thread::Builder::new().name("openfuse-notify".into()).spawn(move || {
        let _ = zbus::block_on(async move {
            let conn = zbus::Connection::session().await?;
            let hints: std::collections::HashMap<&str, zbus::zvariant::Value<'_>> = std::collections::HashMap::new();
            conn.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &("OpenFuse", 0u32, "audio-card", title.as_str(), body.as_str(), Vec::<&str>::new(), hints, 6000i32),
            )
            .await?;
            Ok::<(), zbus::Error>(())
        });
    });
}

#[cfg(test)]
mod ink_tests {
    use super::*;

    #[test]
    fn glyph_color_follows_the_panel() {
        let never = || -> bool { panic!("not needed") };
        assert_eq!(ink_for(Some("dark"), Some("GNOME"), never), INK_DARK);
        assert_eq!(ink_for(Some("light"), None, never), INK_LIGHT);
        // GNOME / Ubuntu top bars are dark whatever the app theme is
        assert_eq!(ink_for(None, Some("ubuntu:GNOME"), never), INK_LIGHT);
        // elsewhere the panel follows the desktop preference
        assert_eq!(ink_for(None, Some("KDE"), || true), INK_LIGHT);
        assert_eq!(ink_for(None, Some("KDE"), || false), INK_DARK);
        assert_eq!(ink_for(None, None, || false), INK_DARK);
    }
}
