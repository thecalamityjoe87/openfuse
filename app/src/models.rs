//! Per-model data table.
//!
//! Everything the UI needs to know about a MiniFuse model lives here: how to
//! recognise it, which render to show, which SVG parts are live controls, which
//! are analog-only (inert), where their callouts go, and the loopback-mixer /
//! meter details. To add a model, add a [`ModelSpec`] to [`MODELS`].
//!
//! Logical control keys (`"phantom"`, `"lb_vol_l_3"` ...) are the vocabulary
//! shared by the UI and the backends. The mapping from these keys to ALSA
//! control names is in `alsa_controls.rs` (the one place to edit names).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    Front,
    Back,
}

impl View {
    pub fn name(self) -> &'static str {
        match self {
            View::Front => "front",
            View::Back => "back",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// live on/off button with a lens on the render
    Toggle,
    /// like Toggle, but asks for confirmation before enabling
    Phantom,
    /// live, read-only LED meter (the front-panel ladder)
    Meter,
    /// analog-only control: labelled, not controllable/readable
    Analog,
    /// socket: descriptive label only
    Connector,
    /// USB host port: opens the loopback mixer panel
    Loopback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Up,
    Down,
    Left,
    Right,
}

/// A callout on the device render.
///
/// * `ids`    SVG element ids (several = a group, anchored with a bracket).
/// * `side`   where the callout card sits: top or bottom of the render.
/// * `exit`   which edge of the part the leader line leaves from. Sideways
///   exits are used where printed legends sit directly under a button.
/// * `via_x`  for sideways exits: SVG x where the leader turns towards `side`.
/// * `pad`    SVG units between the part's edge and the leader start / bracket.
#[derive(Debug, Clone, Copy)]
pub struct Part {
    pub key: &'static str,
    pub view: View,
    pub kind: Kind,
    pub label: &'static str,
    pub hint: &'static str,
    pub ids: &'static [&'static str],
    pub control: Option<&'static str>,
    pub side: Side,
    pub exit: Option<Exit>,
    pub via_x: Option<f64>,
    pub pad: f64,
}

impl Part {
    const fn new(
        key: &'static str,
        view: View,
        kind: Kind,
        label: &'static str,
        hint: &'static str,
        ids: &'static [&'static str],
    ) -> Part {
        Part {
            key,
            view,
            kind,
            label,
            hint,
            ids,
            control: None,
            side: Side::Bottom,
            exit: None,
            via_x: None,
            pad: 6.0,
        }
    }
    /// Card above the render, leader leaving upwards.
    const fn top(mut self) -> Part {
        self.side = Side::Top;
        self.exit = Some(Exit::Up);
        self
    }
    const fn control(mut self, key: &'static str) -> Part {
        self.control = Some(key);
        self
    }
    /// Sideways exit, turning towards the card at SVG x `via_x`.
    const fn via(mut self, exit: Exit, via_x: f64) -> Part {
        self.exit = Some(exit);
        self.via_x = Some(via_x);
        self
    }
    const fn pad(mut self, pad: f64) -> Part {
        self.pad = pad;
        self
    }

    pub fn interactive(&self) -> bool {
        matches!(self.kind, Kind::Toggle | Kind::Phantom | Kind::Loopback)
    }
}

/// Which meter word(s) a display shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterSource {
    Mono(usize),
    /// left, right
    Stereo(usize, usize),
}

impl MeterSource {
    pub fn indices(self) -> Vec<usize> {
        match self {
            MeterSource::Mono(i) => vec![i],
            MeterSource::Stereo(l, r) => vec![l, r],
        }
    }
}

/// Level meters: THE table that says what each meter word means and which
/// displays show it. `labels` follow the order of the meter array (spec §5).
/// Values are dBFS (see levels.rs for the raw format).
#[derive(Debug)]
pub struct MeterSpec {
    pub labels: &'static [&'static str],
    /// callout cards that show a level meter (part key -> meter). Every
    /// audio-carrying part's card has one; parts without a source of their
    /// own show the closest one. The render itself shows level only through
    /// its LED ladder (`led_columns`).
    pub card_meters: &'static [(&'static str, MeterSource)],
    /// knobs whose light (the pointer line on the render) is a meter, as on
    /// the hardware's input GAIN knobs: dark without signal, blue rising with
    /// level, red while clipping (knob id -> meter)
    pub knob_lights: &'static [(&'static str, usize)],
    /// front-panel LED ladder columns -> index into the meter array (the
    /// render shows level only through these LEDs)
    pub led_columns: &'static [(&'static str, usize)],
    /// dBFS threshold per LED row, top (clip, red) to bottom. Our choice; the
    /// hardware ladder is not characterised.
    pub led_thresholds: &'static [f64],
    /// bottom of the on-screen bars
    pub floor_db: f64,
    pub ceil_db: f64,
    /// meters shown in the loopback panel (L, R), if any
    pub loopback_indices: &'static [usize],
    /// meters measured on the host from the playback stream (the device does
    /// not meter playback, spec §5). The device's own meter words fill the
    /// remaining entries, in order.
    pub host_outputs: &'static [usize],
}

impl MeterSpec {
    pub fn card_meter(&self, part_key: &str) -> Option<MeterSource> {
        self.card_meters.iter().find(|(k, _)| *k == part_key).map(|(_, m)| *m)
    }
}

const LED_THRESHOLDS: &[f64] = &[-1.0, -6.0, -12.0, -24.0, -36.0, -48.0];

#[derive(Debug)]
pub struct LoopbackChannel {
    pub label: &'static str,
    pub note: &'static str,
}

/// Loopback mixer (spec §4.1, §4.3, §4.4, §7.1). Mostly unverified.
#[derive(Debug)]
pub struct LoopbackSpec {
    pub channels: &'static [LoopbackChannel],
    /// unverified (Q3)
    pub gain_min_db: f64,
    /// unverified (Q3)
    pub gain_max_db: f64,
    /// value observed for "off" channels; slider bottom notch
    pub gain_off_db: f64,
    pub gain_step_db: f64,
    /// (control key, label, hint)
    pub switches: &'static [(&'static str, &'static str, &'static str)],
}

impl LoopbackSpec {
    pub fn keys(&self) -> Vec<String> {
        let mut ks: Vec<String> = self.switches.iter().map(|s| s.0.to_string()).collect();
        for n in 0..self.channels.len() {
            ks.push(format!("lb_vol_l_{n}"));
            ks.push(format!("lb_vol_r_{n}"));
            ks.push(format!("lb_mute_{n}"));
        }
        ks
    }
}

#[derive(Debug)]
pub struct ModelSpec {
    pub key: &'static str,
    pub name: &'static str,
    /// "vvvv:pppp" lower-case
    pub usb_ids: &'static [&'static str],
    /// matched against /proc/asound/cardN/id
    pub card_names: &'static [&'static str],
    pub parts: &'static [Part],
    pub meters: MeterSpec,
    pub loopback: Option<&'static LoopbackSpec>,
    /// logical boolean controls of the device
    pub toggles: &'static [&'static str],
    /// protocol characterised on real hardware?
    pub verified: bool,
    /// Factory defaults used by the simulator (dB for gains).
    pub defaults: &'static [(&'static str, f64)],
    svg_front: &'static str,
    svg_back: &'static str,
}

impl ModelSpec {
    /// The embedded render (assets/render/<key>_<view>.svg).
    pub fn svg(&self, view: View) -> &'static str {
        match view {
            View::Front => self.svg_front,
            View::Back => self.svg_back,
        }
    }

    pub fn parts_for(&self, view: View) -> impl Iterator<Item = &'static Part> + '_ {
        self.parts.iter().filter(move |p| p.view == view)
    }

    pub fn part(&self, key: &str) -> Option<&'static Part> {
        self.parts.iter().find(|p| p.key == key)
    }

    pub fn control_keys(&self) -> Vec<String> {
        let mut ks: Vec<String> = self.toggles.iter().map(|s| s.to_string()).collect();
        if let Some(lb) = self.loopback {
            ks.extend(lb.keys());
        }
        ks
    }

    pub fn part_for_control(&self, control: &str) -> Option<&'static Part> {
        self.parts.iter().find(|p| p.control == Some(control))
    }

    pub fn label_for(&self, control: &str) -> String {
        if let Some(p) = self.part_for_control(control) {
            return p.label.to_string();
        }
        if let Some(lb) = self.loopback {
            for (k, lab, _) in lb.switches {
                if *k == control {
                    return if *k == "loopback" { "Loopback".into() } else { format!("Loopback: {lab}") };
                }
            }
        }
        control.to_string()
    }
}

const ANALOG_HINT: &str = "Analog · set on the device";

const M0: MeterSource = MeterSource::Mono(0);
const M1: MeterSource = MeterSource::Mono(1);
const OUT12: MeterSource = MeterSource::Stereo(1, 2);
const OUT23: MeterSource = MeterSource::Stereo(2, 3);
const OUT45: MeterSource = MeterSource::Stereo(4, 5);

macro_rules! render {
    ($file:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/render/", $file))
    };
}

use Kind::*;
use View::*;

// ---------------------------------------------------------------- MiniFuse 2
pub static MINIFUSE2: ModelSpec = ModelSpec {
    key: "minifuse2",
    name: "MiniFuse 2",
    usb_ids: &["1c75:af90"],
    card_names: &["MiniFuse 2", "MiniFuse2", "M2"],
    verified: true,
    toggles: &["in1_inst", "in2_inst", "phantom", "direct_mono"],
    meters: MeterSpec {
        // Device words 0..3 (spec §5) fill the entries that are not host
        // outputs, in order: input 1, input 2, loopback L, loopback R (2/3
        // are most likely the loopback pair, unconfirmed). The output pair
        // is measured on the host from the playback stream.
        labels: &["Input 1", "Input 2", "Output L", "Output R", "Loopback L", "Loopback R"],
        host_outputs: &[2, 3],
        // the front LED ladder shows the output pair (flip to 0/1 if the
        // hardware ladder turns out to follow the inputs)
        led_columns: &[("L", 2), ("R", 3)],
        // MONITOR, MIX and the headphones are analog after the metering
        // point: they show the output pair (assumption)
        card_meters: &[
            ("in1-gain", M0),
            ("in2-gain", M1),
            ("monitor", OUT23),
            ("mix", OUT23),
            ("hp1-level", OUT23),
            ("outputs", OUT23),
            ("usb-host", OUT23),
        ],
        knob_lights: &[("in1-gain", 0), ("in2-gain", 1)],
        led_thresholds: LED_THRESHOLDS,
        floor_db: -60.0,
        ceil_db: 0.0,
        loopback_indices: &[4, 5],
    },
    loopback: Some(&LoopbackSpec {
        // Order unknown (spec §4.4, Q4). 4/5 look like a stereo playback pair.
        channels: &[
            LoopbackChannel { label: "Ch 1", note: "input 1" },
            LoopbackChannel { label: "Ch 2", note: "input 2" },
            LoopbackChannel { label: "Ch 3", note: "USB playback" },
            LoopbackChannel { label: "Ch 4", note: "USB playback" },
            LoopbackChannel { label: "Ch 5", note: "USB playback L" },
            LoopbackChannel { label: "Ch 6", note: "USB playback R" },
        ],
        gain_min_db: -60.0,
        gain_max_db: 12.0,
        gain_off_db: -63.0,
        gain_step_db: 0.5,
        switches: &[
            ("loopback", "Loopback", "Enable the loopback mix"),
            ("loopback_to_usb", "To USB In 1/2", "Send the mix to the computer's capture inputs 1/2"),
            ("loopback_to_monitor", "To monitor", "Hear the mix on speakers / phones"),
        ],
    }),
    // Factory state, spec §4.1
    defaults: &[("lb_vol_l_4", -3.0), ("lb_vol_r_5", -3.0)],
    parts: &[
        // ---- front, top row (knobs are analog-only, spec §4.2)
        Part::new("in1-gain", Front, Analog, "GAIN 1", ANALOG_HINT, &["in1-gain"]).top(),
        Part::new("in2-gain", Front, Analog, "GAIN 2", ANALOG_HINT, &["in2-gain"]).top(),
        Part::new("meters", Front, Meter, "OUTPUT LEVEL", "Front-panel meter", &[
            "meter-L-0", "meter-L-1", "meter-L-2", "meter-L-3", "meter-L-4", "meter-L-5",
            "meter-R-0", "meter-R-1", "meter-R-2", "meter-R-3", "meter-R-4", "meter-R-5",
        ]).top().pad(54.0),
        Part::new("monitor", Front, Analog, "MONITOR", ANALOG_HINT, &["monitor"]).top(),
        Part::new("mix", Front, Analog, "MIX", "Analog · input ↔ USB", &["mix"]).top(),
        Part::new("hp1-level", Front, Analog, "PHONES", ANALOG_HINT, &["hp1-level"]).top(),
        // ---- front, bottom row (live buttons). Legends sit under each
        // button, so leaders leave sideways and turn down in a clear gap.
        Part::new("in1-inst", Front, Toggle, "INST 1", "Hi-Z instrument input", &["in1-inst"])
            .control("in1_inst").via(Exit::Right, 506.0),
        Part::new("in2-inst", Front, Toggle, "INST 2", "Hi-Z instrument input", &["in2-inst"])
            .control("in2_inst").via(Exit::Right, 936.0),
        Part::new("phantom-48v", Front, Phantom, "48 V", "Phantom power, both inputs", &["phantom-48v"])
            .control("phantom").via(Exit::Right, 1052.0),
        Part::new("direct-mono", Front, Toggle, "DIRECT MONO", "Direct monitor in mono", &["direct-mono"])
            .control("direct_mono").via(Exit::Left, 1452.0),
        // ---- back
        Part::new("outputs", Back, Connector, "OUTPUTS", "Line out L / R", &["out-L", "out-R"]),
        Part::new("midi", Back, Connector, "MIDI", "DIN in / out", &["midi-in", "midi-out"]),
        Part::new("usb-hub", Back, Connector, "USB HUB", "USB-A", &["usb-hub1"]),
        Part::new("usb-host", Back, Loopback, "COMPUTER", "USB-C · loopback mixer", &["usb-host"]),
    ],
    svg_front: render!("minifuse2_front.svg"),
    svg_back: render!("minifuse2_back.svg"),
};

// ---------------------------------------------------------------- MiniFuse 1
// Not characterised yet (spec §1, Q6): no USB ID, so it is never auto-detected
// from USB; reachable with --mock --model minifuse1 for layout previews.
pub static MINIFUSE1: ModelSpec = ModelSpec {
    key: "minifuse1",
    name: "MiniFuse 1",
    usb_ids: &[],
    card_names: &["MiniFuse 1", "MiniFuse1", "M1"],
    verified: false,
    toggles: &["in1_inst", "phantom", "direct_monitor"],
    meters: MeterSpec {
        // preview assumption: input 1 and an output pair
        labels: &["Input 1", "Output L", "Output R"],
        host_outputs: &[1, 2],
        led_columns: &[("L", 1), ("R", 2)],
        card_meters: &[("in1-gain", M0), ("monitor", OUT12), ("hp1-level", OUT12), ("outputs", OUT12), ("usb-host", OUT12)],
        knob_lights: &[("in1-gain", 0)],
        led_thresholds: LED_THRESHOLDS,
        floor_db: -60.0,
        ceil_db: 0.0,
        loopback_indices: &[],
    },
    loopback: None,
    defaults: &[],
    parts: &[
        Part::new("in1-gain", Front, Analog, "GAIN", ANALOG_HINT, &["in1-gain"]).top(),
        Part::new("meters", Front, Meter, "OUTPUT LEVEL", "Front-panel meter", &[
            "meter-L-0", "meter-L-1", "meter-L-2", "meter-L-3", "meter-L-4", "meter-L-5",
            "meter-R-0", "meter-R-1", "meter-R-2", "meter-R-3", "meter-R-4", "meter-R-5",
        ]).top().pad(62.0),
        Part::new("monitor", Front, Analog, "MONITOR", ANALOG_HINT, &["monitor"]).top(),
        Part::new("hp1-level", Front, Analog, "PHONES", ANALOG_HINT, &["hp1-level"]).top(),
        Part::new("in1-inst", Front, Toggle, "INST", "Hi-Z instrument input", &["in1-inst"])
            .control("in1_inst").via(Exit::Right, 700.0),
        Part::new("phantom-48v", Front, Phantom, "48 V", "Phantom power", &["phantom-48v"])
            .control("phantom").via(Exit::Right, 850.0),
        Part::new("direct-monitor", Front, Toggle, "DIRECT MONITOR", "Direct monitoring", &["direct-monitor"])
            .control("direct_monitor").via(Exit::Left, 1370.0),
        Part::new("outputs", Back, Connector, "OUTPUTS", "Line out L / R", &["out-L", "out-R"]),
        Part::new("usb-hub", Back, Connector, "USB HUB", "USB-A", &["usb-hub1"]),
        Part::new("usb-host", Back, Connector, "COMPUTER", "USB-C · loopback not characterised", &["usb-host"]),
    ],
    svg_front: render!("minifuse1_front.svg"),
    svg_back: render!("minifuse1_back.svg"),
};

// ---------------------------------------------------------------- MiniFuse 4
pub static MINIFUSE4: ModelSpec = ModelSpec {
    key: "minifuse4",
    name: "MiniFuse 4",
    usb_ids: &[],
    card_names: &["MiniFuse 4", "MiniFuse4", "M4"],
    verified: false,
    toggles: &["in1_inst", "in2_inst", "phantom"],
    meters: MeterSpec {
        // preview assumption: four inputs and an output pair
        labels: &["Input 1", "Input 2", "Input 3", "Input 4", "Output L", "Output R"],
        host_outputs: &[4, 5],
        led_columns: &[("1", 0), ("2", 1), ("3", 2), ("4", 3)],
        card_meters: &[
            ("in1-gain", M0),
            ("in2-gain", M1),
            ("monitor", OUT45),
            ("phones", OUT45),
            ("inputs", MeterSource::Stereo(2, 3)),
            ("outputs", OUT45),
            ("usb-host", OUT45),
        ],
        knob_lights: &[("in1-gain", 0), ("in2-gain", 1)],
        led_thresholds: LED_THRESHOLDS,
        floor_db: -60.0,
        ceil_db: 0.0,
        loopback_indices: &[],
    },
    loopback: None,
    defaults: &[],
    parts: &[
        Part::new("in1-gain", Front, Analog, "GAIN 1", ANALOG_HINT, &["in1-gain"]).top(),
        Part::new("in2-gain", Front, Analog, "GAIN 2", ANALOG_HINT, &["in2-gain"]).top(),
        Part::new("meters", Front, Meter, "LEVEL", "Front-panel meter", &[
            "meter-1-0", "meter-1-1", "meter-1-2", "meter-1-3", "meter-1-4", "meter-1-5",
            "meter-2-0", "meter-2-1", "meter-2-2", "meter-2-3", "meter-2-4", "meter-2-5",
            "meter-3-0", "meter-3-1", "meter-3-2", "meter-3-3", "meter-3-4", "meter-3-5",
            "meter-4-0", "meter-4-1", "meter-4-2", "meter-4-3", "meter-4-4", "meter-4-5",
        ]).top().pad(42.0),
        Part::new("monitor", Front, Analog, "MONITOR", ANALOG_HINT, &["monitor"]).top(),
        Part::new("phones", Front, Analog, "PHONES 1 / 2", ANALOG_HINT, &["hp1-level", "hp2-level"]).top().pad(26.0),
        // below the render: the top row is full on this model
        Part::new("func-button", Front, Analog, "FUNCTION", "Not characterised", &["func-button"]),
        Part::new("in1-inst", Front, Toggle, "INST 1", "Hi-Z instrument input", &["in1-inst"])
            .control("in1_inst").via(Exit::Right, 452.0),
        Part::new("in2-inst", Front, Toggle, "INST 2", "Hi-Z instrument input", &["in2-inst"])
            .control("in2_inst").via(Exit::Right, 822.0),
        Part::new("phantom-48v", Front, Phantom, "48 V", "Phantom power", &["phantom-48v"])
            .control("phantom").via(Exit::Right, 925.0),
        Part::new("inputs", Back, Connector, "INPUTS 3 / 4", "Line in", &["in3-jack", "in4-jack"]),
        Part::new("outputs", Back, Connector, "OUTPUTS", "Line out 1–4", &["out-1", "out-2", "out-3", "out-4"]),
        Part::new("midi", Back, Connector, "MIDI", "DIN in / out", &["midi-in", "midi-out"]),
        Part::new("usb-hub", Back, Connector, "USB HUB", "USB-A ×2", &["usb-hub1", "usb-hub2"]),
        Part::new("usb-host", Back, Connector, "COMPUTER", "USB-C · loopback not characterised", &["usb-host"]),
        Part::new("dc-in", Back, Connector, "DC IN", "Power", &["dc-in"]),
    ],
    svg_front: render!("minifuse4_front.svg"),
    svg_back: render!("minifuse4_back.svg"),
};

/// All models, MiniFuse 2 first (the default for simulation).
pub static MODELS: [&ModelSpec; 3] = [&MINIFUSE2, &MINIFUSE1, &MINIFUSE4];

pub fn model_by_key(key: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().copied().find(|m| m.key == key)
}

pub fn model_for_usb_id(usbid: &str) -> Option<&'static ModelSpec> {
    let id = usbid.trim().to_ascii_lowercase();
    MODELS.iter().copied().find(|m| m.usb_ids.contains(&id.as_str()))
}

/// Fallback match on the ALSA card name (for models without a known PID).
pub fn model_for_card_name(name: &str) -> Option<&'static ModelSpec> {
    let norm = |s: &str| s.trim().to_ascii_lowercase().replace(' ', "");
    let n = norm(name);
    MODELS.iter().copied().find(|m| m.card_names.iter().any(|c| norm(c) == n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alsa_controls;

    #[test]
    fn every_control_has_an_alsa_name() {
        for m in MODELS {
            for key in m.control_keys() {
                assert!(alsa_controls::lookup(&key).is_some(), "{key}");
            }
        }
        assert_eq!(alsa_controls::lookup("phantom").unwrap().name, "Phantom Power Switch");
        assert_eq!(alsa_controls::lookup("lb_vol_r_5").unwrap().name, "Loopback Ch 5 Right Volume");
    }

    #[test]
    fn lookups() {
        assert_eq!(model_for_usb_id("1C75:AF90\n").unwrap().key, "minifuse2");
        assert!(model_for_usb_id("1c75:0000").is_none());
        assert_eq!(model_for_card_name("MiniFuse4").unwrap().key, "minifuse4");
        assert_eq!(model_for_card_name("minifuse 1").unwrap().key, "minifuse1");
        assert_eq!(MINIFUSE2.label_for("loopback_to_usb"), "Loopback: To USB In 1/2");
        assert_eq!(MINIFUSE2.label_for("phantom"), "48 V");
    }
}
