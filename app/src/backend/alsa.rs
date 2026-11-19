//! Real device backend: talks to the OpenFuse ALSA controls (spec §7.1).

use super::alsa_ffi::{self as al, AlsaError, Ctl, Elem, EVENT_MASK_REMOVE, EVENT_MASK_VALUE};
use super::detect::FoundCard;
use super::{Backend, Event, Status, Value};
use crate::alsa_controls::{self as names, RAW_VOLUME_FALLBACK};
use crate::levels::{SILENCE, raw_to_dbfs};
use crate::models::ModelSpec;
use crate::alsa_stream::{self, StreamInfo};
use crate::playback::PlaybackMeter;
use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::{Duration, Instant};

/// safety net in case an event is missed
const RESYNC_INTERVAL: Duration = Duration::from_secs(1);
const STREAM_INTERVAL: Duration = Duration::from_millis(500);
/// How often the card's /proc/asound entry is checked (an unplug doesn't always come as an event).
const PRESENCE_INTERVAL: Duration = Duration::from_secs(1);

/// Reads/writes the card's ALSA controls, follows change events, polls meters.
///
/// Controls that the driver does not expose are left out of `keys()`; the UI
/// shows those parts as unavailable.
pub struct AlsaBackend {
    model: &'static ModelSpec,
    card: FoundCard,
    ctl: Option<Ctl>,
    elems: BTreeMap<String, Elem>,
    by_name: HashMap<(String, u32), String>,
    meter: Option<Elem>,
    /// the driver's own meter of the playback stream, when the module has it
    playback_meter: Option<Elem>,
    values: BTreeMap<String, Value>,
    meters: Vec<f64>,
    playback: PlaybackMeter,
    stream: Option<StreamInfo>,
    last_stream: Instant,
    meter_error_logged: bool,
    live: bool,
    dead: bool,
    subscribed: bool,
    status: Status,
    last_resync: Instant,
    last_presence: Instant,
}

impl AlsaBackend {
    pub fn new(card: FoundCard) -> Result<AlsaBackend, AlsaError> {
        let model = card.model;
        let ctl = Ctl::open(&card.hw())?;
        let mut elems = BTreeMap::new();
        let mut by_name = HashMap::new();
        for key in model.control_keys() {
            let Some(an) = names::lookup(&key) else { continue };
            match ctl.element(&an.name, an.index, an.iface) {
                Ok(el) => {
                    by_name.insert((an.name.clone(), an.index), key.clone());
                    elems.insert(key, el);
                }
                Err(_) => log(&format!("control not present: {} ({key})", an.name)),
            }
        }
        let an = names::lookup("meters").expect("meters name");
        let meter = ctl.element(&an.name, an.index, an.iface).ok();
        if meter.is_none() {
            log(&format!("meter control not present: {}", an.name));
        }
        let pn = names::lookup("playback_meters").expect("playback meters name");
        let playback_meter = ctl.element(&pn.name, pn.index, pn.iface).ok();
        if playback_meter.is_none() {
            log("no playback meter in the driver (older module): measuring the sound server's sink instead");
        }
        Ok(AlsaBackend {
            model,
            meters: vec![SILENCE; model.meters.labels.len()],
            playback: PlaybackMeter::new(card.index),
            stream: alsa_stream::playback_stream(Path::new("/proc/asound"), card.index),
            last_stream: Instant::now(),
            last_presence: Instant::now(),
            meter_error_logged: false,
            card,
            ctl: Some(ctl),
            elems,
            by_name,
            meter,
            playback_meter,
            values: BTreeMap::new(),
            live: true,
            dead: false,
            subscribed: false,
            status: Status::Connected,
            last_resync: Instant::now(),
        })
    }

    fn raw_to_db(&self, el: &Elem, raw: i64) -> f64 {
        if el.has_db
            && let Some(ctl) = &self.ctl
            && let Ok(db) = ctl.to_db(el, raw)
        {
            return db;
        }
        raw as f64 * RAW_VOLUME_FALLBACK.scale_db_per_step + RAW_VOLUME_FALLBACK.offset_db
    }

    fn db_to_raw(&self, el: &Elem, mut db: f64) -> i64 {
        if let Some(lb) = self.model.loopback
            && db < lb.gain_min_db
        {
            db = lb.gain_off_db;
        }
        if el.has_db
            && let Some(ctl) = &self.ctl
            && let Ok(raw) = ctl.raw_from_db(el, db)
        {
            return raw.clamp(el.min, el.max);
        }
        let raw = ((db - RAW_VOLUME_FALLBACK.offset_db) / RAW_VOLUME_FALLBACK.scale_db_per_step).round() as i64;
        raw.clamp(el.min, el.max)
    }

    fn read(&self, key: &str) -> Result<Value, AlsaError> {
        let el = &self.elems[key];
        let ctl = self.ctl.as_ref().ok_or_else(|| AlsaError("closed".into()))?;
        let vals = ctl.read(el)?;
        let v0 = vals.first().copied().unwrap_or(0);
        Ok(if el.is_boolean() {
            Value::Bool(v0 != 0)
        } else {
            Value::Db((self.raw_to_db(el, v0) * 100.0).round() / 100.0)
        })
    }

    fn refresh(&mut self, key: &str) -> Vec<Event> {
        match self.read(key) {
            Ok(v) if self.values.get(key) != Some(&v) => {
                self.values.insert(key.to_string(), v);
                vec![Event::ValueChanged(key.to_string(), v)]
            }
            Ok(_) => vec![],
            Err(e) => vec![Event::Error(e.0)],
        }
    }

    fn resync(&mut self) -> Vec<Event> {
        self.last_resync = Instant::now();
        let keys: Vec<String> = self.elems.keys().cloned().collect();
        keys.iter().flat_map(|k| self.refresh(k)).collect()
    }

    /// Device meter words fill the entries that are not host outputs, in
    /// order; the host outputs come from the playback stream (the device
    /// does not meter playback, spec §5).
    fn poll_meters(&mut self) -> Vec<Event> {
        let spec = &self.model.meters;
        let n = spec.labels.len();
        let mut vals = vec![SILENCE; n];
        if let (Some(ctl), Some(el)) = (self.ctl.as_ref(), self.meter.as_ref()) {
            match ctl.read(el) {
                Ok(raw) => {
                    let slots = (0..n).filter(|i| !spec.host_outputs.contains(i));
                    for (slot, r) in slots.zip(raw) {
                        vals[slot] = raw_to_dbfs(r);
                    }
                }
                Err(e) if !self.meter_error_logged => {
                    self.meter_error_logged = true;
                    log(&format!("reading the meters failed: {e}"));
                }
                Err(_) => {}
            }
        }
        let outputs = match (self.ctl.as_ref(), self.playback_meter.as_ref()) {
            // the driver sees every sample sent to the card, from any application
            (Some(ctl), Some(el)) => match ctl.read(el) {
                Ok(raw) => raw.iter().map(|r| raw_to_dbfs(*r)).collect(),
                Err(_) => vec![SILENCE; 2],
            },
            _ => self.playback.take().to_vec(),
        };
        for (slot, db) in spec.host_outputs.iter().zip(outputs) {
            vals[*slot] = db;
        }
        self.meters = vals.clone();
        let mut out = vec![Event::Meters(vals)];
        if self.playback.take_unavailable() {
            out.push(Event::Error("Output meters need pw-record (PipeWire) or parec (PulseAudio).".into()));
        }
        out
    }

    /// The device went away: say so and stop.
    fn disconnected(&mut self) -> Vec<Event> {
        self.status = Status::Disconnected;
        self.die();
        vec![Event::Error("The device was disconnected.".into()), Event::StatusChanged(Status::Disconnected)]
    }

    fn die(&mut self) {
        self.playback.set_running(false);
        self.dead = true;
    }
}

/// Has card `index` gone from /proc/asound (the device was unplugged)?
fn card_gone(proc_root: &Path, index: u32) -> bool {
    proc_root.join("cards").exists() && !proc_root.join(format!("card{index}")).exists()
}

fn log(msg: &str) {
    if std::env::var_os("OPENFUSE_DEBUG").is_some() {
        eprintln!("openfuse: {msg}");
    }
}

impl Backend for AlsaBackend {
    fn model(&self) -> &'static ModelSpec {
        self.model
    }
    fn simulated(&self) -> bool {
        false
    }
    fn status(&self) -> Status {
        self.status
    }
    fn description(&self) -> String {
        format!("{} · card {} ({})", self.model.name, self.card.index, self.card.hw())
    }

    fn start(&mut self) -> Vec<Event> {
        let mut out = self.resync();
        if let Some(ctl) = &self.ctl {
            match ctl.subscribe() {
                Ok(()) => self.subscribed = true,
                Err(e) => log(&format!("no ALSA change events ({e}); relying on polling")),
            }
        }
        out.extend(self.poll_meters());
        out
    }

    fn stop(&mut self) {
        self.elems.clear();
        self.meter = None;
        self.playback_meter = None;
        self.ctl = None;
        self.playback.set_running(false);
        self.dead = true;
    }

    fn keys(&self) -> Vec<String> {
        self.elems.keys().cloned().collect()
    }

    fn get(&self, key: &str) -> Option<Value> {
        self.values.get(key).copied()
    }

    fn set(&mut self, key: &str, value: Value) -> Vec<Event> {
        let Some(el) = self.elems.get(key) else {
            return vec![Event::Error(format!("{key}: control not available"))];
        };
        let mut out = vec![];
        let raw = if el.is_boolean() { i64::from(value.as_bool()) } else { self.db_to_raw(el, value.as_db()) };
        let vals = vec![raw; el.count as usize];
        if let Some(ctl) = &self.ctl
            && let Err(e) = ctl.write(el, &vals)
        {
            out.push(Event::Error(format!("Could not change {}: {e}", self.model.label_for(key))));
        }
        out.extend(self.refresh(key)); // read back: the device is the source of truth
        out
    }

    fn meters(&self) -> Vec<f64> {
        self.meters.clone()
    }

    fn set_live(&mut self, on: bool) {
        let was = self.live;
        self.live = on;
        self.playback.set_running(on && !self.dead && self.playback_meter.is_none());
        if on && !was && !self.dead {
            // catch anything an event might have missed on the next poll
            self.last_resync = Instant::now() - RESYNC_INTERVAL;
        }
    }

    fn output_metered(&self) -> bool {
        self.playback_meter.is_some()
    }

    fn playback_stream(&self) -> Option<StreamInfo> {
        self.stream.clone()
    }

    fn meters_polling(&self) -> bool {
        self.live && !self.dead
    }

    fn poll(&mut self) -> Vec<Event> {
        if self.ctl.is_none() || self.status == Status::Disconnected {
            return vec![];
        }
        // checked even after an error stopped the backend: the card may go a moment later
        if self.last_presence.elapsed() >= PRESENCE_INTERVAL {
            self.last_presence = Instant::now();
            if card_gone(Path::new("/proc/asound"), self.card.index) {
                return self.disconnected();
            }
        }
        if self.dead {
            return vec![];
        }
        let mut out = vec![];
        if self.subscribed {
            let events = self.ctl.as_ref().map(|c| c.read_events()).unwrap_or(Ok(vec![]));
            match events {
                // reading fails once the device is unplugged (the card can still be listed for a
                // moment); if it is in fact still there, the app finds it again and reconnects
                Err(e) => {
                    log(&format!("reading device events failed: {e}"));
                    return self.disconnected();
                }
                Ok(evs) => {
                    for (name, index, mask) in evs {
                        if mask == EVENT_MASK_REMOVE {
                            return self.disconnected();
                        }
                        if mask & EVENT_MASK_VALUE != 0
                            && let Some(key) = self.by_name.get(&(name, index)).cloned()
                        {
                            out.extend(self.refresh(&key));
                        }
                    }
                }
            }
        }
        let resync_due = self.last_resync.elapsed() >= RESYNC_INTERVAL;
        if resync_due && (self.live || !self.subscribed) {
            out.extend(self.resync());
        }
        if self.live {
            out.extend(self.poll_meters());
            if self.last_stream.elapsed() >= STREAM_INTERVAL {
                self.last_stream = Instant::now();
                self.stream = alsa_stream::playback_stream(Path::new("/proc/asound"), self.card.index);
            }
        }
        out
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Diagnostic for --list-controls: every element on the card, and which
/// OpenFuse keys resolved.
#[allow(clippy::type_complexity)]
pub fn list_card_controls(hw: &str) -> Result<(Vec<(&'static str, String, u32)>, Vec<(String, bool)>), String> {
    let ctl = Ctl::open(hw).map_err(|e| e.0)?;
    let present = ctl.list_elements().map_err(|e| e.0)?;
    let mapping = names::alsa_controls()
        .iter()
        .map(|(k, a)| (k.clone(), present.iter().any(|(_, n, i)| *n == a.name && *i == a.index)))
        .collect();
    Ok((present, mapping))
}

#[allow(dead_code)]
fn _assert_send() {
    fn is_send<T: Send>() {}
    is_send::<al::Ctl>();
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_card_missing_from_proc_is_gone() {
        let t = tempfile::tempdir().unwrap();
        assert!(!super::card_gone(t.path(), 3), "no /proc/asound at all: can't tell, so not gone");
        std::fs::write(t.path().join("cards"), "").unwrap();
        std::fs::create_dir(t.path().join("card3")).unwrap();
        assert!(!super::card_gone(t.path(), 3));
        assert!(super::card_gone(t.path(), 4));
    }

    /// Read-only smoke test of the FFI layer against the first sound card
    /// (lists element names; never writes). `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn list_controls_of_card0() {
        let (present, mapping) = super::list_card_controls("hw:0").expect("hw:0");
        assert!(!present.is_empty());
        assert!(!mapping.is_empty());
        eprintln!("{} controls, e.g. {:?}", present.len(), present.first());
    }
}
