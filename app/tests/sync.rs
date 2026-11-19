//! State <-> UI synchronisation with the mock backend, clicking through the
//! real (headless-rendered) view.

mod common;
use common::*;
use openfuse::backend::mock::MockBackend;
use openfuse::backend::{Backend, Event, Status, Value};
use openfuse::levels::dbfs_to_raw;
use openfuse::models::{self, Kind, ModelSpec, View};
use openfuse::ui::{Message, loopback};
use std::any::Any;

#[test]
fn initial_state_reflected() {
    let mut app = app();
    for key in models::MINIFUSE2.toggles {
        assert!(!app.value_bool(key), "{key}");
    }
    // factory loopback defaults (spec §4.1): ch 5 (index 4) left at -3 dB
    assert_eq!(app.value_db("lb_vol_l_4"), Some(-3.0));
    assert_eq!(app.value_db("lb_vol_l_0"), Some(-63.0));
    assert!(non_meter_lit(&app).is_empty());
    let _ = app.update(Message::SetPanel(true));
    assert!(shows(&app, "−3.0 dB") && shows(&app, "off"));
}

#[test]
fn card_switch_toggles_backend_and_render() {
    let mut app = app();
    click_card_switch(&mut app, "in1-inst");
    assert_eq!(app.backend().get("in1_inst"), Some(Value::Bool(true)));
    assert!(app.value_bool("in1_inst"));
    assert!(app.lit_elements().contains(&"in1-inst".to_string()));
    click_card_switch(&mut app, "in1-inst");
    assert_eq!(app.backend().get("in1_inst"), Some(Value::Bool(false)));
    assert!(!app.lit_elements().contains(&"in1-inst".to_string()));
}

#[test]
fn clicking_part_on_render_toggles() {
    let mut app = app();
    click_part(&mut app, "direct-mono");
    assert_eq!(app.backend().get("direct_mono"), Some(Value::Bool(true)));
    assert!(app.value_bool("direct_mono"));
}

#[test]
fn hit_targets_are_larger_than_the_drawing() {
    let mut app = app();
    let r = app.part_rect("in2-inst").unwrap();
    // just outside the button's drawn edge still hits it
    click_at(&mut app, iced::Point::new((r.right() + 6.0) as f32, r.center().y as f32));
    assert!(app.value_bool("in2_inst"));
}

#[test]
fn hardware_press_updates_ui_and_flashes() {
    let mut app = app();
    app.mock().simulate_hardware_press("in2_inst");
    let _ = app.update(Message::Tick);
    assert!(app.value_bool("in2_inst"));
    assert!(app.is_flashing("in2-inst"));
    assert!(app.status_message().unwrap().contains("Changed on the device: INST 2 → ON"));
    assert!(app.lit_elements().contains(&"in2-inst".to_string()));
}

#[test]
fn own_change_does_not_flash() {
    let mut app = app();
    let _ = app.update(Message::Toggle("in1_inst".into(), true));
    assert!(app.value_bool("in1_inst") && !app.is_flashing("in1-inst"));
    assert!(app.status_message().is_none());
}

/// A device that ignores writes.
struct Deaf(MockBackend);

impl Backend for Deaf {
    fn model(&self) -> &'static ModelSpec {
        self.0.model()
    }
    fn simulated(&self) -> bool {
        true
    }
    fn status(&self) -> Status {
        self.0.status()
    }
    fn description(&self) -> String {
        self.0.description()
    }
    fn start(&mut self) -> Vec<Event> {
        self.0.start()
    }
    fn stop(&mut self) {}
    fn keys(&self) -> Vec<String> {
        self.0.keys()
    }
    fn get(&self, key: &str) -> Option<Value> {
        self.0.get(key)
    }
    fn set(&mut self, _: &str, _: Value) -> Vec<Event> {
        vec![]
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
    fn poll(&mut self) -> Vec<Event> {
        self.0.poll()
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn ui_only_changes_when_backend_confirms() {
    let mut app = app_with(Box::new(Deaf(MockBackend::still(&models::MINIFUSE2))));
    click_card_switch(&mut app, "direct-mono");
    assert!(!app.value_bool("direct_mono"));
    assert!(!app.lit_elements().contains(&"direct-mono".to_string()));
}

#[test]
fn meter_leds_follow_the_output_pair() {
    let mut app = app();
    // the front ladder shows meters 2/3 (output pair, models.rs meter table)
    assert_eq!(models::MINIFUSE2.meters.led_columns, &[("L", 2), ("R", 3)]);
    // -3 dBFS on output L: every row but the clip LED; output R silent (raw 0)
    app.mock().set_meter_raw(&[dbfs_to_raw(-3.0), dbfs_to_raw(-3.0), dbfs_to_raw(-3.0), 0]);
    let _ = app.update(Message::Tick);
    let lit = app.lit_elements();
    for r in 1..6 {
        assert!(lit.contains(&format!("meter-L-{r}")));
    }
    assert!(!lit.contains(&"meter-L-0".to_string()));
    assert!(!lit.iter().any(|e| e.starts_with("meter-R")));
    // clip lights the top (red) LED
    app.mock().set_meter_raw(&[0, 0, dbfs_to_raw(-0.2), 0]);
    let _ = app.update(Message::Tick);
    assert!(app.lit_elements().contains(&"meter-L-0".to_string()));
    // idle noise (spec §5): the ladder falls back smoothly, not at once
    app.mock().set_meter_raw(&[0xF300, 0x7500, 0xF300, 0x7500]);
    let _ = app.update(Message::Tick);
    assert!(app.lit_elements().contains(&"meter-L-5".to_string()), "decays, no flicker");
    app.advance_meters(5.0);
    assert!(app.lit_elements().is_empty());
    assert!(app.levels[1] < -90.0);
    // the input 1 meter reads "< −60 dB" (drawn on a canvas)
    assert_eq!(openfuse::levels::format_dbfs(app.levels[0], models::MINIFUSE2.meters.floor_db), "< −60 dB");
}

#[test]
fn lenses_follow_state_not_the_svg_default() {
    let mut app = app();
    // the SVG draws every lens lit; the off variant is what shows by default
    assert!(non_meter_lit(&app).is_empty());
    for (key, part) in [("in1_inst", "in1-inst"), ("in2_inst", "in2-inst"), ("direct_mono", "direct-mono")] {
        let _ = app.update(Message::Toggle(key.into(), true));
        assert!(app.lit_elements().contains(&part.to_string()), "{part} glows when on");
        let _ = app.update(Message::Toggle(key.into(), false));
        assert!(!app.lit_elements().contains(&part.to_string()), "{part} dark when off");
    }
    app.mock().simulate_hardware_press("phantom");
    let _ = app.update(Message::Tick);
    assert!(app.lit_elements().contains(&"phantom-48v".to_string()), "hardware press shows at once");
}

#[test]
fn meters_live_in_the_callout_cards_only() {
    use openfuse::models::MeterSource::{Mono, Stereo};
    let ms = &models::MINIFUSE2.meters;
    // inputs on the GAIN cards, the output pair on MONITOR / MIX / PHONES /
    // OUTPUTS / COMPUTER; the ladder card keeps its L/R bars
    assert_eq!(ms.card_meter("in1-gain"), Some(Mono(0)));
    assert_eq!(ms.card_meter("in2-gain"), Some(Mono(1)));
    for k in ["monitor", "mix", "hp1-level", "outputs", "usb-host"] {
        assert_eq!(ms.card_meter(k), Some(Stereo(2, 3)), "{k}");
    }
    assert!(ms.card_meter("midi").is_none() && ms.card_meter("usb-hub").is_none());
    let app = app();
    assert!(app.card_rect("meters").is_some(), "OUTPUT LEVEL card stays");
    assert!(shows(&app, "OUTPUT LEVEL"));
    // every card meter belongs to a part with a card, for every model
    for m in models::MODELS {
        for (k, _) in m.meters.card_meters {
            assert!(m.part(k).is_some(), "{}: {k}", m.key);
        }
    }
}

#[test]
fn loopback_panel_opens_from_usb_host() {
    let mut app = app();
    let _ = app.update(Message::SetView(View::Back));
    assert!(!app.panel_open);
    click_part(&mut app, "usb-host");
    assert!(app.panel_open && app.stage_layout().panel.is_some());
    click_text(&mut app, "Hide mixer");
    assert!(!app.panel_open);
    click_text(&mut app, "Loopback mixer");
    assert!(app.panel_open);
    // the L key and the panel's close button
    let _ = app.update(Message::SetPanel(false));
    assert!(!app.panel_open);
}

#[test]
fn opening_the_panel_from_the_front_switches_to_back() {
    let mut app = app();
    let _ = app.update(Message::SetPanel(true));
    assert_eq!(app.view, View::Back);
}

#[test]
fn loopback_controls_write_backend() {
    let mut app = app();
    let _ = app.update(Message::SetPanel(true));
    let lb = models::MINIFUSE2.loopback.unwrap();
    let _ = app.update(Message::SetGain("lb_vol_l_0".into(), loopback::db_for(lb, loopback::pos_for(lb, -6.0))));
    assert_eq!(app.backend().get("lb_vol_l_0"), Some(Value::Db(-6.0)));
    assert!(shows(&app, "−6.0 dB"));
    let _ = app.update(Message::SetGain("lb_vol_l_0".into(), loopback::db_for(lb, 0.0))); // bottom notch = off
    assert_eq!(app.backend().get("lb_vol_l_0"), Some(Value::Db(lb.gain_off_db)));
    click_text(&mut app, "Mute"); // first row
    assert_eq!(app.backend().get("lb_mute_0"), Some(Value::Bool(true)));
    assert!(shows(&app, "Muted"));
    let _ = app.update(Message::Toggle("loopback".into(), true));
    assert_eq!(app.backend().get("loopback"), Some(Value::Bool(true)));
    assert!(shows(&app, "LOOPBACK ON"));
}

#[test]
fn backend_change_updates_panel() {
    let mut app = app();
    let _ = app.update(Message::SetPanel(true));
    let _ = app.update(Message::SetGain("lb_vol_r_3".into(), 6.0));
    assert_eq!(app.value_db("lb_vol_r_3"), Some(6.0));
    assert!(shows(&app, "+6.0 dB"));
    let _ = app.update(Message::Toggle("loopback_to_monitor".into(), true));
    assert!(app.value_bool("loopback_to_monitor"));
}

#[test]
fn analog_parts_are_inert() {
    let mut app = app();
    let before: Vec<_> = app.backend().keys().iter().map(|k| app.backend().get(k)).collect();
    for part in models::MINIFUSE2.parts.iter().filter(|p| p.kind == Kind::Analog) {
        assert!(part.control.is_none());
        click_part(&mut app, part.key);
        // a click explains that the device can't report the knob, and moves nothing
        assert!(app.status_message().unwrap().contains("report"), "{}", part.key);
        assert!(app.knob_shown(part.key).is_none_or(|k| k.pos == openfuse::knobs::FIXED_POS), "{}: a click is not a turn", part.key);
    }
    let after: Vec<_> = app.backend().keys().iter().map(|k| app.backend().get(k)).collect();
    assert_eq!(before, after);
    assert!(shows(&app, "GAIN 1") && shows(&app, "Set on the device") && shows(&app, "ANALOG"));
}

struct Partial(MockBackend);

impl Backend for Partial {
    fn model(&self) -> &'static ModelSpec {
        self.0.model()
    }
    fn simulated(&self) -> bool {
        true
    }
    fn status(&self) -> Status {
        self.0.status()
    }
    fn description(&self) -> String {
        self.0.description()
    }
    fn start(&mut self) -> Vec<Event> {
        self.0.start()
    }
    fn stop(&mut self) {}
    fn keys(&self) -> Vec<String> {
        self.0.keys().into_iter().filter(|k| k != "direct_mono").collect()
    }
    fn get(&self, key: &str) -> Option<Value> {
        self.0.get(key)
    }
    fn set(&mut self, k: &str, v: Value) -> Vec<Event> {
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
    fn poll(&mut self) -> Vec<Event> {
        self.0.poll()
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn controls_missing_from_driver_are_disabled() {
    let mut app = app_with(Box::new(Partial(MockBackend::still(&models::MINIFUSE2))));
    assert!(shows(&app, "Not exposed by the driver"));
    click_card_switch(&mut app, "direct-mono");
    click_part(&mut app, "direct-mono");
    assert!(!app.value_bool("direct_mono"));
    click_card_switch(&mut app, "in1-inst");
    assert!(app.value_bool("in1_inst"));
    let snap = app.tray_snapshot();
    assert!(!snap.toggles.iter().find(|t| t.key == "direct_mono").unwrap().enabled);
}

#[test]
fn front_back_switch() {
    let mut app = app();
    click_text(&mut app, "Back");
    assert_eq!(app.view, View::Back);
    assert!(app.card_rect("usb-host").is_some() && app.card_rect("in1-inst").is_none());
    click_text(&mut app, "Front");
    assert_eq!(app.view, View::Front);
}

#[test]
fn previews_of_other_models_render() {
    for key in ["minifuse1", "minifuse4"] {
        let mut app = app_for(models::model_by_key(key).unwrap());
        assert!(shows(&app, "SIMULATED"));
        assert!(app.backend().description().contains("Simulated"));
        let _ = app.update(Message::SetView(View::Back));
        assert!(shows(&app, "COMPUTER"));
    }
}
