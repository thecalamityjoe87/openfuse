//! Simulated MiniFuse, so the whole UI runs without the kernel module.

use super::{Backend, Event, Status, Value};
use crate::levels::{dbfs_to_raw, raw_to_dbfs};
use crate::models::ModelSpec;
use std::any::Any;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Small deterministic PRNG (xorshift64*), enough for a simulation.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: Option<u64>) -> Rng {
        let s = seed.unwrap_or_else(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            t.as_nanos() as u64 ^ (std::process::id() as u64) << 32
        });
        Rng(s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// uniform in [lo, hi)
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64)
    }
    /// uniform integer in [lo, hi]
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next_u64() % (hi - lo + 1)
    }
}

/// Simulates a MiniFuse (any model in the table; MiniFuse 2 by default).
///
/// * switches and loopback values hold state and echo writes back
/// * meters are animated (two "performers" on the inputs, music on playback)
///   and produced in the device's raw form (u32 peak samples, spec §5), then
///   decoded by the same `levels::raw_to_dbfs` the ALSA backend uses
/// * every now and then a front-panel button is "pressed on the hardware"
///
/// `animate = false` freezes meters and hardware presses (tests, screenshots).
pub struct MockBackend {
    model: &'static ModelSpec,
    description: String,
    status: Status,
    live: bool,
    animate: bool,
    rng: Rng,
    hw_interval: (f64, f64),
    started: Instant,
    next_press: Option<Instant>,
    state: BTreeMap<String, Value>,
    /// last raw words, as GET_METERS returns them
    pub meter_raw: Vec<u32>,
    meters: Vec<f64>,
    pending: Vec<Event>,
}

impl MockBackend {
    pub fn new(model: &'static ModelSpec, reason: &str, animate: bool, status: Status, seed: Option<u64>) -> Self {
        let mut state = BTreeMap::new();
        for k in model.toggles {
            state.insert(k.to_string(), Value::Bool(false));
        }
        if let Some(lb) = model.loopback {
            for (k, _, _) in lb.switches {
                state.insert(k.to_string(), Value::Bool(false));
            }
            for n in 0..lb.channels.len() {
                state.insert(format!("lb_vol_l_{n}"), Value::Db(lb.gain_off_db));
                state.insert(format!("lb_vol_r_{n}"), Value::Db(lb.gain_off_db));
                state.insert(format!("lb_mute_{n}"), Value::Bool(false));
            }
        }
        for (k, v) in model.defaults {
            state.insert(k.to_string(), Value::Db(*v));
        }
        let n = model.meters.labels.len();
        MockBackend {
            model,
            description: if reason.is_empty() { "Simulated device".into() } else { reason.into() },
            status,
            live: true,
            animate,
            rng: Rng::new(seed),
            hw_interval: (18.0, 45.0),
            started: Instant::now(),
            next_press: None,
            state,
            meter_raw: vec![0; n],
            meters: vec![raw_to_dbfs(0); n],
            pending: Vec::new(),
        }
    }

    /// Plain simulated device for tests and screenshots (no animation).
    pub fn still(model: &'static ModelSpec) -> Self {
        MockBackend::new(model, "Simulated device (--mock)", false, Status::Simulated, Some(0))
    }

    fn update(&mut self, key: &str, value: Value) -> Vec<Event> {
        if self.state.get(key) != Some(&value) {
            self.state.insert(key.to_string(), value);
            vec![Event::ValueChanged(key.to_string(), value)]
        } else {
            vec![]
        }
    }

    /// Toggle a switch as if its front-panel button had been pressed. The
    /// change is reported by the next `poll`, like a real device event.
    pub fn simulate_hardware_press(&mut self, key: &str) {
        let cur = self.state.get(key).map(|v| v.as_bool()).unwrap_or(false);
        let ev = self.update(key, Value::Bool(!cur));
        self.pending.extend(ev);
    }

    /// Force raw meter words (tests / screenshots).
    pub fn set_meter_raw(&mut self, raw: &[u32]) {
        self.meter_raw = raw.to_vec();
        self.meters = raw.iter().map(|v| raw_to_dbfs(*v as i64)).collect();
        self.pending.push(Event::Meters(self.meters.clone()));
    }

    /// Force meter values given in dBFS (converted to raw first).
    pub fn set_meter_levels(&mut self, dbfs: &[f64]) {
        let raw: Vec<u32> = dbfs.iter().map(|d| dbfs_to_raw(*d)).collect();
        self.set_meter_raw(&raw);
    }

    fn schedule_press(&mut self) {
        let (lo, hi) = self.hw_interval;
        self.next_press = Some(Instant::now() + Duration::from_secs_f64(self.rng.uniform(lo, hi)));
    }

    /// idle input noise, about -110..-97 dBFS
    fn idle_raw(&mut self) -> u32 {
        (self.rng.range(0x75, 0xF3) as u32) << 8
    }

    /// Plausible dBFS levels of the two inputs and playback.
    fn source_levels(&mut self, t: f64) -> (f64, f64, f64) {
        let mut env = |rate: f64, phase: f64, base: f64, depth: f64| {
            let s = 0.5 + 0.5 * (t * rate + phase).sin() * (t * rate * 0.37 + phase * 2.0).sin();
            base + depth * s + self.rng.uniform(-2.5, 2.5)
        };
        let mut voice = env(5.3, 0.0, -34.0, 26.0); // -34 ... -8 dB
        let guitar = env(3.1, 1.3, -30.0, 22.0);
        let music = env(7.7, 2.1, -22.0, 14.0);
        if self.state.get("in1_inst") == Some(&Value::Bool(true)) {
            voice -= 4.0;
        }
        (voice, guitar, music)
    }

    fn tick_meters(&mut self) {
        let t = self.started.elapsed().as_secs_f64();
        let n = self.model.meters.labels.len();
        let (mut voice, guitar, music) = self.source_levels(t);
        // now and then the singer gets too loud: a clipping peak (~every 15 s)
        if self.rng.range(0, 374) == 0 {
            voice = 0.0;
        }
        let _ = self.model.loopback;
        // inputs get the performers, the rest (output / loopback pair) the music
        let n_in = self.model.meters.labels.iter().filter(|l| l.starts_with("Input")).count();
        let mut levels: Vec<f64> = (0..n)
            .map(|i| match i {
                0 => voice,
                1 if n_in > 1 => guitar,
                i if i < n_in => guitar - 3.0 * i as f64,
                i => music - 1.5 * ((i - n_in) % 2) as f64,
            })
            .collect();
        // output pair: playback plus the direct-monitored inputs (measured on the
        // host on real hardware); the loopback pair follows it
        let mix = 10f64.powf(music / 20.0) + 0.5 * (10f64.powf(voice / 20.0) + 10f64.powf(guitar / 20.0));
        for l in levels.iter_mut().skip(n_in) {
            *l = (*l - music) + 20.0 * mix.log10();
        }
        let silent: Vec<usize> = vec![];
        let mut raw = Vec::with_capacity(n);
        for (i, db) in levels.iter().enumerate() {
            if silent.contains(&i) {
                raw.push(0);
            } else {
                let idle = self.idle_raw();
                raw.push(dbfs_to_raw(*db).max(idle));
            }
        }
        self.set_meter_raw(&raw);
    }

    fn random_hw_press(&mut self) {
        // 48 V is deliberately not simulated: it would be alarming in a demo.
        let candidates: Vec<&str> = self.model.toggles.iter().copied().filter(|k| *k != "phantom").collect();
        if !candidates.is_empty() {
            let k = candidates[self.rng.range(0, candidates.len() as u64 - 1) as usize];
            self.simulate_hardware_press(k);
        }
        self.schedule_press();
    }
}

impl Backend for MockBackend {
    fn model(&self) -> &'static ModelSpec {
        self.model
    }
    fn simulated(&self) -> bool {
        true
    }
    fn status(&self) -> Status {
        self.status
    }
    fn description(&self) -> String {
        self.description.clone()
    }

    fn start(&mut self) -> Vec<Event> {
        if self.animate {
            self.tick_meters();
            self.schedule_press();
        }
        std::mem::take(&mut self.pending)
    }

    fn stop(&mut self) {
        self.next_press = None;
        self.animate = false;
    }

    fn keys(&self) -> Vec<String> {
        self.state.keys().cloned().collect()
    }

    fn get(&self, key: &str) -> Option<Value> {
        self.state.get(key).copied()
    }

    fn set(&mut self, key: &str, value: Value) -> Vec<Event> {
        if !self.state.contains_key(key) {
            return vec![Event::Error(format!("Unknown control {key}"))];
        }
        let value = if key.starts_with("lb_vol_") {
            let lb = self.model.loopback.expect("loopback model");
            let mut v = value.as_db();
            if v < lb.gain_min_db {
                v = lb.gain_off_db;
            }
            Value::Db(v.min(lb.gain_max_db))
        } else {
            Value::Bool(value.as_bool())
        };
        self.update(key, value)
    }

    fn meters(&self) -> Vec<f64> {
        self.meters.clone()
    }

    fn set_live(&mut self, on: bool) {
        self.live = on; // hardware presses keep coming while hidden
    }

    fn meters_polling(&self) -> bool {
        self.live && self.animate
    }

    fn poll(&mut self) -> Vec<Event> {
        if self.animate {
            if self.live {
                self.tick_meters();
            }
            if self.next_press.is_some_and(|t| Instant::now() >= t) {
                self.random_hw_press();
            }
        }
        std::mem::take(&mut self.pending)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MINIFUSE2;

    #[test]
    fn mock_meters_are_raw() {
        let mut be = MockBackend::new(&MINIFUSE2, "", true, Status::Simulated, Some(3));
        be.start();
        assert_eq!(be.meter_raw.len(), MINIFUSE2.meters.labels.len());
        assert!(be.meter_raw.iter().all(|v| v & 0xFF == 0));
        assert!((be.meters()[0] - raw_to_dbfs(be.meter_raw[0] as i64)).abs() < 1e-9);
        // the output pair carries signal
        assert!(be.meter_raw[2] > 0x10000 && be.meter_raw[3] > 0x10000);
        for m in crate::models::MODELS {
            let mut be = MockBackend::new(m, "", true, Status::Simulated, Some(2));
            be.start();
            assert_eq!(be.meters().len(), m.meters.labels.len());
        }
    }

    #[test]
    fn mock_clips_now_and_then() {
        let mut be = MockBackend::new(&MINIFUSE2, "", true, Status::Simulated, Some(7));
        be.start();
        let mut clips = 0;
        for _ in 0..3000 {
            be.poll();
            if crate::levels::is_clip(be.meters()[0]) {
                clips += 1;
            }
        }
        assert!((1..60).contains(&clips), "occasional clipping peaks, got {clips}");
    }

    #[test]
    fn gains_clamp_and_snap_to_off() {
        let mut be = MockBackend::still(&MINIFUSE2);
        be.set("lb_vol_l_0", Value::Db(40.0));
        assert_eq!(be.get("lb_vol_l_0"), Some(Value::Db(12.0)));
        be.set("lb_vol_l_0", Value::Db(-61.0));
        assert_eq!(be.get("lb_vol_l_0"), Some(Value::Db(-63.0)));
        assert!(matches!(be.set("nope", Value::Bool(true))[0], Event::Error(_)));
    }

    #[test]
    fn hidden_stops_meter_polling() {
        let mut be = MockBackend::new(&MINIFUSE2, "", true, Status::Simulated, Some(1));
        be.start();
        assert!(be.meters_polling());
        be.set_live(false);
        assert!(!be.meters_polling());
        assert!(be.poll().iter().all(|e| !matches!(e, Event::Meters(_))));
        be.set_live(true);
        assert!(be.poll().iter().any(|e| matches!(e, Event::Meters(_))));
    }
}
