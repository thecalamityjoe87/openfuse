//! Virtual knobs: the device can't report its analog knobs. GAIN is shown where
//! the user set it and followed through the input's idle noise; MONITOR, MIX
//! and PHONES are drawn at one fixed position.

mod common;
use common::*;
use iced::mouse;
use openfuse::autostart::Settings;
use openfuse::backend::mock::MockBackend;
use openfuse::knobs;
use openfuse::models::{self, View};
use openfuse::ui::{App, KnobSource, Message, ThemeMode};

fn app_with_settings(path: std::path::PathBuf) -> App {
    let mut app = App::new(Box::new(MockBackend::still(&models::MINIFUSE2)), ThemeMode::Light, Settings::at(path));
    app.window_size = iced::Size::new(W, H);
    app.window = Some(iced::window::Id::unique());
    app.set_visible(true);
    app.start();
    app
}

const QUIET: f64 = f64::NEG_INFINITY;

/// Seconds of idle noise at `db` on input 1, fed at 25 Hz from `t0`; returns the end time.
fn idle(app: &mut App, t0: f64, secs: f64, db: f64) -> f64 {
    let mut t = t0;
    while t < t0 + secs {
        app.feed_gain_estimators(t, &[db, -95.0, QUIET, QUIET, QUIET, QUIET]);
        t += 0.04;
    }
    t
}

#[test]
fn only_the_gain_knobs_turn_and_the_others_are_fixed() {
    let mut app = app();
    for k in ["in1-gain", "in2-gain"] {
        assert!(app.is_knob(k), "{k}");
        assert!(app.knob_shown(k).is_none(), "{k}: nothing is shown until the user sets it");
    }
    // MONITOR, MIX, PHONES: nothing can tell where they are, so they sit at one fixed position
    for k in ["monitor", "mix", "hp1-level"] {
        assert!(!app.is_knob(k), "{k}");
        let _ = app.update(Message::SetKnob(k, 0.9));
        let s = app.knob_shown(k).unwrap();
        assert_eq!((s.source, s.pos), (KnobSource::Fixed, knobs::FIXED_POS), "{k}");
        assert_eq!(app.knob_note(k), None, "{k}");
    }
    for k in ["in1-inst", "phantom-48v", "meters", "usb-host"] {
        assert!(!app.is_knob(k), "{k}");
    }
}

#[test]
fn setting_a_gain_knob_shows_it_on_the_card_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    let mut a = app_with_settings(file.clone());
    let _ = a.update(Message::SetKnob("in1-gain", 0.617));
    assert_eq!(a.knob_shown("in1-gain").unwrap().source, KnobSource::Manual);
    assert!(shows(&a, "By you · 62%"));

    let b = app_with_settings(file.clone());
    assert!((b.knob_shown("in1-gain").unwrap().pos - 0.617).abs() < 1e-9, "restored");
    assert!(b.knob_shown("in2-gain").is_none(), "other knobs untouched");

    let mut b = b;
    let _ = b.update(Message::ResetKnob("in1-gain"));
    assert!(b.knob_shown("in1-gain").is_none() && !shows(&b, "By you · 62%"));
    assert!(app_with_settings(file).knob_shown("in1-gain").is_none(), "the reset is saved too");
}

#[test]
fn positions_are_clamped() {
    let mut app = app();
    let _ = app.update(Message::SetKnob("in1-gain", 3.0));
    assert_eq!(app.knob_shown("in1-gain").unwrap().pos, 1.0);
    let _ = app.update(Message::SetKnob("in1-gain", -1.0));
    assert_eq!(app.knob_shown("in1-gain").unwrap().pos, 0.0);
}

#[test]
fn the_wheel_turns_a_gain_knob_but_not_a_fixed_one() {
    let mut app = app();
    let r = app.part_rect("in1-gain").expect("GAIN 1 knob on the front view");
    let start = app.knob_start("in1-gain");
    interact(&mut app, |sim| {
        sim.point_at(center(r));
        sim.simulate([iced::Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Lines { x: 0.0, y: 2.0 } })]);
    });
    let k = app.knob_shown("in1-gain").expect("the wheel set it");
    assert_eq!(k.source, KnobSource::Manual);
    assert!((k.pos - (start + 0.08).clamp(0.0, 1.0)).abs() < 1e-9, "{start} -> {}", k.pos);

    let r = app.part_rect("monitor").expect("monitor knob on the front view");
    interact(&mut app, |sim| {
        sim.point_at(center(r));
        sim.simulate([iced::Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Lines { x: 0.0, y: 2.0 } })]);
    });
    assert_eq!(app.knob_shown("monitor").unwrap().pos, knobs::FIXED_POS);
}

#[test]
fn the_learned_instrument_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    let mut a = app_with_settings(file.clone());
    let t = idle(&mut a, 0.0, 3.0, -70.6);
    let _ = a.update(Message::SetKnob("in1-gain", 0.0));
    assert_eq!(a.knob_shown("in1-gain").unwrap().source, KnobSource::Estimated);

    // next start: your setting until the input is read, then the estimate follows the floor again
    let mut b = app_with_settings(file);
    assert_eq!(b.knob_shown("in1-gain").unwrap().source, KnobSource::Manual);
    idle(&mut b, t, 4.0, -41.4);
    let k = b.knob_shown("in1-gain").unwrap();
    assert_eq!(k.source, KnobSource::Estimated);
    assert!((k.pos - 0.5).abs() < 0.03, "{}", k.pos);
}

#[test]
fn a_floor_buried_in_converter_noise_reads_as_a_bound_not_a_value() {
    let mut app = app();
    // nothing plugged in, set at 12:00: the preamp's own noise is learned
    let t = idle(&mut app, 0.0, 3.0, -85.5);
    let _ = app.update(Message::SetKnob("in1-gain", 0.5));
    // turned right down: the floor is the converter's, the knob is somewhere below about 9:30
    idle(&mut app, t, 4.0, -92.4);
    assert!(shows(&app, "Estimated · under 25%"));
    assert!(app.knob_shown("in1-gain").unwrap().pos < 0.25);
}

#[test]
fn with_a_guitar_low_gain_reads_as_a_number() {
    let mut app = app();
    let t = idle(&mut app, 0.0, 3.0, -41.4);
    let _ = app.update(Message::SetKnob("in1-gain", 0.5));
    // the guitar's noise stays well above the converter's: every position shows
    idle(&mut app, t, 4.0, -60.0);
    let k = app.knob_shown("in1-gain").unwrap();
    assert!(k.pos > 0.1 && k.pos < 0.3, "{}", k.pos);
    assert!(shows(&app, &format!("Estimated · {}%", knobs::percent(k.pos))));
}

#[test]
fn a_gain_knob_claims_no_position_until_it_is_set_or_calibrated() {
    let mut app = app();
    // a guitar at minimum gain reads like the empty-input table at ~80%: that must not be shown
    idle(&mut app, 0.0, 4.0, -70.6);
    assert!(app.knob_shown("in1-gain").is_none());
    assert!(!shows(&app, "Estimated"));
    // quiet noise at any level: still nothing to go on
    idle(&mut app, 4.0, 4.0, -85.5);
    assert!(app.knob_shown("in1-gain").is_none());
}

#[test]
fn gain_follows_the_floor_from_the_position_the_user_set() {
    let mut app = app();
    let t = idle(&mut app, 0.0, 3.0, -77.0);
    let _ = app.update(Message::SetKnob("in1-gain", 0.4));
    let k = app.knob_shown("in1-gain").unwrap();
    assert_eq!((k.source, (k.pos * 100.0).round() as u32), (KnobSource::Estimated, 40));

    // +7 dB on the floor is +7 dB of gain, through the measured gain curve
    let t = idle(&mut app, t, 5.0, -70.0);
    let want = knobs::pos_for_gain(knobs::gain_db(0.4) + 7.0);
    let k = app.knob_shown("in1-gain").unwrap();
    assert!((k.pos - want).abs() < 0.02, "{} vs {want}", k.pos);
    assert!(shows(&app, &format!("Estimated · {}%", knobs::percent(k.pos))));

    // a played phrase: held at the last good value, and says so
    let mut t = t;
    let mut i = 0;
    while t < 12.0 {
        let db = [-20.0, -42.0, -28.0, -50.0, -24.0][i % 5];
        app.feed_gain_estimators(t, &[db, -95.0, QUIET, QUIET, QUIET, QUIET]);
        t += 0.04;
        i += 1;
    }
    assert_eq!(app.knob_shown("in1-gain").unwrap().source, KnobSource::Held);
    assert!(shows(&app, &format!("Last known · {}%", knobs::percent(k.pos))));

    // set while playing: your setting is shown until the input is quiet, and what was learned is kept
    let _ = app.update(Message::SetKnob("in1-gain", 0.25));
    let k = app.knob_shown("in1-gain").unwrap();
    assert_eq!((k.source, k.pos), (KnobSource::Held, 0.25));

    // reset forgets everything
    let _ = app.update(Message::ResetKnob("in1-gain"));
    assert!(app.knob_shown("in1-gain").is_none());
}

#[test]
fn with_a_guitar_plugged_in_setting_the_knob_once_makes_it_follow_the_knob() {
    let mut app = app();
    // guitar at minimum gain: its own hum would read ~80% against the empty-input table
    let t = idle(&mut app, 0.0, 3.0, -70.6);
    assert!(app.knob_shown("in1-gain").is_none(), "no position is claimed from the level alone");

    // the user drags the on-screen knob to where the hardware is (minimum): that teaches the guitar's level
    let _ = app.update(Message::SetKnob("in1-gain", 0.0));
    let k = app.knob_shown("in1-gain").unwrap();
    assert_eq!(k.source, KnobSource::Estimated, "now followed, not frozen at the setting");
    assert!(k.pos < 0.02);

    // turning the hardware up: guitar floor -41.4 at 12 o'clock, -26.6 at 3 o'clock, -15.8 at maximum
    let t = idle(&mut app, t, 4.0, -41.4);
    let half = app.knob_shown("in1-gain").unwrap().pos;
    assert!((half - 0.5).abs() < 0.03);
    assert!(shows(&app, &format!("Estimated · {}%", knobs::percent(half))));
    let t = idle(&mut app, t, 4.0, -26.6);
    assert!((app.knob_shown("in1-gain").unwrap().pos - 0.833).abs() < 0.03);
    idle(&mut app, t, 4.0, -15.8);
    assert!((app.knob_shown("in1-gain").unwrap().pos - 1.0).abs() < 0.03);

    // reset forgets the guitar: back to the table for an empty input
    let _ = app.update(Message::ResetKnob("in1-gain"));
    assert!(app.knob_shown("in1-gain").is_none());
}

#[test]
fn setting_a_gain_knob_while_playing_just_shows_your_setting() {
    let mut app = app();
    let mut t = 0.0;
    // a played phrase: peaks all over the place
    for i in 0..100 {
        app.feed_gain_estimators(t, &[if i % 3 == 0 { -12.0 } else { -48.0 - (i % 5) as f64 * 3.0 }, -95.0, QUIET, QUIET, QUIET, QUIET]);
        t += 0.04;
    }
    let _ = app.update(Message::SetKnob("in1-gain", 0.6));
    let k = app.knob_shown("in1-gain").unwrap();
    assert_eq!((k.source, k.pos), (KnobSource::Manual, 0.6));
    assert!(shows(&app, "By you · 60%"));
}

#[test]
fn the_back_view_has_no_knobs_to_turn() {
    let mut app = app();
    app.set_view(View::Back);
    assert!(app.part_rect("monitor").is_none());
}

#[test]
fn a_no_device_start_shows_a_waiting_page_and_switches_when_a_minifuse_appears() {
    let proc_dir = tempfile::tempdir().unwrap();
    std::fs::write(proc_dir.path().join("cards"), " 0 [Generic        ]: HDA-Intel - HD-Audio Generic\n").unwrap();
    let be = openfuse::backend::detect::create_backend(false, None, None, proc_dir.path(), false);
    let mut app = App::new(be, ThemeMode::Light, Settings::at(proc_dir.path().join("settings.ini")));
    app.window_size = iced::Size::new(W, H);
    app.set_visible(true);
    app.start();
    app.watch_for_device(None, proc_dir.path().to_path_buf());
    assert!(app.no_device());
    assert!(shows(&app, "No MiniFuse connected") && !shows(&app, "SIMULATED") && !shows(&app, "GAIN 1"));

    // a MiniFuse 2 appears in /proc/asound (here without its controls: it can't be opened in a test)
    // card 31: a real card at that index would be opened, and tests must not touch hardware
    let card = proc_dir.path().join("card31");
    std::fs::create_dir(&card).unwrap();
    std::fs::write(card.join("usbid"), "1c75:af90\n").unwrap();
    std::fs::write(card.join("id"), "M2\n").unwrap();
    std::fs::write(proc_dir.path().join("cards"), "31 [M2             ]: USB-Audio - MiniFuse 2\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let _ = app.update(Message::Tick);
    assert!(!app.no_device(), "switched to the device that appeared: {:?}", app.status);
    assert!(!shows(&app, "No MiniFuse connected"));
}
