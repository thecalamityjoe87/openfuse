//! The iced application: state, messages, update logic and the window view.
//!
//! The device is the source of truth. The UI never flips its own state: it
//! asks the backend, which writes, reads the value back and returns events.
//! Events returned by `Backend::set` are the read-back of our own request;
//! events from `Backend::poll` happened on the device by itself (hardware
//! buttons) and are highlighted.

pub mod about;
pub mod audio;
pub mod calwizard;
pub mod cards;
pub mod glyphs;
pub mod loopback;
pub mod meterbars;
pub mod phantom;
pub mod stage;
pub mod theme;

use crate::audioclock::{AudioClock, ClockControl, Memory, PipeWire};
use crate::autostart::{self, Settings};
use crate::backend::{Backend, Event, Status, Value};
use crate::geom::Rect;
use crate::calibrate::{Phase as CalPhase, Wizard as CalWizard};
use crate::knobs::{self, Calibration, GainEstimator};
use crate::layout::{self, StageLayout};
use crate::models::{Kind, ModelSpec, View};
use crate::render::DeviceRender;
use crate::tray::{TrayLink, TraySnapshot};
use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use iced::futures::{SinkExt, StreamExt};
use iced::widget::{button, column, container, mouse_area, responsive, row, space, stack, text};
use iced::{Border, Element, Length, Size, Subscription, Task, keyboard, window};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use theme::Tokens;

pub const ANALOG_TIP: &str = "The MiniFuse doesn't report this knob, so it is drawn at a fixed position. Use the knob on the device.";

pub const FLASH: Duration = Duration::from_millis(1400);
const HW_STATUS: Duration = Duration::from_secs(5);
/// How often /proc/asound is checked for a MiniFuse while none is connected.
const DETECT_INTERVAL: Duration = Duration::from_secs(2);
const ERR_STATUS: Duration = Duration::from_secs(8);
pub const TICK_VISIBLE: Duration = Duration::from_millis(40);
pub const TICK_HIDDEN: Duration = Duration::from_millis(250);

const KEY_CLOSE_HINT: &str = "tray_close_hint_shown";
const KEY_NOTIFY_PHANTOM: &str = "notify_phantom_hardware";
/// ask before switching 48 V on (the dialog's "Don't ask again" clears it)
pub const KEY_ASK_PHANTOM: &str = "ask_before_phantom";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Auto,
    Light,
    Dark,
}

/// Where a knob is drawn when it is not at the drawing's own angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnobShown {
    /// 0 = minimum, 1 = maximum
    pub pos: f64,
    pub source: KnobSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnobSource {
    /// set by the user: the device cannot report it
    Manual,
    /// GAIN, from the idle noise of the input meter
    Estimated,
    /// GAIN, the last estimate (a signal is present, so it is not updated)
    Held,
    /// MONITOR, MIX, PHONES: drawn at one fixed position, nothing can tell where they are
    Fixed,
}

/// State of a knob light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KnobLight {
    Off,
    /// brightness 0..1
    Signal(f32),
    Clip,
}

/// Which button of the 48 V dialog has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmButton {
    Cancel,
    Enable,
}

/// Requests from outside the window: tray menu, a second instance.
#[derive(Debug, Clone, PartialEq)]
pub enum External {
    /// a second launch: show the window
    Activate,
    /// tray icon clicked
    ToggleWindow,
    Open,
    Toggle(String),
    SetAutostart(bool),
    SetNotify(bool),
    /// "Ask before turning on 48 V"
    SetAskPhantom(bool),
    Quit,
    TrayOffline,
    /// tray: Audio settings…
    OpenAudio,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    /// request a switch change (key, requested value)
    Toggle(String, bool),
    /// request a loopback gain (key, dB)
    SetGain(String, f64),
    PartClicked(&'static str),
    HoverPart(Option<&'static str>),
    HoverCard(Option<&'static str>),
    SetView(View),
    SetPanel(bool),
    SetAudioPanel(bool),
    /// the About dialog
    SetAbout(bool),
    /// the on-screen position of an analog knob, 0..1 (the device cannot report it)
    SetKnob(&'static str, f64),
    /// back to automatic: the GAIN estimate, or the drawing's own pointer
    ResetKnob(&'static str),
    /// GAIN calibration wizard (the knob is the part key)
    CalStart(&'static str),
    CalRecord,
    CalNext,
    CalBack,
    CalSave,
    CalCancel,
    /// forget the user's calibration of this knob
    CalClear(&'static str),
    /// forced sample rate (Hz); 0 = automatic
    SetClockRate(u32),
    /// forced buffer size (frames); 0 = automatic
    SetClockQuantum(u32),
    /// answer of the 48 V dialog
    Confirm(bool),
    /// the dialog's "Don't ask again" checkbox
    ConfirmDontAsk(bool),
    /// "Ask before turning on" on the 48 V card
    SetAskPhantom(bool),
    ConfirmFocus(ConfirmButton),
    Key(keyboard::Event),
    WindowOpened(window::Id),
    CloseRequested(window::Id),
    WindowClosed(window::Id),
    Resized(window::Id, Size),
    ScaleFactor(f32),
    SystemTheme(iced::theme::Mode),
    External(External),
    /// move the window (press on an empty area)
    Drag,
    /// double-click on the header
    ToggleMaximize,
    Minimize,
    Maximized(bool),
    DragResize(window::Direction),
    /// name of the graphics backend in use
    Renderer(String),
    /// clear the clip indicators of these meter words (click on a CLIP pill)
    ClearClip(Vec<usize>),
    /// OPENFUSE_SNAPSHOT debug aid: a capture of the real window
    Snapshot(window::Screenshot),
}

#[derive(Default)]
struct Rasters {
    /// (view, width px, height px) -> base image
    base: Option<((View, u32, u32), iced::widget::image::Handle)>,
    /// (view, id, width px) -> sprite + its rect in SVG units
    sprites: HashMap<(View, String, u32), (iced::widget::image::Handle, Rect)>,
}

pub struct App {
    backend: Box<dyn Backend>,
    pub model: &'static ModelSpec,
    pub theme_mode: ThemeMode,
    pub system_dark: bool,
    pub view: View,
    pub values: HashMap<String, Value>,
    /// latest meter readings (dBFS)
    pub levels: Vec<f64>,
    /// what the meters display (smoothed, with peak hold)
    pub meter_view: Vec<crate::levels::Ballistics>,
    /// per meter word: time of the last clipping reading (latched for CLIP_LATCH)
    clip_at: Vec<Option<Instant>>,
    last_meter_update: Instant,
    pub available: HashSet<String>,
    pub panel_open: bool,
    /// audio settings slide-out (sample rate / buffer size)
    pub audio_panel_open: bool,
    pub about_open: bool,
    /// the open GAIN calibration wizard
    pub cal_wizard: Option<CalWizard>,
    /// knob positions set by the user (part key -> 0..1), saved in the settings file
    knob_manual: HashMap<&'static str, f64>,
    /// GAIN estimates from the idle noise of the input meters (knob -> estimator)
    gain_est: HashMap<&'static str, GainEstimator>,
    /// the user's GAIN calibrations, one per knob and INST setting (INST changes the input's noise)
    cals: HashMap<(&'static str, bool), Calibration>,
    /// the user's corrections of those calibrations (dB), from setting the knob by hand
    cal_offsets: HashMap<(&'static str, bool), f64>,
    /// clock of the estimators
    started_at: Instant,
    pub audio: AudioClock,
    /// the app icon, shown in the header
    logo: iced::widget::image::Handle,
    pub hover_part: Option<&'static str>,
    pub hover_card: Option<&'static str>,
    flashes: HashMap<&'static str, Instant>,
    status_msg: Option<(String, Instant)>,
    /// open 48 V dialog, with the focused button
    pub confirm: Option<ConfirmButton>,
    /// the dialog's "Don't ask again" box (unticked whenever the dialog opens)
    pub confirm_dont_ask: bool,
    pub window: Option<window::Id>,
    pub visible: bool,
    pub scale: f32,
    pub window_size: Size,
    renders: [DeviceRender; 2],
    rasters: RefCell<Rasters>,
    pub tray: Option<TrayLink>,
    last_tray: Option<TraySnapshot>,
    pub settings: Settings,
    /// notifications sent (title, body); also shown on the desktop when enabled
    pub notifications: Vec<(String, String)>,
    pub desktop_notifications: bool,
    pub status: Status,
    started: bool,
    pub maximized: bool,
    /// tests: behave as if a tray icon were registered
    #[doc(hidden)]
    pub fake_tray: bool,
    /// set once quit() ran (the runtime then exits)
    pub quitting: bool,
    ticks: u32,
    /// transparent window with rounded corners (Wayland + a renderer with alpha)
    pub rounded: bool,
    /// while no MiniFuse is connected: where to look for one (--card filter, /proc/asound)
    device_watch: Option<(Option<u32>, PathBuf)>,
    last_detect: Instant,
}

impl App {
    pub fn new(backend: Box<dyn Backend>, theme_mode: ThemeMode, settings: Settings) -> App {
        let model = backend.model();
        let status = backend.status();
        let mut app = App {
            model,
            theme_mode,
            system_dark: false,
            view: View::Front,
            values: HashMap::new(),
            levels: vec![model.meters.floor_db; model.meters.labels.len()],
            meter_view: vec![Default::default(); model.meters.labels.len()],
            clip_at: vec![None; model.meters.labels.len()],
            last_meter_update: Instant::now(),
            available: HashSet::new(),
            panel_open: false,
            audio_panel_open: false,
            about_open: false,
            cal_wizard: None,
            knob_manual: HashMap::new(),
            cals: HashMap::new(),
            cal_offsets: HashMap::new(),
            gain_est: model.meters.knob_lights.iter().map(|(k, _)| (*k, GainEstimator::new(Calibration::default()))).collect(),
            started_at: Instant::now(),
            // with no MiniFuse the audio server is still real; only a simulation (--mock) fakes it
            audio: AudioClock::new(if backend.simulated() && status != Status::NoDevice {
                Box::<Memory>::default() as Box<dyn ClockControl + Send>
            } else {
                Box::new(PipeWire)
            }),
            logo: {
                let i = crate::icons::render_icon(false, false, 96);
                iced::widget::image::Handle::from_rgba(i.width, i.height, i.rgba)
            },
            hover_part: None,
            hover_card: None,
            flashes: HashMap::new(),
            status_msg: None,
            confirm: None,
            confirm_dont_ask: false,
            window: None,
            visible: false,
            scale: 1.0,
            window_size: Size::new(1200.0, 760.0),
            renders: [DeviceRender::new(model.svg(View::Front)), DeviceRender::new(model.svg(View::Back))],
            rasters: RefCell::new(Rasters::default()),
            tray: None,
            last_tray: None,
            settings,
            notifications: Vec::new(),
            desktop_notifications: false,
            status,
            started: false,
            maximized: false,
            fake_tray: false,
            quitting: false,
            ticks: 0,
            rounded: false,
            device_watch: None,
            last_detect: Instant::now(),
            backend,
        };
        app.load_knobs();
        app
    }

    fn knob_key(&self, id: &str) -> String {
        format!("knob.{}.{}", self.model.key, id)
    }

    fn anchor_key(&self, knob: &str) -> String {
        format!("gainsrc.{}.{}", self.model.key, knob)
    }

    fn adc_key(&self, knob: &str) -> String {
        // (".v3": floors saved by earlier versions took short dips for the floor, too low)
        format!("adcfloor.v3.{}.{}", self.model.key, knob)
    }

    fn cal_key(&self, knob: &str, inst: bool, i: usize) -> String {
        format!("gaincal.{}.{}.inst{}.{}", self.model.key, knob, if inst { "on" } else { "off" }, i)
    }

    fn cal_offset_key(&self, knob: &str, inst: bool) -> String {
        format!("gaincal.{}.{}.inst{}.offset", self.model.key, knob, if inst { "on" } else { "off" })
    }

    /// Save (or with 0, remove) the correction of a knob's calibration for an INST setting.
    fn store_cal_offset(&mut self, knob: &'static str, inst: bool, db: f64) {
        let v = (db != 0.0).then_some(db);
        match v {
            Some(v) => self.cal_offsets.insert((knob, inst), v),
            None => self.cal_offsets.remove(&(knob, inst)),
        };
        let _ = self.settings.set_f64(&self.cal_offset_key(knob, inst), v);
    }

    /// Before calibrations were kept per INST setting: one set, and the INST it was made with.
    fn legacy_cal_key(&self, knob: &str, i: Option<usize>) -> String {
        match i {
            Some(i) => format!("gaincal.{}.{}.{}", self.model.key, knob, i),
            None => format!("gaincal.{}.{}.inst", self.model.key, knob),
        }
    }

    /// The INST switch of a GAIN knob's input, if the model has one.
    fn inst_control(&self, knob: &str) -> Option<&'static str> {
        let c = match knob {
            "in1-gain" => "in1_inst",
            "in2-gain" => "in2_inst",
            _ => return None,
        };
        self.model.toggles.contains(&c).then_some(c)
    }

    /// The INST switch of a GAIN knob's input right now (None: it has none).
    pub fn inst_on(&self, knob: &str) -> Option<bool> {
        self.inst_control(knob).map(|c| self.value_bool(c))
    }

    /// Is there a calibration of this knob for this INST setting?
    pub fn calibrated_for(&self, knob: &str, inst: bool) -> bool {
        self.cals.keys().any(|(k, i)| *k == knob && *i == inst)
    }

    /// Give the knob's estimator the calibration for the current INST setting (or none).
    fn sync_calibration(&mut self, knob: &'static str) {
        let inst = self.inst_on(knob).unwrap_or(false);
        let want = self.cals.get(&(knob, inst)).cloned().unwrap_or_default();
        let offset = self.cal_offsets.get(&(knob, inst)).copied().unwrap_or(0.0);
        if let Some(e) = self.gain_est.get_mut(knob) {
            if *e.calibration() != want {
                e.set_calibration(want);
            }
            if e.cal_offset() != offset {
                e.set_cal_offset(offset);
            }
        }
    }

    /// The user's saved calibrations, if every position of one was recorded.
    fn load_calibrations(&mut self) {
        for (knob, _) in self.model.meters.knob_lights {
            self.migrate_calibration(knob);
            for inst in [false, true] {
                let measured: Option<Vec<(f64, f64)>> = knobs::CAL_STEPS
                    .iter()
                    .enumerate()
                    .map(|(i, s)| self.settings.get_f64(&self.cal_key(knob, inst, i)).map(|f| (s.0, f)))
                    .collect();
                if let Some(cal) = measured.and_then(|m| Calibration::from_measured(&m).ok()) {
                    self.cals.insert((knob, inst), cal);
                    if let Some(db) = self.settings.get_f64(&self.cal_offset_key(knob, inst)) {
                        self.cal_offsets.insert((knob, inst), db);
                    }
                }
            }
            self.sync_calibration(knob);
        }
    }

    /// Move a calibration saved before they were kept per INST setting into its slot.
    fn migrate_calibration(&mut self, knob: &'static str) {
        let n = knobs::CAL_STEPS.len();
        let old: Vec<Option<f64>> = (0..n).map(|i| self.settings.get_f64(&self.legacy_cal_key(knob, Some(i)))).collect();
        if old.iter().all(Option::is_none) {
            return;
        }
        let inst = self.settings.get_f64(&self.legacy_cal_key(knob, None)).is_some_and(|v| v != 0.0);
        for (i, v) in old.iter().enumerate() {
            if let Some(v) = v {
                let _ = self.settings.set_f64(&self.cal_key(knob, inst, i), Some(*v));
            }
            let _ = self.settings.set_f64(&self.legacy_cal_key(knob, Some(i)), None);
        }
        let _ = self.settings.set_f64(&self.legacy_cal_key(knob, None), None);
    }

    fn load_knobs(&mut self) {
        self.load_calibrations();
        let ids: Vec<&'static str> = self.model.meters.knob_lights.iter().map(|(k, _)| *k).collect();
        for id in ids {
            if let Some(v) = self.settings.get_f64(&self.knob_key(id)) {
                self.knob_manual.insert(id, v.clamp(0.0, 1.0));
            }
            let (anchor, adc) = (self.settings.get_f64(&self.anchor_key(id)), self.settings.get_f64(&self.adc_key(id)));
            if let Some(e) = self.gain_est.get_mut(id) {
                if let Some(db) = adc {
                    e.restore_converter_floor(db);
                }
                if let Some(db) = anchor {
                    e.restore_anchor(db);
                }
            }
        }
    }

    /// Is this part a knob the user can turn on the picture? Only the GAIN
    /// knobs: the others can't be followed at all, so they stay fixed.
    pub fn is_knob(&self, key: &str) -> bool {
        self.gain_est.contains_key(key)
            && self.renders[0].pointers.contains_key(key)
            && self.model.part(key).is_some_and(|p| p.kind == Kind::Analog)
    }

    /// Where a knob is shown, if anywhere other than the drawing's own pointer:
    /// the user's setting, else the GAIN estimate.
    pub fn knob_shown(&self, id: &str) -> Option<KnobShown> {
        // A GAIN knob follows the input's idle level only once that level is
        // known to mean something: after the user set the knob (the instrument's
        // own noise is learned from it) or calibrated it. Before that, nothing
        // can tell an instrument's hum at minimum gain from the preamp's own
        // noise at a high setting, so no position is claimed.
        if let Some(e) = self.gain_est.get(id).filter(|e| e.is_anchored() || e.is_learned())
            && let Some(pos) = e.estimate()
        {
            return Some(KnobShown { pos, source: if e.is_live() { KnobSource::Estimated } else { KnobSource::Held } });
        }
        if !self.gain_est.contains_key(id) {
            return self.renders[0].pointers.contains_key(id).then_some(KnobShown { pos: knobs::FIXED_POS, source: KnobSource::Fixed });
        }
        self.knob_manual.get(id).map(|p| KnobShown { pos: *p, source: KnobSource::Manual })
    }

    /// Where to start from when the user grabs a knob.
    pub fn knob_start(&self, id: &str) -> f64 {
        self.knob_shown(id).map(|k| k.pos).or_else(|| self.renders[0].pointers.get(id).and_then(|p| knobs::pos_from_angle(p.angle_deg()))).unwrap_or(0.5)
    }

    /// Pointer angle to draw for a knob (degrees), None = as drawn in the SVG.
    pub fn knob_angle(&self, id: &str) -> Option<f64> {
        self.knob_shown(id).map(|k| knobs::angle_deg(k.pos))
    }

    /// The card's second line for a knob: where it is shown and why. Short,
    /// so it fits the narrow analog cards.
    pub fn knob_note(&self, id: &str) -> Option<String> {
        let Some(k) = self.knob_shown(id) else {
            // calibrated only for the other INST setting: say so rather than nothing
            let on = self.inst_on(id)?;
            let other = !self.calibrated_for(id, on) && self.calibrated_for(id, !on);
            return other.then(|| format!("Calibrated with INST {}", if on { "off" } else { "on" }));
        };
        // buried in converter noise (nothing plugged in, gain low): only an upper bound is known
        let below = self.gain_est.get(id).and_then(|e| e.masked_below()).filter(|_| k.source != KnobSource::Manual);
        let value = match below {
            Some(top) => format!("under {}%", knobs::percent(top).div_ceil(5) * 5),
            None => format!("{}%", knobs::percent(k.pos)),
        };
        Some(match k.source {
            KnobSource::Manual => format!("By you · {value}"),
            KnobSource::Estimated => format!("Estimated · {value}"),
            KnobSource::Held => format!("Last known · {value}"),
            KnobSource::Fixed => return None,
        })
    }

    /// Hand the input meters (dBFS, in meter order) to the GAIN estimators; `t` is seconds on any monotonic clock.
    pub fn feed_gain_estimators(&mut self, t: f64, meters: &[f64]) {
        if let Some(w) = self.cal_wizard.as_mut()
            && let Some(db) = meters.get(w.meter)
        {
            w.feed(t, *db);
        }
        for (knob, idx) in self.model.meters.knob_lights {
            self.sync_calibration(knob);
            let Some(est) = self.gain_est.get_mut(knob) else { continue };
            let Some(db) = meters.get(*idx) else { continue };
            let adc = est.converter_floor();
            est.update(t, *db);
            // a lower converter floor is rare (it only ever goes down): keep it for next time
            let now = est.converter_floor();
            if now < adc - 0.2 {
                let key = self.adc_key(knob);
                let _ = self.settings.set_f64(&key, Some(now));
            }
        }
    }

    /// Seconds on the clock the wizard and the estimators share.
    pub fn started_at_secs(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }

    /// Start recording the wizard's current position at time `t` (tests drive their own clock).
    pub fn cal_record_at(&mut self, t: f64) {
        if let Some(w) = self.cal_wizard.as_mut() {
            w.record(t);
        }
    }

    pub fn start_calibration(&mut self, knob: &'static str) {
        if let Some((_, idx)) = self.model.meters.knob_lights.iter().find(|k| k.0 == knob) {
            let mut w = CalWizard::new(knob, *idx);
            w.inst = self.inst_on(knob);
            self.cal_wizard = Some(w);
        }
    }

    /// Is the knob calibrated for either INST setting?
    pub fn is_calibrated(&self, knob: &str) -> bool {
        self.calibrated_for(knob, false) || self.calibrated_for(knob, true)
    }

    fn knob_label(&self, knob: &str) -> &'static str {
        self.model.part(knob).map_or("GAIN", |p| p.label)
    }

    fn save_calibration(&mut self) {
        let Some(w) = self.cal_wizard.take() else { return };
        match w.calibration() {
            Ok(cal) => {
                // saved for the INST setting it was recorded with; the other one is kept
                let inst = w.inst.unwrap_or(false);
                for (i, (_, floor)) in w.measured().unwrap_or_default().iter().enumerate() {
                    let _ = self.settings.set_f64(&self.cal_key(w.knob, inst, i), Some(*floor));
                }
                self.cals.insert((w.knob, inst), cal);
                self.store_cal_offset(w.knob, inst, 0.0);
                self.sync_calibration(w.knob);
                // the old manual setting no longer applies: the calibrated estimate takes over
                self.knob_manual.remove(w.knob);
                let _ = self.settings.set_f64(&self.knob_key(w.knob), None);
                let _ = self.settings.set_f64(&self.anchor_key(w.knob), None);
                let label = self.knob_label(w.knob);
                self.set_status(format!("{label} calibrated: its position now follows your setup."), HW_STATUS);
            }
            Err(e) => {
                let mut w = w;
                w.phase = CalPhase::Failed(e);
                self.cal_wizard = Some(w);
            }
        }
    }

    fn clear_calibration(&mut self, knob: &'static str) {
        for inst in [false, true] {
            for i in 0..knobs::CAL_STEPS.len() {
                let _ = self.settings.set_f64(&self.cal_key(knob, inst, i), None);
            }
            self.cals.remove(&(knob, inst));
            self.store_cal_offset(knob, inst, 0.0);
        }
        self.sync_calibration(knob);
        let label = self.knob_label(knob);
        self.set_status(format!("{label}: calibration removed."), HW_STATUS);
    }

    fn set_knob(&mut self, id: &'static str, pos: Option<f64>) {
        if !self.gain_est.contains_key(id) {
            return; // MONITOR, MIX, PHONES are fixed
        }
        match pos {
            Some(p) => {
                let p = p.clamp(0.0, 1.0);
                self.knob_manual.insert(id, p);
                let _ = self.settings.set_f64(&self.knob_key(id), Some(p));
                // learn the input's own noise from the user's setting (or correct the
                // calibration with it), so the estimate follows the floor from here
                let inst = self.inst_on(id).unwrap_or(false);
                let Some(e) = self.gain_est.get_mut(id) else { return };
                if !e.set_by_user(p) {
                    e.clear_anchor();
                }
                let (learned, anchor, offset) = (e.is_learned(), e.anchor(), e.cal_offset());
                if learned {
                    self.store_cal_offset(id, inst, offset);
                } else {
                    let _ = self.settings.set_f64(&self.anchor_key(id), anchor);
                }
            }
            None => {
                self.knob_manual.remove(id);
                let _ = self.settings.set_f64(&self.knob_key(id), None);
                let _ = self.settings.set_f64(&self.anchor_key(id), None);
                let inst = self.inst_on(id).unwrap_or(false);
                if self.cals.contains_key(&(id, inst)) {
                    // back to the calibration as recorded
                    self.store_cal_offset(id, inst, 0.0);
                    self.sync_calibration(id);
                } else if let Some(e) = self.gain_est.get_mut(id) {
                    e.clear_anchor();
                }
            }
        }
    }

    /// Start the backend and load the device state.
    pub fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.audio.refresh();
        let evs = self.backend.start();
        self.available = self.backend.keys().into_iter().collect();
        for k in self.backend.keys() {
            if let Some(v) = self.backend.get(&k) {
                self.values.insert(k, v);
            }
        }
        self.levels = self.backend.meters();
        self.advance_meters(0.0);
        self.apply(evs, None);
        self.backend.set_live(self.visible);
    }

    /// Run the meter ballistics `dt` seconds forward with the latest readings.
    pub fn advance_meters(&mut self, dt: f64) {
        self.last_meter_update = Instant::now();
        self.meter_view.resize(self.levels.len().max(self.meter_view.len()), Default::default());
        for (i, b) in self.meter_view.iter_mut().enumerate() {
            b.update(self.levels.get(i).copied().unwrap_or(f64::NEG_INFINITY), dt);
        }
    }

    /// Is meter word `idx` clipping (latched) at time `now`?
    pub fn is_clipping_at(&self, idx: usize, now: Instant) -> bool {
        self.clip_at.get(idx).copied().flatten().is_some_and(|t| now.saturating_duration_since(t) < crate::levels::CLIP_LATCH)
    }

    pub fn is_clipping(&self, idx: usize) -> bool {
        self.is_clipping_at(idx, Instant::now())
    }

    /// The light of a GAIN knob (`MeterSpec::knob_lights`): off without
    /// signal, blue rising with the smoothed input level, red while clipping.
    pub fn knob_light(&self, knob: &str) -> Option<KnobLight> {
        let (_, idx) = self.model.meters.knob_lights.iter().find(|k| k.0 == knob)?;
        Some(if self.is_clipping(*idx) {
            KnobLight::Clip
        } else {
            let lvl = self.meter_level(*idx);
            let floor = self.model.meters.floor_db;
            if lvl <= floor {
                KnobLight::Off
            } else {
                KnobLight::Signal(((lvl - floor) / -floor).clamp(0.0, 1.0) as f32)
            }
        })
    }

    /// Labels of the meters currently clipping ("Input 1" ...).
    pub fn clipping_labels(&self) -> Vec<&'static str> {
        let now = Instant::now();
        self.model.meters.labels.iter().enumerate().filter(|(i, _)| self.is_clipping_at(*i, now)).map(|(_, l)| *l).collect()
    }

    /// Displayed level of a meter word (dBFS, smoothed).
    pub fn meter_level(&self, idx: usize) -> f64 {
        self.meter_view.get(idx).map(|b| b.level).unwrap_or(f64::NEG_INFINITY)
    }

    pub fn meter_peak(&self, idx: usize) -> f64 {
        self.meter_view.get(idx).map(|b| b.peak).unwrap_or(f64::NEG_INFINITY)
    }

    /// What the audio is running at: shown on the main page.
    pub fn audio_summary(&self) -> Option<crate::audioclock::Summary> {
        crate::audioclock::summary(self.audio.clock.as_ref(), self.backend.playback_stream().as_ref())
    }

    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    pub fn backend_mut(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }

    /// The mock backend, for tests and screenshots.
    pub fn mock(&mut self) -> &mut crate::backend::mock::MockBackend {
        self.backend.as_any_mut().downcast_mut().expect("mock backend")
    }

    pub fn tokens(&self) -> &'static Tokens {
        let dark = match self.theme_mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::Auto => self.system_dark,
        };
        if dark { &theme::DARK } else { &theme::LIGHT }
    }

    pub fn render(&self, view: View) -> &DeviceRender {
        &self.renders[match view {
            View::Front => 0,
            View::Back => 1,
        }]
    }

    pub fn value_bool(&self, key: &str) -> bool {
        self.values.get(key).is_some_and(|v| v.as_bool())
    }

    pub fn value_db(&self, key: &str) -> Option<f64> {
        self.values.get(key).map(|v| v.as_db())
    }

    pub fn is_flashing(&self, part_key: &str) -> bool {
        self.flashes.get(part_key).is_some_and(|t| Instant::now() < *t)
    }

    /// Drop transient highlights (screenshots).
    pub fn clear_transients(&mut self) {
        self.flashes.clear();
        self.status_msg = None;
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status_msg.as_ref().filter(|(_, t)| Instant::now() < *t).map(|(m, _)| m.as_str())
    }

    pub fn card_hot(&self, part_key: &str) -> bool {
        self.hover_card == Some(part_key) || self.hover_part == Some(part_key)
    }

    fn set_status(&mut self, msg: impl Into<String>, d: Duration) {
        self.status_msg = Some((msg.into(), Instant::now() + d));
    }

    // ------------------------------------------------------------ layout

    pub fn stage_size(&self) -> Size {
        let banner = if self.shows_sim_banner() { layout::BANNER_H } else { 0.0 };
        Size::new(
            self.window_size.width,
            self.window_size.height - (layout::HEADER_H + banner + layout::STATUS_H) as f32,
        )
    }

    /// Top of the stage in window coordinates.
    pub fn stage_origin_y(&self) -> f64 {
        layout::HEADER_H + if self.shows_sim_banner() { layout::BANNER_H } else { 0.0 }
    }

    pub fn layout_for(&self, size: Size) -> StageLayout {
        layout::compute(self.model, self.render(self.view), self.view, size.width as f64, size.height as f64, self.panel_open)
    }

    /// Current stage layout for the known window size (tests, hit testing).
    pub fn stage_layout(&self) -> StageLayout {
        self.layout_for(self.stage_size())
    }

    /// Window-space rectangle of a part.
    pub fn part_rect(&self, key: &str) -> Option<Rect> {
        let r = self.stage_layout().part_rect(key)?;
        Some(Rect::new(r.x, r.y + self.stage_origin_y(), r.w, r.h))
    }

    /// Window-space rectangle of a callout card.
    pub fn card_rect(&self, key: &str) -> Option<Rect> {
        let r = self.stage_layout().card(key)?.rect;
        Some(Rect::new(r.x, r.y + self.stage_origin_y(), r.w, r.h))
    }

    /// Ids of render elements currently lit (buttons on, meter LEDs).
    pub fn lit_elements(&self) -> Vec<String> {
        let render = self.render(self.view);
        let mut lit = vec![];
        for part in self.model.parts_for(self.view) {
            match part.kind {
                Kind::Toggle | Kind::Phantom => {
                    if part.control.is_some_and(|c| self.value_bool(c)) {
                        lit.extend(part.ids.iter().filter(|i| render.buttons.contains(**i)).map(|i| i.to_string()));
                    }
                }
                _ => {}
            }
        }
        // the LED ladder: the render's own meter
        let ms = &self.model.meters;
        for (col, idx) in ms.led_columns {
            let lvl = self.meter_level(*idx);
            let clip = self.is_clipping(*idx);
            for (row, thr) in ms.led_thresholds.iter().enumerate() {
                let id = format!("meter-{col}-{row}");
                // the top (red) LED also follows the clip latch
                if (lvl >= *thr || (row == 0 && clip)) && render.leds.contains(&id) {
                    lit.push(id);
                }
            }
        }
        lit
    }

    /// Image handles for the render at `device` size (cached per size).
    pub(crate) fn base_image(&self, device: &Rect) -> iced::widget::image::Handle {
        let (w, h) = ((device.w * self.scale as f64).round() as u32, (device.h * self.scale as f64).round() as u32);
        let key = (self.view, w, h);
        let mut r = self.rasters.borrow_mut();
        if let Some((k, hd)) = &r.base
            && *k == key
        {
            return hd.clone();
        }
        let ras = self.render(self.view).rasterize_base(w, h);
        let hd = iced::widget::image::Handle::from_rgba(ras.width, ras.height, ras.rgba);
        r.base = Some((key, hd.clone()));
        r.sprites.retain(|(v, _, sw), _| *v == self.view && *sw == w);
        hd
    }

    pub(crate) fn sprite(&self, id: &str, device: &Rect) -> Option<(iced::widget::image::Handle, Rect)> {
        let w = (device.w * self.scale as f64).round() as u32;
        let key = (self.view, id.to_string(), w);
        let mut r = self.rasters.borrow_mut();
        if let Some(s) = r.sprites.get(&key) {
            return Some(s.clone());
        }
        let (ras, rect) = self.render(self.view).rasterize_lit(id, w)?;
        let hd = iced::widget::image::Handle::from_rgba(ras.width, ras.height, ras.rgba);
        r.sprites.insert(key, (hd.clone(), rect));
        Some((hd, rect))
    }

    // ------------------------------------------------------------ update

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        let task = self.handle(msg);
        self.sync_tray();
        task
    }

    fn handle(&mut self, msg: Message) -> Task<Message> {
        if std::env::var_os("OPENFUSE_DEBUG").is_some() && !matches!(msg, Message::Tick | Message::HoverPart(_) | Message::HoverCard(_)) {
            eprintln!("openfuse: {msg:?}");
        }
        match msg {
            Message::Tick => {
                self.ticks = self.ticks.wrapping_add(1);
                if self.ticks == 50
                    && let (Some(id), Some(_)) = (self.window, std::env::var_os("OPENFUSE_SNAPSHOT"))
                {
                    return window::screenshot(id).map(Message::Snapshot);
                }
                let evs = self.backend.poll();
                self.apply(evs, None);
                if self.visible {
                    self.audio.poll();
                } else if self.has_tray() {
                    // the tray menu shows the audio settings too
                    self.audio.poll_every(crate::audioclock::POLL_HIDDEN);
                }
                let now = Instant::now();
                self.flashes.retain(|_, t| now < *t);
                if self.status_msg.as_ref().is_some_and(|(_, t)| now >= *t) {
                    self.status_msg = None;
                }
                self.check_for_device();
            }
            Message::Toggle(key, value) => return self.request_toggle(&key, value),
            Message::SetGain(key, db) => self.send(&key, Value::Db(db)),
            Message::PartClicked(key) => return self.activate_part(key),
            Message::HoverPart(p) => self.hover_part = p,
            Message::HoverCard(c) => self.hover_card = c,
            Message::SetView(v) => self.set_view(v),
            Message::SetPanel(on) => self.set_panel_open(on),
            Message::SetAudioPanel(on) => self.set_audio_panel_open(on),
            Message::SetAbout(on) => self.about_open = on,
            Message::SetKnob(id, pos) => self.set_knob(id, Some(pos)),
            Message::ResetKnob(id) => self.set_knob(id, None),
            Message::CalStart(knob) => self.start_calibration(knob),
            Message::CalRecord => self.cal_record_at(self.started_at.elapsed().as_secs_f64()),
            Message::CalNext => {
                if let Some(w) = self.cal_wizard.as_mut() {
                    w.next();
                }
            }
            Message::CalBack => {
                if let Some(w) = self.cal_wizard.as_mut() {
                    w.back();
                }
            }
            Message::CalSave => self.save_calibration(),
            Message::CalCancel => self.cal_wizard = None,
            Message::CalClear(knob) => self.clear_calibration(knob),
            Message::SetClockRate(r) => self.audio.set_rate(r),
            Message::SetClockQuantum(q) => self.audio.set_quantum(q),
            Message::Confirm(ok) => {
                if self.confirm.take().is_some() && ok {
                    // only "Turn on" saves "Don't ask again"; Cancel never does
                    if self.confirm_dont_ask {
                        let _ = self.settings.set_bool(KEY_ASK_PHANTOM, false);
                    }
                    self.send("phantom", Value::Bool(true));
                }
                self.confirm_dont_ask = false;
            }
            Message::ConfirmDontAsk(on) => {
                if self.confirm.is_some() {
                    self.confirm_dont_ask = on;
                }
            }
            Message::SetAskPhantom(on) => {
                let _ = self.settings.set_bool(KEY_ASK_PHANTOM, on);
            }
            Message::ConfirmFocus(b) => {
                if self.confirm.is_some() {
                    self.confirm = Some(b);
                }
            }
            Message::Key(ev) => return self.on_key(ev),
            Message::WindowOpened(id) => {
                if self.window == Some(id) {
                    return Task::batch([
                        window::scale_factor(id).map(Message::ScaleFactor),
                        iced::system::information().map(|i| Message::Renderer(i.graphics_backend)),
                    ]);
                }
            }
            Message::CloseRequested(id) => {
                // (the snapshot debug aid ignores close requests until it has its picture)
                if self.window == Some(id) && std::env::var_os("OPENFUSE_SNAPSHOT").is_none() {
                    return self.close_window();
                }
            }
            Message::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                    self.set_visible(false);
                    if !self.has_tray() {
                        return self.quit();
                    }
                }
            }
            Message::Resized(id, size) => {
                self.window_size = size;
                if self.window == Some(id) {
                    return Task::batch([
                        window::scale_factor(id).map(Message::ScaleFactor),
                        window::is_maximized(id).map(Message::Maximized),
                    ]);
                }
            }
            Message::ScaleFactor(s) => {
                if s.is_finite() && s > 0.0 {
                    self.scale = s;
                }
            }
            Message::SystemTheme(mode) => self.system_dark = mode == iced::theme::Mode::Dark,
            Message::External(e) => return self.on_external(e),
            Message::Drag => {
                if let Some(id) = self.window {
                    return window::drag(id);
                }
            }
            Message::ToggleMaximize => {
                if let Some(id) = self.window {
                    return window::toggle_maximize::<Message>(id)
                        .chain(window::is_maximized(id).map(Message::Maximized));
                }
            }
            Message::Minimize => {
                if let Some(id) = self.window {
                    return window::minimize(id, true);
                }
            }
            Message::Maximized(m) => self.maximized = m,
            Message::DragResize(dir) => {
                if let Some(id) = self.window
                    && !self.maximized
                {
                    return window::drag_resize(id, dir);
                }
            }
            Message::ClearClip(idx) => {
                for i in idx {
                    if let Some(c) = self.clip_at.get_mut(i) {
                        *c = None;
                    }
                }
            }
            Message::Snapshot(shot) => {
                if let Some(path) = std::env::var_os("OPENFUSE_SNAPSHOT") {
                    let (w, h) = (shot.size.width, shot.size.height);
                    let saved = resvg::tiny_skia::Pixmap::from_vec(
                        shot.rgba.to_vec(),
                        resvg::tiny_skia::IntSize::from_wh(w, h).expect("size"),
                    )
                    .map(|p| p.save_png(&path));
                    eprintln!("openfuse: snapshot {w}x{h} -> {} ({saved:?})", std::path::Path::new(&path).display());
                }
                return self.quit();
            }
            Message::Renderer(name) => {
                if std::env::var_os("OPENFUSE_DEBUG").is_some() {
                    eprintln!("openfuse: renderer {name}");
                }
                // softbuffer (tiny-skia fallback) cannot present alpha: keep square corners
                if name.to_ascii_lowercase().contains("tiny-skia") {
                    self.rounded = false;
                }
            }
        }
        Task::none()
    }

    /// Apply backend events. `own` is the key of the request that caused
    /// them; everything else happened on the device itself.
    fn apply(&mut self, events: Vec<Event>, own: Option<&str>) {
        for ev in events {
            match ev {
                Event::ValueChanged(key, v) => {
                    self.values.insert(key.clone(), v);
                    if own != Some(key.as_str()) && self.started {
                        self.hardware_changed(&key, v);
                    }
                }
                Event::Meters(m) => {
                    let now = Instant::now();
                    if self.clip_at.len() < m.len() {
                        self.clip_at.resize(m.len(), None);
                    }
                    for (i, db) in m.iter().enumerate() {
                        if crate::levels::is_clip(*db) {
                            self.clip_at[i] = Some(now);
                        }
                    }
                    self.feed_gain_estimators(self.started_at.elapsed().as_secs_f64(), &m);
                    self.levels = m;
                    let dt = self.last_meter_update.elapsed().as_secs_f64();
                    self.advance_meters(dt);
                }
                Event::Error(e) => self.set_status(e, ERR_STATUS),
                Event::StatusChanged(s) => self.status = s,
            }
        }
    }

    fn hardware_changed(&mut self, key: &str, v: Value) {
        if let Some(part) = self.model.part_for_control(key) {
            self.flashes.insert(part.key, Instant::now() + FLASH);
            let state = if v.as_bool() { "ON" } else { "OFF" };
            self.set_status(format!("Changed on the device: {} → {state}", part.label), HW_STATUS);
        }
        if key == "phantom" && self.settings.get_bool(KEY_NOTIFY_PHANTOM, false) {
            let title = format!("48 V phantom power {}", if v.as_bool() { "ON" } else { "OFF" });
            self.notify(&title, &format!("Switched with the button on the {}.", self.model.name));
        }
    }

    fn send(&mut self, key: &str, v: Value) {
        let evs = self.backend.set(key, v);
        self.apply(evs, Some(key));
    }

    /// Switch request from a card, the render, the panel or the tray.
    pub fn request_toggle(&mut self, key: &str, value: bool) -> Task<Message> {
        if !self.available.contains(key) {
            return Task::none();
        }
        if key == "phantom" && value && self.ask_phantom() {
            // 48 V needs confirmation; Cancel is the default
            let task = if self.window.is_none() && !self.visible { self.open_window() } else { Task::none() };
            if self.confirm.is_none() {
                self.confirm = Some(ConfirmButton::Cancel);
                self.confirm_dont_ask = false;
            }
            return task;
        }
        self.send(key, Value::Bool(value));
        Task::none()
    }

    /// Ask before switching 48 V on? (default yes; a missing or unreadable
    /// settings file means yes)
    pub fn ask_phantom(&self) -> bool {
        self.settings.get_bool(KEY_ASK_PHANTOM, true)
    }

    /// What clicking a part on the render does (same as its callout).
    pub fn activate_part(&mut self, key: &str) -> Task<Message> {
        let Some(part) = self.model.part(key) else { return Task::none() };
        match part.kind {
            Kind::Toggle | Kind::Phantom => {
                let c = part.control.expect("toggle has a control");
                return self.request_toggle(c, !self.value_bool(c));
            }
            Kind::Loopback => self.set_panel_open(!self.panel_open),
            Kind::Analog if self.is_knob(part.key) => self.set_status(
                format!(
                    "{}: emulated, the MiniFuse doesn't report it. Drag or scroll the knob to match the device \
                     (double-click resets), or calibrate it in Audio settings (A).",
                    part.label
                ),
                ERR_STATUS,
            ),
            Kind::Analog => self.set_status(format!("{}: {ANALOG_TIP}", part.label), ERR_STATUS),
            Kind::Meter | Kind::Connector => {}
        }
        Task::none()
    }

    pub fn set_view(&mut self, v: View) {
        if v != self.view {
            self.view = v;
            self.hover_part = None;
            self.hover_card = None;
            if v == View::Front {
                self.panel_open = false;
            }
        }
    }

    pub fn set_panel_open(&mut self, on: bool) {
        let on = on && self.model.loopback.is_some();
        if on && self.view != View::Back {
            self.set_view(View::Back);
        }
        self.panel_open = on;
    }

    pub fn set_audio_panel_open(&mut self, on: bool) {
        if on && !self.audio_panel_open {
            self.audio.refresh();
        }
        self.audio_panel_open = on;
    }

    fn on_key(&mut self, ev: keyboard::Event) -> Task<Message> {
        use keyboard::key::Named;
        let keyboard::Event::KeyPressed { key, modifiers, .. } = ev else { return Task::none() };
        if let Some(focus) = self.confirm {
            match key.as_ref() {
                keyboard::Key::Named(Named::Escape) => return self.handle(Message::Confirm(false)),
                keyboard::Key::Named(Named::Enter | Named::Space) => {
                    return self.handle(Message::Confirm(focus == ConfirmButton::Enable));
                }
                keyboard::Key::Named(Named::Tab | Named::ArrowLeft | Named::ArrowRight) => {
                    self.confirm = Some(match focus {
                        ConfirmButton::Cancel => ConfirmButton::Enable,
                        ConfirmButton::Enable => ConfirmButton::Cancel,
                    });
                }
                _ => {}
            }
            return Task::none();
        }
        if self.cal_wizard.is_some() {
            // modal: Escape cancels the calibration, nothing else acts behind it
            if matches!(key.as_ref(), keyboard::Key::Named(Named::Escape)) {
                self.cal_wizard = None;
            }
            return Task::none();
        }
        if self.about_open {
            // modal: Escape / Enter close it, nothing else acts behind it
            if matches!(key.as_ref(), keyboard::Key::Named(Named::Escape | Named::Enter)) {
                self.about_open = false;
            }
            return Task::none();
        }
        match key.as_ref() {
            keyboard::Key::Character("q") if modifiers.command() => return self.quit(),
            keyboard::Key::Character("w") if modifiers.command() => {
                return self.close_window();
            }
            _ if modifiers.command() || modifiers.alt() => {}
            keyboard::Key::Character("f") => self.set_view(View::Front),
            keyboard::Key::Character("b") => self.set_view(View::Back),
            keyboard::Key::Character("l") => self.set_panel_open(!self.panel_open),
            keyboard::Key::Character("a") => self.set_audio_panel_open(!self.audio_panel_open),
            keyboard::Key::Named(Named::Escape) if self.audio_panel_open => self.set_audio_panel_open(false),
            keyboard::Key::Named(Named::Escape) if self.panel_open => self.set_panel_open(false),
            _ => {}
        }
        Task::none()
    }

    fn on_external(&mut self, e: External) -> Task<Message> {
        match e {
            External::Activate | External::Open => self.open_window(),
            External::OpenAudio => {
                self.set_audio_panel_open(true);
                self.open_window()
            }
            External::ToggleWindow => {
                if self.window.is_some() {
                    self.close_window()
                } else {
                    self.open_window()
                }
            }
            External::Toggle(key) => {
                let cur = self.value_bool(&key);
                self.request_toggle(&key, !cur)
            }
            External::SetAutostart(on) => {
                if let Err(e) = autostart::set_autostart(on) {
                    self.notify("OpenFuse", &format!("Could not change start-at-login: {e}"));
                }
                Task::none()
            }
            External::SetAskPhantom(on) => {
                let _ = self.settings.set_bool(KEY_ASK_PHANTOM, on);
                Task::none()
            }
            External::SetNotify(on) => {
                let _ = self.settings.set_bool(KEY_NOTIFY_PHANTOM, on);
                Task::none()
            }
            External::Quit => self.quit(),
            External::TrayOffline => {
                self.tray = None;
                self.fake_tray = false;
                self.last_tray = None;
                if self.window.is_none() { self.open_window() } else { Task::none() }
            }
        }
    }

    // ------------------------------------------------------------ windows

    pub fn window_settings() -> window::Settings {
        let icon = crate::icons::render_icon(false, false, 64);
        window::Settings {
            size: Size::new(1200.0, 760.0),
            min_size: Some(Size::new(1060.0, 600.0)),
            icon: window::icon::from_rgba(icon.rgba, icon.width, icon.height).ok(),
            exit_on_close_request: false,
            // our own title controls; transparent so the rounded corners show the desktop
            decorations: false,
            transparent: rounded_window(),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "openfuse".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn open_window(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            return window::gain_focus(id);
        }
        let (id, task) = window::open(Self::window_settings());
        self.window = Some(id);
        self.maximized = false;
        self.rounded = rounded_window();
        self.set_visible(true);
        task.map(Message::WindowOpened)
    }

    /// Window close button / Ctrl+W / tray click: hide to the tray when there
    /// is one, otherwise quit.
    pub fn has_tray(&self) -> bool {
        self.tray.is_some() || self.fake_tray
    }

    pub fn close_window(&mut self) -> Task<Message> {
        if !self.has_tray() {
            return self.quit();
        }
        self.confirm = None;
        let task = match self.window.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        };
        self.set_visible(false);
        if !self.settings.get_bool(KEY_CLOSE_HINT, false) {
            self.notify(
                "OpenFuse is still running",
                "It keeps watching your MiniFuse from the tray. Use Quit in the tray menu to exit.",
            );
            let _ = self.settings.set_bool(KEY_CLOSE_HINT, true);
        }
        task
    }

    /// Window shown (true) or hidden (false): meters are only polled while it can be seen.
    pub fn set_visible(&mut self, on: bool) {
        self.visible = on;
        self.backend.set_live(on);
    }

    pub fn quit(&mut self) -> Task<Message> {
        self.quitting = true;
        self.confirm = None;
        self.backend.stop();
        if let Some(t) = self.tray.take() {
            t.shutdown();
        }
        iced::exit()
    }

    fn notify(&mut self, title: &str, body: &str) {
        self.notifications.push((title.to_string(), body.to_string()));
        if self.desktop_notifications {
            crate::tray::desktop_notification(title, body);
        }
    }

    // ------------------------------------------------------------ tray

    pub fn tray_snapshot(&self) -> TraySnapshot {
        TraySnapshot::from_app(self)
    }

    fn sync_tray(&mut self) {
        if self.tray.is_none() {
            return;
        }
        let snap = self.tray_snapshot();
        if self.last_tray.as_ref() != Some(&snap) {
            if let Some(t) = &self.tray {
                t.update(snap.clone());
            }
            self.last_tray = Some(snap);
        }
    }

    // ------------------------------------------------------------ subscriptions

    pub fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(if self.visible { TICK_VISIBLE } else { TICK_HIDDEN }).map(|_| Message::Tick);
        let mut subs = vec![
            tick,
            keyboard::listen().map(Message::Key),
            window::close_requests().map(Message::CloseRequested),
            window::close_events().map(Message::WindowClosed),
            window::resize_events().map(|(id, s)| Message::Resized(id, s)),
            Subscription::run(external_stream).map(Message::External),
        ];
        if self.theme_mode == ThemeMode::Auto {
            subs.push(iced::system::theme_changes().map(Message::SystemTheme));
        }
        Subscription::batch(subs)
    }

    pub fn theme(&self, _id: window::Id) -> iced::Theme {
        theme::iced_theme(*self.tokens())
    }

    pub fn title(&self, _id: window::Id) -> String {
        if self.status == Status::NoDevice { "OpenFuse".to_string() } else { format!("OpenFuse — {}", self.model.name) }
    }

    // ------------------------------------------------------------ view

    /// Corner radius of the window frame (0 when maximized or without a compositor).
    pub fn corner_radius(&self) -> f32 {
        if self.rounded && !self.maximized { CORNER } else { 0.0 }
    }

    pub fn style(&self, th: &iced::Theme) -> iced::theme::Style {
        let t = theme::tokens(th);
        iced::theme::Style {
            background_color: if self.rounded { iced::Color::TRANSPARENT } else { t.bg },
            text_color: t.text,
        }
    }

    pub fn view(&self, _id: window::Id) -> Element<'_, Message> {
        let mut col = column![self.header()];
        if self.shows_sim_banner() {
            col = col.push(self.notice());
        }
        if self.no_device() {
            col = col.push(self.no_device_page());
        } else {
            col = col.push(responsive(move |size| stage::stage(self, size)));
        }
        col = col.push(self.status_bar());
        let r = self.corner_radius();
        let frame: Element<'_, Message> = container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |th| {
                let t = theme::tokens(th);
                container::Style {
                    background: Some(theme::backdrop(&t)),
                    text_color: Some(t.text),
                    border: Border {
                        color: if r > 0.0 {
                            if t.dark { theme::alpha(iced::Color::WHITE, 0.08) } else { theme::alpha(iced::Color::BLACK, 0.12) }
                        } else {
                            iced::Color::TRANSPARENT
                        },
                        width: if r > 0.0 { 1.0 } else { 0.0 },
                        radius: r.into(),
                    },
                    ..Default::default()
                }
            })
            .into();
        let mut layers = vec![frame];
        if self.audio_panel_open {
            layers.push(
                container(audio::panel(self))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_right(Length::Fill)
                    .padding(iced::Padding { top: layout::HEADER_H as f32, right: 16.0, bottom: layout::STATUS_H as f32, left: 16.0 })
                    .into(),
            );
        }
        if self.about_open {
            layers.push(about::dialog(self));
        }
        if self.cal_wizard.is_some() {
            layers.push(calwizard::dialog(self));
        }
        if self.confirm.is_some() {
            layers.push(phantom::dialog(self));
        }
        if !self.maximized {
            layers.push(resize_handles());
        }
        stack(layers).into()
    }

    fn header(&self) -> Element<'_, Message> {
        let t = *self.tokens();
        let subtitle = if self.status == Status::NoDevice {
            "Control panel for Arturia MiniFuse".to_string()
        } else if self.backend.simulated() {
            "Simulated device".to_string()
        } else if self.status == Status::Disconnected {
            format!("{} — disconnected", self.backend.description())
        } else {
            self.backend.description()
        };
        let (dot, state) = match self.status {
            Status::Connected => (t.good, "Connected"),
            Status::Disconnected => (t.bad, "Disconnected"),
            Status::NoDevice => (t.muted, "Not connected"),
            _ => (t.warn, "Simulation"),
        };
        // Audio chip, Front / Back and the "i" button share one height
        const BAR_H: f32 = 36.0;
        let seg = |label: &'static str, v: View| {
            button(text(label).size(14).font(theme::MEDIUM).height(Length::Fill).align_y(iced::Alignment::Center))
                .padding([0, 18])
                .height(Length::Fill)
                .on_press(Message::SetView(v))
                .style(theme::segment(self.view == v))
        };
        let segmented = container(row![seg("Front", View::Front), seg("Back", View::Back)].spacing(2))
            .padding(3)
            .height(BAR_H)
            .style(theme::segmented_track);
        let audio_btn = {
            let summary = self.audio_summary();
            let mut r = row![text("Audio").size(14).font(theme::SEMIBOLD).color(t.text)].spacing(10).align_y(iced::Alignment::Center);
            if let Some(s) = summary {
                if s.live {
                    r = r.push(container(space::horizontal().width(7)).width(7).height(7).style(theme::dot(t.good)));
                }
                r = r.push(text(s.text).size(13).font(theme::MEDIUM).color(t.text));
                r = r.push(text(s.source).size(13).color(t.muted));
                if s.forced {
                    r = r.push(container(text("SET").size(10).font(theme::BOLD)).padding([2, 7]).style(theme::pill(t.accent)));
                }
            }
            button(container(r).height(Length::Fill).align_y(iced::Alignment::Center))
                .padding([0, 16])
                .height(BAR_H)
                .on_press(Message::SetAudioPanel(!self.audio_panel_open))
                .style(theme::chip(self.audio_panel_open))
        };
        let about_btn = iced::widget::tooltip(
            button(text("i").size(15).font(theme::BOLD).width(Length::Fill).height(Length::Fill).center())
                .width(BAR_H)
                .height(BAR_H)
                .padding(0)
                .on_press(Message::SetAbout(true))
                .style(theme::chip(self.about_open)),
            meterbars::tip_bubble("About OpenFuse"),
            iced::widget::tooltip::Position::Bottom,
        );
        let status = container(
            row![
                container(space::horizontal().width(7)).width(7).height(7).style(theme::dot(dot)),
                text(state).size(12).font(theme::SEMIBOLD).color(dot),
            ]
            .spacing(7)
            .align_y(iced::Alignment::Center),
        )
        .padding([4, 11])
        .style(theme::pill(dot));
        // bottom-aligned with the title; the lift clears the descender space under the text's baseline
        let status = container(status).padding(iced::Padding { top: 0.0, right: 0.0, bottom: 5.0, left: 0.0 });
        let titles = column![
            row![text(if self.status == Status::NoDevice { "OpenFuse" } else { self.model.name }).size(28).font(theme::BOLD).color(t.text).line_height(1.15), status]
                .spacing(14)
                .align_y(iced::Alignment::End),
            text(subtitle).size(14).color(t.muted),
        ]
        .spacing(2);
        let logo = iced::widget::image(self.logo.clone()).width(46).height(46);
        let titles = row![logo, titles].spacing(14).align_y(iced::Alignment::Center);
        let ctl = |g: glyphs::Glyph, msg: Message, close: bool| {
            button(glyphs::glyph(g, glyphs::Tint::Chrome, 30.0))
                .width(30)
                .height(30)
                .padding(0)
                .on_press(msg)
                .style(theme::chrome_button(close))
        };
        let controls = row![
            ctl(glyphs::Glyph::Minimize, Message::Minimize, false),
            ctl(
                if self.maximized { glyphs::Glyph::Restore } else { glyphs::Glyph::Maximize },
                Message::ToggleMaximize,
                false
            ),
            ctl(glyphs::Glyph::Close, Message::CloseRequested(self.window.unwrap_or(window::Id::unique())), true),
        ]
        .spacing(6);
        // one row, centred on one line, so chips of different heights line up
        let mut tools = row![audio_btn].spacing(10).align_y(iced::Alignment::Center);
        if !self.no_device() {
            tools = tools.push(segmented); // nothing to turn around without a device
        }
        let tools = tools.push(about_btn);
        let bar = row![
            container(titles).padding(iced::Padding { top: 14.0, right: 0.0, bottom: 0.0, left: 28.0 }),
            space::horizontal(),
            container(tools).padding(iced::Padding { top: 22.0, right: 14.0, bottom: 0.0, left: 0.0 }),
            container(controls).padding(iced::Padding { top: 12.0, right: 12.0, bottom: 0.0, left: 0.0 }),
        ]
        .height(layout::HEADER_H as f32);
        // press to move the window, double-click to maximize
        mouse_area(bar).on_press(Message::Drag).on_double_click(Message::ToggleMaximize).into()
    }

    /// No MiniFuse to show: none was found, or it was unplugged.
    pub fn no_device(&self) -> bool {
        matches!(self.status, Status::NoDevice | Status::Disconnected)
    }

    /// The yellow "simulated" strip: only for a simulation the user asked for, or a device OpenFuse can't drive.
    fn shows_sim_banner(&self) -> bool {
        self.backend.simulated() && !self.no_device()
    }

    fn no_device_page(&self) -> Element<'_, Message> {
        let t = *self.tokens();
        let title = if self.status == Status::Disconnected { "The MiniFuse was disconnected" } else { "No MiniFuse connected" };
        let mut col = column![
            text(title).size(24).font(theme::BOLD).color(t.text),
            text("Plug it in with its USB cable: OpenFuse switches to it by itself.").size(15).color(t.text),
        ]
        .spacing(10)
        .align_x(iced::Alignment::Center);
        if self.device_watch.is_none() {
            col = col.push(text("(Not watching for one: restart OpenFuse once it is plugged in.)").size(13).color(t.muted));
        }
        col = col.push(text("To look around without one, start OpenFuse with --mock.").size(13).color(t.muted));
        container(col).width(Length::Fill).height(Length::Fill).center(Length::Fill).into()
    }

    /// Look for a MiniFuse in /proc/asound (from now on, while none is connected) and switch to it once it appears.
    pub fn watch_for_device(&mut self, card: Option<u32>, proc_root: PathBuf) {
        self.device_watch = Some((card, proc_root));
    }

    fn check_for_device(&mut self) {
        if !self.no_device() || self.last_detect.elapsed() < DETECT_INTERVAL {
            return;
        }
        let Some((card, root)) = self.device_watch.clone() else { return };
        self.last_detect = Instant::now();
        let found = crate::backend::detect::find_cards(&root).iter().any(|c| card.is_none_or(|n| c.index == n));
        if !found {
            return;
        }
        let be = crate::backend::detect::create_backend(false, None, card, &root, true);
        if be.status() != Status::NoDevice {
            self.switch_backend(be);
        }
    }

    /// Start over with another backend (a MiniFuse was plugged in), keeping the window, tray and settings.
    pub fn switch_backend(&mut self, backend: Box<dyn Backend>) {
        let mut next = App::new(backend, self.theme_mode, self.settings.clone());
        next.system_dark = self.system_dark;
        next.window = self.window;
        next.visible = self.visible;
        next.scale = self.scale;
        next.window_size = self.window_size;
        next.maximized = self.maximized;
        next.rounded = self.rounded;
        next.fake_tray = self.fake_tray;
        next.desktop_notifications = self.desktop_notifications;
        next.tray = self.tray.take();
        next.device_watch = self.device_watch.take();
        next.notifications = std::mem::take(&mut self.notifications);
        next.start();
        *self = next;
        let msg = match self.status {
            Status::Connected => format!("{} connected.", self.model.name),
            _ => self.backend.description(),
        };
        self.set_status(msg, HW_STATUS);
    }

    fn notice(&self) -> Element<'_, Message> {
        let t = *self.tokens();
        let mut msg = format!("{}. Nothing is sent to hardware.", self.backend.description());
        if !self.model.verified {
            msg.push_str(&format!(" The {} protocol is not characterised yet; this is a layout preview.", self.model.name));
        }
        let strip = container(
            row![
                container(text("SIMULATED").size(11).font(theme::BOLD)).padding([3, 9]).style(theme::pill(t.warn)),
                text(msg).size(13).color(t.text),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
        )
        .padding([0, 8])
        .height(layout::BANNER_H as f32 - 10.0)
        .center_y(layout::BANNER_H as f32 - 10.0)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(theme::alpha(t.warn, if t.dark { 0.10 } else { 0.08 }).into()),
            border: Border { color: theme::alpha(t.warn, 0.25), width: 1.0, radius: 12.0.into() },
            ..Default::default()
        });
        container(strip)
            .padding(iced::Padding { top: 0.0, right: 32.0, bottom: 10.0, left: 32.0 })
            .height(layout::BANNER_H as f32)
            .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let t = *self.tokens();
        let direct = self.backend.playback_stream().filter(|s| !s.via_server && !self.backend.output_metered());
        let plain = |msg: String, color: iced::Color| -> Element<'_, Message> { text(msg).size(13).color(color).into() };
        let content: Element<'_, Message> = match self.status_message() {
            Some(m) => plain(m.to_string(), t.text),
            None if self.no_device() => plain("Waiting for a MiniFuse…".into(), t.muted),
            None if direct.is_some() => plain(
                format!(
                    "{} plays directly to the MiniFuse (ALSA): output level can't be shown, only the inputs",
                    direct.map(|s| s.owner).unwrap_or_default()
                ),
                t.warn,
            ),
            None => {
                let muted = theme::alpha(t.muted, 0.9);
                let keys = |keys: &[&'static str], label: &'static str| -> Element<'_, Message> {
                    let mut r = row![].spacing(4).align_y(iced::Alignment::Center);
                    for k in keys {
                        r = r.push(
                            container(text(*k).size(11).font(theme::SEMIBOLD))
                                .padding([1, 6])
                                .center_x(Length::Shrink)
                                .style(theme::keycap),
                        );
                    }
                    r.push(text(label).size(13).color(muted)).spacing(6).into()
                };
                row![
                    text("Click a part of the device or its card").size(13).color(muted),
                    keys(&["F", "B"], "front / back"),
                    keys(&["L"], "loopback mixer"),
                    keys(&["A"], "audio settings"),
                ]
                .spacing(22)
                .align_y(iced::Alignment::Center)
                .into()
            }
        };
        let line = container(content)
            .padding(iced::Padding { top: 0.0, right: 32.0, bottom: 4.0, left: 32.0 })
            .center_y(Length::Fixed(layout::STATUS_H as f32))
            .width(Length::Fill);
        mouse_area(line).on_press(Message::Drag).into()
    }
}

pub const CORNER: f32 = 14.0;

/// The desktop's light/dark preference (XDG desktop portal), light if unknown.
pub fn prefers_dark() -> bool {
    zbus::block_on(async {
        let conn = zbus::Connection::session().await.ok()?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "ReadOne",
                &("org.freedesktop.appearance", "color-scheme"),
            )
            .await
            .ok()?;
        let v: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
        u32::try_from(v).ok()
    }) == Some(1)
}

/// Rounded, transparent corners need a compositor; Wayland always has one.
/// On X11 the corners stay square rather than risk black corners.
pub fn rounded_window() -> bool {
    cfg!(feature = "gpu")
        && (std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false)
            || std::env::var_os("WAYLAND_DISPLAY").is_some())
}

/// Invisible edge and corner grips: the window has no decorations, so it
/// resizes through these (winit's drag-resize).
fn resize_handles<'a>() -> Element<'a, Message> {
    use iced::mouse::Interaction as I;
    use window::Direction as D;
    const E: f32 = 6.0;
    const C: f32 = 14.0;
    let grip = |w: Length, h: Length, dir: D, cursor: I| -> Element<'a, Message> {
        mouse_area(space::horizontal().width(w).height(h)).on_press(Message::DragResize(dir)).interaction(cursor).into()
    };
    let fill = Length::Fill;
    column![
        row![
            grip(C.into(), E.into(), D::NorthWest, I::ResizingDiagonallyDown),
            grip(fill, E.into(), D::North, I::ResizingVertically),
            grip(C.into(), E.into(), D::NorthEast, I::ResizingDiagonallyUp),
        ],
        row![
            column![grip(E.into(), (C - E).into(), D::NorthWest, I::ResizingDiagonallyDown), grip(E.into(), fill, D::West, I::ResizingHorizontally), grip(E.into(), (C - E).into(), D::SouthWest, I::ResizingDiagonallyUp)],
            space::horizontal().width(fill).height(fill),
            column![grip(E.into(), (C - E).into(), D::NorthEast, I::ResizingDiagonallyUp), grip(E.into(), fill, D::East, I::ResizingHorizontally), grip(E.into(), (C - E).into(), D::SouthEast, I::ResizingDiagonallyDown)],
        ]
        .height(fill),
        row![
            grip(C.into(), E.into(), D::SouthWest, I::ResizingDiagonallyUp),
            grip(fill, E.into(), D::South, I::ResizingVertically),
            grip(C.into(), E.into(), D::SouthEast, I::ResizingDiagonallyDown),
        ],
    ]
    .into()
}

// ---------------------------------------------------------------- external commands

static EXTERNAL_RX: Mutex<Option<UnboundedReceiver<External>>> = Mutex::new(None);

/// The channel through which the tray and the single-instance listener talk
/// to the app. Call once, before running iced.
pub fn external_channel() -> UnboundedSender<External> {
    let (tx, rx) = unbounded();
    *EXTERNAL_RX.lock().unwrap() = Some(rx);
    tx
}

fn external_stream() -> impl iced::futures::Stream<Item = External> {
    iced::stream::channel(32, async |mut out| {
        let rx = EXTERNAL_RX.lock().unwrap().take();
        if let Some(mut rx) = rx {
            while let Some(e) = rx.next().await {
                if out.send(e).await.is_err() {
                    break;
                }
            }
        }
        iced::futures::future::pending::<()>().await;
    })
}
