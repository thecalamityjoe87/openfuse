//! GAIN calibration wizard: the user turns GAIN to each of five known
//! positions and the idle level of the input is recorded there, with their
//! own instrument (or nothing) plugged in. `Calibration::from_measured` turns
//! the five levels into a knob-position estimate that fits their setup.

use crate::knobs::{CAL_STEPS, Calibration};

/// Seconds to let the level settle after the click, then record.
pub const SETTLE_S: f64 = 2.0;
pub const RECORD_S: f64 = 6.0;
/// Fewest readings that make a recording.
const MIN_READINGS: usize = 20;
/// Spread (90th - 10th percentile) up to which a recording is steady.
const STEADY_SPREAD_DB: f64 = 6.0;
/// What digital silence counts as, in dBFS.
const SILENCE_DB: f64 = -98.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// waiting for the user to set the knob and press Record
    Ready,
    Recording { since: f64, readings: Vec<f64> },
    /// recorded: the median level
    Done(f64),
    /// the level wandered (playing, touching the instrument): try again
    Unsteady(f64),
    /// all five recorded
    Summary,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Wizard {
    /// the knob (part key) being calibrated
    pub knob: &'static str,
    /// the meter word of its input
    pub meter: usize,
    /// INST when the wizard opened (None: the input has no INST switch); the
    /// calibration is saved for this setting
    pub inst: Option<bool>,
    pub step: usize,
    /// median idle level per step
    pub floors: Vec<Option<f64>>,
    pub phase: Phase,
    /// the latest reading, shown live so the user can see the input respond
    pub live_db: Option<f64>,
}

impl Wizard {
    pub fn new(knob: &'static str, meter: usize) -> Wizard {
        Wizard { knob, meter, inst: None, step: 0, floors: vec![None; CAL_STEPS.len()], phase: Phase::Ready, live_db: None }
    }

    pub fn steps(&self) -> usize {
        CAL_STEPS.len()
    }

    /// Start (or redo) the recording of this step at time `t` (seconds).
    pub fn record(&mut self, t: f64) {
        if matches!(self.phase, Phase::Ready | Phase::Done(_) | Phase::Unsteady(_)) {
            self.phase = Phase::Recording { since: t, readings: Vec::new() };
        }
    }

    /// Hand over a meter reading (dBFS) taken at `t` seconds.
    pub fn feed(&mut self, t: f64, db: f64) {
        let db = if db.is_finite() { db } else { SILENCE_DB };
        self.live_db = Some(db);
        let Phase::Recording { since, readings } = &mut self.phase else { return };
        if t - *since >= SETTLE_S {
            readings.push(db);
        }
        if t - *since < SETTLE_S + RECORD_S {
            return;
        }
        let mut v = std::mem::take(readings);
        if v.len() < MIN_READINGS {
            self.phase = Phase::Failed("No meter readings arrived. Is the MiniFuse connected?".into());
            return;
        }
        v.sort_by(f64::total_cmp);
        let median = v[v.len() / 2];
        let spread = v[v.len() * 9 / 10] - v[v.len() / 10];
        if spread > STEADY_SPREAD_DB {
            self.phase = Phase::Unsteady(spread);
        } else {
            self.floors[self.step] = Some(median);
            self.phase = Phase::Done(median);
        }
    }

    /// Fraction of the recording done, 0..1.
    pub fn progress(&self, t: f64) -> f64 {
        match &self.phase {
            Phase::Recording { since, .. } => ((t - since) / (SETTLE_S + RECORD_S)).clamp(0.0, 1.0),
            Phase::Done(_) | Phase::Summary => 1.0,
            _ => 0.0,
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.phase, Phase::Recording { .. })
    }

    /// After a good recording: on to the next position, or the summary.
    pub fn next(&mut self) {
        if !matches!(self.phase, Phase::Done(_)) {
            return;
        }
        if self.step + 1 < self.steps() {
            self.step += 1;
            self.phase = Phase::Ready;
        } else {
            self.phase = Phase::Summary;
        }
    }

    /// Back one position (its level is re-recorded) or out of the summary / an error.
    pub fn back(&mut self) {
        if self.is_recording() {
            return;
        }
        if matches!(self.phase, Phase::Summary | Phase::Failed(_)) {
            self.step = self.steps() - 1;
        } else if self.step > 0 {
            self.step -= 1;
        }
        self.phase = Phase::Ready;
    }

    /// The measured positions and levels, once all are recorded.
    pub fn measured(&self) -> Option<Vec<(f64, f64)>> {
        CAL_STEPS.iter().zip(&self.floors).map(|(s, f)| f.map(|f| (s.0, f))).collect()
    }

    pub fn calibration(&self) -> Result<Calibration, String> {
        Calibration::from_measured(&self.measured().ok_or("Record every position first.")?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run one recording at constant `db` (25 Hz), starting at `t0`; returns the end time.
    fn record(w: &mut Wizard, t0: f64, db: f64) -> f64 {
        w.record(t0);
        let mut t = t0;
        while w.is_recording() {
            w.feed(t, db);
            t += 0.04;
        }
        t
    }

    #[test]
    fn five_steady_recordings_make_a_calibration() {
        let mut w = Wizard::new("in1-gain", 0);
        let mut t = 0.0;
        for (i, floor) in [-70.6, -62.0, -41.4, -26.6, -15.8].into_iter().enumerate() {
            assert_eq!((w.step, &w.phase), (i, &Phase::Ready));
            t = record(&mut w, t, floor);
            assert!(matches!(w.phase, Phase::Done(f) if (f - floor).abs() < 1e-9));
            w.next();
        }
        assert_eq!(w.phase, Phase::Summary);
        let cal = w.calibration().unwrap();
        assert!(cal.learned && (cal.position(-41.4) - 0.5).abs() < 0.02);
    }

    #[test]
    fn the_first_seconds_are_not_recorded_and_the_progress_runs() {
        let mut w = Wizard::new("in1-gain", 0);
        w.record(10.0);
        for i in 0..50 {
            w.feed(10.0 + i as f64 * 0.04, -10.0); // the settling time: a hand still on the knob
        }
        assert!((w.progress(10.0 + SETTLE_S) - SETTLE_S / (SETTLE_S + RECORD_S)).abs() < 1e-9);
        let mut t = 10.0 + 2.0;
        while w.is_recording() {
            w.feed(t, -50.0);
            t += 0.04;
        }
        assert_eq!(w.phase, Phase::Done(-50.0), "the loud settling readings were not counted");
    }

    #[test]
    fn an_unsteady_input_is_refused_and_can_be_retried() {
        let mut w = Wizard::new("in1-gain", 0);
        w.record(0.0);
        let mut t = 0.0;
        let mut i = 0;
        while w.is_recording() {
            w.feed(t, if i % 2 == 0 { -20.0 } else { -60.0 });
            t += 0.04;
            i += 1;
        }
        assert!(matches!(w.phase, Phase::Unsteady(s) if s > 30.0));
        assert_eq!(w.floors[0], None, "nothing kept");
        w.next();
        assert!(matches!(w.phase, Phase::Unsteady(_)) && w.step == 0, "can't move on");
        record(&mut w, t, -70.0);
        assert!(matches!(w.phase, Phase::Done(_)));
    }

    #[test]
    fn no_readings_is_an_error_and_back_steps_out_of_it() {
        let mut w = Wizard::new("in1-gain", 0);
        w.record(0.0);
        w.feed(100.0, -50.0); // time jumped past the end with almost no readings
        assert!(matches!(w.phase, Phase::Failed(_)));
        w.back();
        assert_eq!((w.step, &w.phase), (CAL_STEPS.len() - 1, &Phase::Ready));
    }

    #[test]
    fn back_redoes_the_previous_position_and_a_missing_step_blocks_saving() {
        let mut w = Wizard::new("in1-gain", 0);
        let t = record(&mut w, 0.0, -70.0);
        w.next();
        w.back();
        assert_eq!(w.step, 0);
        assert!(w.measured().is_none() || w.floors[1].is_none());
        assert!(w.calibration().is_err());
        let _ = t;
    }

    #[test]
    fn digital_silence_is_recorded_as_a_floor() {
        let mut w = Wizard::new("in1-gain", 0);
        record(&mut w, 0.0, f64::NEG_INFINITY);
        assert_eq!(w.phase, Phase::Done(-98.0));
    }
}
