//! Clip indication: threshold, 2 s latch, click-to-clear, ladder top LED,
//! GAIN knob lights, tray tooltip.

mod common;
use common::*;
use openfuse::levels::{CLIP_LATCH, dbfs_to_raw};
use openfuse::ui::{KnobLight, Message};
use std::time::{Duration, Instant};

const FULL: u32 = 0x8000_0000;

fn feed(app: &mut openfuse::ui::App, raw: [u32; 4]) {
    app.mock().set_meter_raw(&raw);
    let _ = app.update(Message::Tick);
}

#[test]
fn clip_latches_for_two_seconds() {
    let mut app = app();
    feed(&mut app, [dbfs_to_raw(-0.5), 0, 0, 0]);
    assert!(!app.is_clipping(0), "-0.5 dBFS is loud, not clipping");
    feed(&mut app, [FULL, 0, 0, 0]);
    assert!(app.is_clipping(0) && !app.is_clipping(1));
    feed(&mut app, [dbfs_to_raw(-30.0), 0, 0, 0]); // signal drops: stays latched
    let now = Instant::now();
    assert!(app.is_clipping_at(0, now + CLIP_LATCH - Duration::from_millis(100)));
    assert!(!app.is_clipping_at(0, now + CLIP_LATCH + Duration::from_millis(50)));
    assert_eq!(CLIP_LATCH, Duration::from_secs(2));
}

#[test]
fn clip_pill_on_the_gain_card_and_click_clears() {
    let mut app = app();
    assert!(!shows(&app, "CLIP"));
    feed(&mut app, [FULL, dbfs_to_raw(-20.0), 0, 0]);
    assert!(shows(&app, "CLIP"), "GAIN 1 card shows a CLIP pill");
    assert_eq!(app.knob_light("in1-gain"), Some(KnobLight::Clip));
    click_text(&mut app, "CLIP");
    assert!(!app.is_clipping(0) && !shows(&app, "CLIP"));
    assert!(matches!(app.knob_light("in1-gain"), Some(KnobLight::Signal(_))), "clearing the pill clears the knob light too");
}

#[test]
fn stereo_cards_name_the_clipping_channel() {
    let mut app = app();
    feed(&mut app, [0, 0, dbfs_to_raw(-10.0), FULL]);
    assert!(shows(&app, "CLIP R") && !shows(&app, "CLIP L R"));
    feed(&mut app, [0, 0, FULL, FULL]);
    assert!(shows(&app, "CLIP L R"));
}

#[test]
fn ladder_top_led_follows_the_latch() {
    let mut app = app();
    // the ladder shows meters 2/3: clip on output L
    feed(&mut app, [0, 0, FULL, 0]);
    assert!(app.lit_elements().contains(&"meter-L-0".to_string()));
    feed(&mut app, [0, 0, 0, 0]);
    app.advance_meters(1.0); // the bar falls ...
    let lit = app.lit_elements();
    assert!(lit.contains(&"meter-L-0".to_string()), "... the clip LED stays latched");
    assert!(!lit.contains(&"meter-L-1".to_string()));
    let _ = app.update(Message::ClearClip(vec![2]));
    assert!(!app.lit_elements().contains(&"meter-L-0".to_string()));
}

#[test]
fn gain_knob_light_idle_signal_clip() {
    let mut app = app();
    assert_eq!(app.knob_light("in1-gain"), Some(KnobLight::Off), "no signal: dark");
    assert_eq!(app.knob_light("monitor"), None, "other knobs have no light");
    feed(&mut app, [dbfs_to_raw(-40.0), dbfs_to_raw(-6.0), 0, 0]);
    let (Some(KnobLight::Signal(quiet)), Some(KnobLight::Signal(loud))) = (app.knob_light("in1-gain"), app.knob_light("in2-gain")) else {
        panic!("signal lights the knobs");
    };
    assert!(quiet > 0.0 && loud > quiet, "brightness rises with level");
    feed(&mut app, [FULL, dbfs_to_raw(-6.0), 0, 0]);
    assert_eq!(app.knob_light("in1-gain"), Some(KnobLight::Clip));
    // falls back to blue only after the latch
    feed(&mut app, [dbfs_to_raw(-40.0), dbfs_to_raw(-6.0), 0, 0]);
    assert_eq!(app.knob_light("in1-gain"), Some(KnobLight::Clip));
}

#[test]
fn tray_tooltip_mentions_clipping() {
    let mut app = app();
    feed(&mut app, [FULL, 0, 0, 0]);
    let snap = app.tray_snapshot();
    assert_eq!(snap.clipping, ["Input 1"]);
    assert!(snap.tooltip().contains("clipping on Input 1"));
    let _ = app.update(Message::ClearClip(vec![0]));
    assert!(!app.tray_snapshot().tooltip().contains("clipping"));
}
