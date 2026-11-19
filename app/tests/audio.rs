//! Audio settings panel: sample rate and buffer size (simulated server clock).

mod common;
use common::*;
use iced::keyboard;
use openfuse::ui::{External, Message};

fn press(app: &mut openfuse::ui::App, c: &'static str) {
    let _ = app.update(Message::Key(keyboard::Event::KeyPressed {
        key: keyboard::Key::Character(c.into()),
        modified_key: keyboard::Key::Character(c.into()),
        physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    }));
}

#[test]
fn header_button_and_keys_open_and_close_the_panel() {
    let mut app = app();
    assert!(!app.audio_panel_open && !shows(&app, "Audio settings"));
    click_text(&mut app, "Audio");
    assert!(app.audio_panel_open && shows(&app, "Audio settings"));
    key(&mut app, iced::keyboard::key::Named::Escape);
    assert!(!app.audio_panel_open);
    press(&mut app, "a");
    assert!(app.audio_panel_open);
    press(&mut app, "a");
    assert!(!app.audio_panel_open);
}

#[test]
fn panel_works_on_both_views_without_disturbing_the_loopback_panel() {
    let mut app = app();
    app.set_panel_open(true);
    app.set_audio_panel_open(true);
    assert!(app.panel_open && app.audio_panel_open && shows(&app, "Audio settings") && shows(&app, "Loopback mixer"));
    app.set_view(openfuse::models::View::Front);
    assert!(app.audio_panel_open, "the audio panel is not tied to the back view");
}

#[test]
fn the_drop_downs_set_the_clock_and_automatic_releases_it() {
    use openfuse::ui::audio::{quantum_choices, rate_choices};
    let mut app = app();
    app.set_audio_panel_open(true);
    assert!(shows(&app, "Not set by OpenFuse: the audio server decides."));

    let _ = app.update(Message::SetClockRate(96_000));
    let _ = app.update(Message::SetClockQuantum(128));
    let c = app.audio.clock.unwrap();
    assert_eq!((c.force_rate, c.force_quantum), (96_000, 128));
    assert!(shows(&app, "Now: 96 kHz · 128 frames · 1.3 ms per buffer"));

    let _ = app.update(Message::SetClockRate(0));
    let _ = app.update(Message::SetClockQuantum(0));
    let c = app.audio.clock.unwrap();
    assert_eq!((c.force_rate, c.force_quantum), (0, 0));
    assert!(!c.forced() && shows(&app, "Not set by OpenFuse: the audio server decides."));

    // every rate the device supports, automatic first; buffers stay in the server's limits
    let rates: Vec<String> = rate_choices(&c).iter().map(|r| r.to_string()).collect();
    assert_eq!(rates, ["Automatic (48 kHz)", "44.1 kHz", "48 kHz", "88.2 kHz", "96 kHz", "176.4 kHz", "192 kHz"]);
    let sizes: Vec<String> = quantum_choices(&c).iter().map(|q| q.to_string()).collect();
    assert_eq!(sizes.first().map(String::as_str), Some("Automatic (1024 frames)"));
    assert!(sizes.contains(&"32 frames · 0.7 ms".to_string()) && sizes.contains(&"2048 frames · 42.7 ms".to_string()));
}

#[test]
fn the_main_page_shows_the_current_audio_settings() {
    let mut app = app();
    assert!(shows(&app, "48 kHz · 1024 frames · 21.3 ms") && shows(&app, "PipeWire"), "server defaults");
    let _ = app.update(Message::SetClockRate(96_000));
    let _ = app.update(Message::SetClockQuantum(128));
    assert!(shows(&app, "96 kHz · 128 frames · 1.3 ms") && shows(&app, "SET"));
    assert!(app.audio_summary().unwrap().forced);
}

#[test]
fn the_tray_shows_the_current_audio_settings() {
    let mut app = app();
    let snap = app.tray_snapshot();
    assert_eq!(snap.audio_line.as_deref(), Some("48 kHz · 1024 frames · 21.3 ms · PipeWire"));
    assert!(snap.tooltip().contains("48 kHz · 1024 frames · 21.3 ms · PipeWire"));
    let _ = app.update(Message::SetClockRate(96_000));
    let _ = app.update(Message::SetClockQuantum(128));
    assert_eq!(
        app.tray_snapshot().audio_line.as_deref(),
        Some("96 kHz · 128 frames · 1.3 ms · PipeWire (set by OpenFuse)")
    );
}

#[test]
fn tray_entry_opens_the_window_on_the_audio_settings() {
    let mut app = app();
    app.fake_tray = true;
    let _ = app.close_window();
    assert!(app.window.is_none());
    let _ = app.update(Message::External(External::OpenAudio));
    assert!(app.window.is_some() && app.visible && app.audio_panel_open);
}

/// A device whose playback stream is held by another app.
struct Playing(openfuse::backend::mock::MockBackend, openfuse::alsa_stream::StreamInfo, bool);

impl openfuse::backend::Backend for Playing {
    fn model(&self) -> &'static openfuse::models::ModelSpec {
        self.0.model()
    }
    fn simulated(&self) -> bool {
        true
    }
    fn status(&self) -> openfuse::backend::Status {
        self.0.status()
    }
    fn description(&self) -> String {
        self.0.description()
    }
    fn start(&mut self) -> Vec<openfuse::backend::Event> {
        self.0.start()
    }
    fn stop(&mut self) {}
    fn keys(&self) -> Vec<String> {
        self.0.keys()
    }
    fn get(&self, key: &str) -> Option<openfuse::backend::Value> {
        self.0.get(key)
    }
    fn set(&mut self, k: &str, v: openfuse::backend::Value) -> Vec<openfuse::backend::Event> {
        self.0.set(k, v)
    }
    fn meters(&self) -> Vec<f64> {
        self.0.meters()
    }
    fn set_live(&mut self, on: bool) {
        self.0.set_live(on)
    }
    fn meters_polling(&self) -> bool {
        self.0.meters_polling()
    }
    fn playback_stream(&self) -> Option<openfuse::alsa_stream::StreamInfo> {
        Some(self.1.clone())
    }
    fn output_metered(&self) -> bool {
        self.2
    }
    fn poll(&mut self) -> Vec<openfuse::backend::Event> {
        self.0.poll()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn playing(via_server: bool, driver_meter: bool) -> openfuse::ui::App {
    let info = openfuse::alsa_stream::StreamInfo {
        owner: "Lithium Studio".into(),
        via_server,
        rate: 48_000,
        period: 64,
        buffer: 256,
        channels: 4,
    };
    app_with(Box::new(Playing(openfuse::backend::mock::MockBackend::still(&openfuse::models::MINIFUSE2), info, driver_meter)))
}

#[test]
fn a_direct_alsa_app_is_named_instead_of_offering_controls_that_do_not_apply() {
    let mut app = playing(false, false);
    assert!(shows(&app, "Lithium Studio plays directly to the MiniFuse (ALSA): output level can't be shown, only the inputs"));
    app.set_audio_panel_open(true);
    assert!(shows(&app, "Lithium Studio is using the MiniFuse directly (ALSA)"));
    assert!(shows(&app, "48 kHz · 64-frame period (1.3 ms) · 256-frame buffer"));
}

#[test]
fn a_server_stream_gets_no_warning() {
    let mut app = playing(true, false);
    assert!(!shows(&app, "Lithium Studio plays directly to the MiniFuse (ALSA): output level can't be shown, only the inputs"));
    app.set_audio_panel_open(true);
    assert!(shows(&app, "Lithium Studio is playing through the audio server"));
}

#[test]
fn no_warning_when_the_driver_meters_the_direct_stream() {
    let mut app = playing(false, true);
    assert!(!shows(&app, "Lithium Studio plays directly to the MiniFuse (ALSA): output level can't be shown, only the inputs"));
    app.set_audio_panel_open(true);
    assert!(shows(&app, "Lithium Studio is using the MiniFuse directly (ALSA)"), "still named: its rate and buffer are its own");
}

#[test]
fn a_direct_app_shows_what_it_opened_on_the_main_page() {
    let app = playing(false, true);
    assert!(shows(&app, "48 kHz · 64 frames · 1.3 ms") && shows(&app, "Lithium Studio"));
    assert!(app.audio_summary().unwrap().live);
}
