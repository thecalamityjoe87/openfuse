//! GAIN calibration: the wizard in the Audio panel, saved per device.

mod common;
use common::*;
use iced::keyboard::key::Named;
use openfuse::autostart::Settings;
use openfuse::backend::mock::MockBackend;
use openfuse::calibrate::{Phase, RECORD_S, SETTLE_S};
use openfuse::knobs::{self, CAL_STEPS};
use openfuse::models;
use openfuse::ui::{App, KnobSource, Message, ThemeMode};

const QUIET: f64 = f64::NEG_INFINITY;
/// idle levels with a guitar plugged in at the five positions (9:00 made up)
const GUITAR: [f64; 5] = [-70.6, -62.0, -41.4, -26.6, -15.8];

fn app_at(path: std::path::PathBuf) -> App {
    let mut app = App::new(Box::new(MockBackend::still(&models::MINIFUSE2)), ThemeMode::Light, Settings::at(path));
    app.window_size = iced::Size::new(W, H);
    app.window = Some(iced::window::Id::unique());
    app.set_visible(true);
    app.start();
    app
}

/// Feed input 1 at `db` (input 2 quiet) for `secs`, 25 Hz, from `t0`; returns the end time.
fn feed(app: &mut App, t0: f64, secs: f64, db: f64) -> f64 {
    let mut t = t0;
    while t < t0 + secs {
        app.feed_gain_estimators(t, &[db, -95.0, QUIET, QUIET, QUIET, QUIET]);
        t += 0.04;
    }
    t
}

/// Press Record at `t` and run the recording to its end at a constant level.
fn record(app: &mut App, t: f64, db: f64) -> f64 {
    app.cal_record_at(t);
    feed(app, t, SETTLE_S + RECORD_S + 0.1, db)
}

fn calibrate_with_guitar(app: &mut App, mut t: f64) -> f64 {
    let _ = app.update(Message::CalStart("in1-gain"));
    for floor in GUITAR {
        t = record(app, t, floor);
        let _ = app.update(Message::CalNext);
    }
    t
}

#[test]
fn the_audio_panel_offers_calibration_per_gain_knob() {
    let mut app = app();
    app.set_audio_panel_open(true);
    assert!(shows(&app, "GAIN KNOB POSITION") && shows(&app, "GAIN 1") && shows(&app, "GAIN 2"));
    assert!(shows(&app, "Not calibrated") && !shows(&app, "Reset"));
    click_text(&mut app, "Calibrate…"); // the first one: GAIN 1
    let w = app.cal_wizard.as_ref().expect("wizard open");
    assert_eq!((w.knob, w.step), ("in1-gain", 0));
    assert!(shows(&app, "Calibrate GAIN 1 · INST off") && shows(&app, "Step 1 of 5") && shows(&app, "Record"));
    assert!(shows(&app, "Turn GAIN 1 to 7:30 o'clock") && shows(&app, "Fully down (minimum)"));
}

#[test]
fn a_full_calibration_replaces_the_estimate_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    let mut app = app_at(file.clone());
    assert!(!app.is_calibrated("in1-gain"));

    let t = calibrate_with_guitar(&mut app, 0.0);
    assert_eq!(app.cal_wizard.as_ref().unwrap().phase, Phase::Summary);
    assert!(shows(&app, "GAIN 1: all 5 positions recorded") && shows(&app, "−41.4 dBFS") && shows(&app, "Save"));
    let _ = app.update(Message::CalSave);
    assert!(app.cal_wizard.is_none() && app.is_calibrated("in1-gain") && !app.is_calibrated("in2-gain"));

    // no anchoring needed any more: the floor alone says where the knob is
    let mut t = t;
    for (step, floor) in CAL_STEPS.iter().zip(GUITAR) {
        t = feed(&mut app, t, 6.0, floor); // the display glides to a new level over a few seconds
        let k = app.knob_shown("in1-gain").unwrap();
        assert_eq!(k.source, KnobSource::Estimated);
        assert!((k.pos - step.0).abs() < 0.04, "{} o'clock: {}", step.1, k.pos);
    }
    app.set_audio_panel_open(true);
    assert!(shows(&app, "Calibrated with INST off") && shows(&app, "Reset"));

    // saved: a fresh start knows it
    let mut b = app_at(file.clone());
    assert!(b.is_calibrated("in1-gain") && !b.is_calibrated("in2-gain"));
    feed(&mut b, 0.0, 3.0, -41.4);
    assert!((b.knob_shown("in1-gain").unwrap().pos - 0.5).abs() < 0.03);

    // reset forgets it, here and after a restart
    let _ = b.update(Message::CalClear("in1-gain"));
    assert!(!b.is_calibrated("in1-gain"));
    assert!(!app_at(file).is_calibrated("in1-gain"));
}

#[test]
fn an_unsteady_recording_is_refused_and_nothing_is_saved_until_every_step_is_done() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    let mut app = app_at(file.clone());
    let _ = app.update(Message::CalStart("in1-gain"));
    app.cal_record_at(0.0);
    let mut t = 0.0;
    let mut i = 0;
    while app.cal_wizard.as_ref().unwrap().is_recording() {
        app.feed_gain_estimators(t, &[if i % 2 == 0 { -20.0 } else { -60.0 }, -95.0, QUIET, QUIET, QUIET, QUIET]);
        t += 0.04;
        i += 1;
    }
    assert!(matches!(app.cal_wizard.as_ref().unwrap().phase, Phase::Unsteady(_)));
    assert!(shows(&app, "The level wasn't steady (it varied by 40 dB). Keep the instrument still and try again.") && shows(&app, "Try again"));
    let _ = app.update(Message::CalNext);
    assert_eq!(app.cal_wizard.as_ref().unwrap().step, 0, "can't skip a bad recording");
    // the wizard is cancelled half way: nothing is stored
    let _ = app.update(Message::CalCancel);
    assert!(app.cal_wizard.is_none() && !app_at(file).is_calibrated("in1-gain"));
}

#[test]
fn a_knob_that_changes_nothing_is_explained_instead_of_saved() {
    let mut app = app();
    let _ = app.update(Message::CalStart("in1-gain"));
    let mut t = 0.0;
    for _ in CAL_STEPS {
        t = record(&mut app, t, -80.0); // the same level at every "position": the knob was not turned
        let _ = app.update(Message::CalNext);
    }
    let _ = app.update(Message::CalSave);
    let w = app.cal_wizard.as_ref().expect("still open");
    assert!(matches!(&w.phase, Phase::Failed(e) if e.contains("only changed by")));
    assert!(shows(&app, "Can't use this calibration") && !app.is_calibrated("in1-gain"));
}

#[test]
fn escape_cancels_and_the_dialog_is_modal() {
    let mut app = app();
    let _ = app.update(Message::CalStart("in2-gain"));
    assert_eq!(app.cal_wizard.as_ref().unwrap().meter, 1, "GAIN 2 reads meter word 1");
    let _ = app.update(Message::SetAbout(false));
    key(&mut app, Named::Escape);
    assert!(app.cal_wizard.is_none());
    // a click outside does not throw it away
    let _ = app.update(Message::CalStart("in1-gain"));
    click_at(&mut app, iced::Point::new(40.0, 400.0));
    assert!(app.cal_wizard.is_some());
}

#[test]
fn recording_shows_progress_and_the_live_reading() {
    let mut app = app();
    let _ = app.update(Message::CalStart("in1-gain"));
    feed(&mut app, 0.0, 0.2, -61.2);
    assert!(shows(&app, "Input now: −61.2 dBFS"));
    app.cal_record_at(0.2);
    assert!(app.cal_wizard.as_ref().unwrap().is_recording() && shows(&app, "Recording…"));
    let _ = knobs::percent(0.5);
}

/// Idle levels with nothing plugged in, INST off (measured).
const EMPTY: [f64; 5] = [-92.4, -92.2, -85.8, -72.9, -62.9];

fn calibrate(app: &mut App, mut t: f64, floors: [f64; 5]) -> f64 {
    let _ = app.update(Message::CalStart("in1-gain"));
    for floor in floors {
        t = record(app, t, floor);
        let _ = app.update(Message::CalNext);
    }
    let _ = app.update(Message::CalSave);
    t
}

#[test]
fn a_calibration_is_kept_per_inst_setting_and_the_matching_one_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    let mut app = app_at(file.clone());
    let set_inst = |app: &mut App, on: bool| {
        let _ = app.update(Message::Toggle("in1_inst".into(), on));
        let _ = app.update(Message::Tick);
    };

    // INST on (an instrument, or an empty jack): the wizard says which setting it records
    set_inst(&mut app, true);
    let _ = app.update(Message::CalStart("in1-gain"));
    assert!(shows(&app, "Calibrate GAIN 1 · INST on"));
    let _ = app.update(Message::CalCancel);
    let t = calibrate(&mut app, 0.0, GUITAR);
    assert!(app.calibrated_for("in1-gain", true) && !app.calibrated_for("in1-gain", false));

    // INST off: that calibration doesn't apply, and the card says so
    set_inst(&mut app, false);
    let t = feed(&mut app, t, 3.0, -85.8);
    assert!(app.knob_shown("in1-gain").is_none());
    assert!(shows(&app, "Calibrated with INST on"));

    // a second calibration with INST off is kept next to the first
    let t = calibrate(&mut app, t, EMPTY);
    assert!(app.calibrated_for("in1-gain", true) && app.calibrated_for("in1-gain", false));
    app.set_audio_panel_open(true);
    assert!(shows(&app, "Calibrated with INST off and on"));

    // the same floor means a different position depending on INST
    let t = feed(&mut app, t, 6.0, -72.9);
    assert!((app.knob_shown("in1-gain").unwrap().pos - 0.833).abs() < 0.04, "INST off: 3:00");
    set_inst(&mut app, true);
    feed(&mut app, t, 6.0, -41.4);
    assert!((app.knob_shown("in1-gain").unwrap().pos - 0.5).abs() < 0.04, "INST on: 12:00");

    // both survive a restart; Reset removes both
    let mut b = app_at(file.clone());
    assert!(b.calibrated_for("in1-gain", true) && b.calibrated_for("in1-gain", false));
    let _ = b.update(Message::CalClear("in1-gain"));
    assert!(!app_at(file).is_calibrated("in1-gain"));
}

#[test]
fn switching_inst_during_the_wizard_pauses_it() {
    let mut app = app();
    let _ = app.update(Message::CalStart("in1-gain"));
    assert!(shows(&app, "Calibrate GAIN 1 · INST off"));
    let _ = app.update(Message::Toggle("in1_inst".into(), true));
    let _ = app.update(Message::Tick);
    assert!(shows(&app, "INST was switched. Turn it back off to go on: this calibration is for INST off."));
}

#[test]
fn a_calibration_saved_before_inst_slots_is_moved_into_its_slot() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    let mut text: String = GUITAR.iter().enumerate().map(|(i, f)| format!("gaincal.minifuse2.in1-gain.{i}={f}\n")).collect();
    text.push_str("gaincal.minifuse2.in1-gain.inst=1\n");
    std::fs::write(&file, text).unwrap();
    let app = app_at(file.clone());
    assert!(app.calibrated_for("in1-gain", true) && !app.calibrated_for("in1-gain", false));
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("gaincal.minifuse2.in1-gain.inston.2=-41.4") && !saved.contains("in1-gain.inst="), "{saved}");
}

#[test]
fn dragging_a_calibrated_knob_corrects_the_calibration() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    let mut app = app_at(file.clone());
    let t = calibrate(&mut app, 0.0, GUITAR);
    // the guitar's level changed since: 12:00 now reads 6 dB higher than calibrated
    let t = feed(&mut app, t, 6.0, -35.4);
    assert!(app.knob_shown("in1-gain").unwrap().pos > 0.6);
    // the user puts the knob where it really is: shown at once, and followed from there
    let _ = app.update(Message::SetKnob("in1-gain", 0.5));
    let k = app.knob_shown("in1-gain").unwrap();
    assert_eq!((k.source, k.pos), (KnobSource::Estimated, 0.5));
    let t = feed(&mut app, t, 6.0, -20.6); // 3:00 with the same shift
    assert!((app.knob_shown("in1-gain").unwrap().pos - 0.833).abs() < 0.04);

    // kept after a restart; a double-click goes back to the calibration as recorded
    let mut b = app_at(file);
    let t = feed(&mut b, t, 6.0, -35.4);
    assert!((b.knob_shown("in1-gain").unwrap().pos - 0.5).abs() < 0.04);
    let _ = b.update(Message::ResetKnob("in1-gain"));
    feed(&mut b, t, 6.0, -35.4);
    assert!(b.knob_shown("in1-gain").unwrap().pos > 0.6);
}
