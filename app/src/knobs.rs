//! Virtual knobs: where to draw the analog knobs the device cannot report.
//!
//! The MiniFuse's knobs are plain analog controls (docs/protocol.md §4.2: no
//! state entry changes when they turn), so a position on screen is either set
//! by the user or, for the input GAIN knobs only, estimated: the input meters
//! are post-gain, so with a quiet input the meter's noise floor rises with the
//! knob. The estimate is rough (a few dB) and only valid while nothing but
//! noise reaches the input.

use std::collections::VecDeque;

/// Pointer angle at minimum, in degrees clockwise from 3 o'clock (screen
/// coordinates): 135 = pointing to 7:30.
pub const SWEEP_START_DEG: f64 = 135.0;
/// Rotation from minimum to maximum (to 4:30).
pub const SWEEP_DEG: f64 = 270.0;

/// Knob position 0..1 -> pointer angle in degrees.
pub fn angle_deg(pos: f64) -> f64 {
    SWEEP_START_DEG + SWEEP_DEG * pos.clamp(0.0, 1.0)
}

/// Pointer angle (any degrees) -> position, if it lies within the sweep.
pub fn pos_from_angle(deg: f64) -> Option<f64> {
    let rel = (deg - SWEEP_START_DEG).rem_euclid(360.0);
    (rel <= SWEEP_DEG).then(|| rel / SWEEP_DEG)
}

pub fn percent(pos: f64) -> u32 {
    (pos.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// What the idle noise floor of an input (peak dBFS, nothing plugged in) is
/// at known knob positions. The knob is not linear: the floor is converter
/// noise over the bottom fifth of the travel, then rises about 22 dB per unit
/// of rotation up to 12 o'clock, about 40 above it and about 56 near the top.
#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    /// (floor in dBFS, knob position 0..1), ascending in both
    pub points: Vec<(f64, f64)>,
    /// where a floor at or below the first point is shown
    pub floor_pos: f64,
    /// measured by the user with their own setup (the instrument's noise is in it)
    pub learned: bool,
}

/// Where a floor at or below the first point is shown by default: the middle
/// of the bottom fifth, where the floor does not move with the knob.
pub const FLOOR_POS: f64 = 0.10;

impl Default for Calibration {
    /// Measured on one MiniFuse 2, GAIN 1 held at 7:30, 9:00, 12:00, 3:00 and
    /// 4:30 for 6 s each with nothing plugged in (median of the meter's
    /// peaks, docs/protocol.md §4.2).
    fn default() -> Self {
        Calibration { points: vec![(-92.0, 0.20), (-85.5, 0.50), (-72.3, 0.83), (-63.0, 1.00)], floor_pos: FLOOR_POS, learned: false }
    }
}

/// The knob positions a calibration is measured at: (position, clock face, where the pointer is).
pub const CAL_STEPS: [(f64, &str, &str); 5] = [
    (0.0, "7:30", "fully down (minimum)"),
    (1.0 / 6.0, "9:00", "pointing straight left"),
    (0.5, "12:00", "pointing straight up"),
    (5.0 / 6.0, "3:00", "pointing straight right"),
    (1.0, "4:30", "fully up (maximum)"),
];

/// A calibration needs the floor to rise at least this much from minimum to maximum.
const MIN_CAL_RANGE_DB: f64 = 6.0;
/// Floors this close to the first one are the same flat floor.
const FLAT_DB: f64 = 1.5;

impl Calibration {
    /// Build a calibration from idle floors measured at knob positions
    /// `(position, floor dBFS)`, as the calibration wizard records them.
    pub fn from_measured(measured: &[(f64, f64)]) -> Result<Calibration, String> {
        if measured.len() < 4 {
            return Err("Not enough points were recorded.".into());
        }
        let mut m: Vec<(f64, f64)> = measured.to_vec();
        m.sort_by(|a, b| a.0.total_cmp(&b.0));
        // the floor can only rise with the knob: smooth out measurement noise
        for i in 1..m.len() {
            m[i].1 = m[i].1.max(m[i - 1].1);
        }
        let range = m[m.len() - 1].1 - m[0].1;
        if range < MIN_CAL_RANGE_DB {
            return Err(format!(
                "The level only changed by {range:.1} dB between minimum and maximum, so the knob can't be told apart from noise. Check the input is connected as you use it and the knob was really turned."
            ));
        }
        // a flat start (converter noise masking the preamp): the knee is just past its end
        let flat = (0..m.len()).take_while(|&i| m[i].1 - m[0].1 <= FLAT_DB).count() - 1;
        let (mut points, floor_pos) = if flat >= 1 && flat + 1 < m.len() {
            let knee = m[flat].0 + (m[flat + 1].0 - m[flat].0) * 0.1;
            let mut p = vec![(m[flat].1 + 0.6, knee)];
            p.extend(m[flat + 1..].iter().map(|&(pos, db)| (db, pos)));
            (p, m[..=flat].iter().map(|x| x.0).sum::<f64>() / (flat + 1) as f64)
        } else {
            (m.iter().map(|&(pos, db)| (db, pos)).collect(), m[0].0)
        };
        // strictly ascending floors, so the interpolation is defined
        for i in 1..points.len() {
            if points[i].0 <= points[i - 1].0 {
                points[i].0 = points[i - 1].0 + 0.01;
            }
        }
        Ok(Calibration { points, floor_pos, learned: true })
    }
}

impl Calibration {
    /// Knob position for an idle floor (peak dBFS).
    pub fn position(&self, db: f64) -> f64 {
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if db <= first.0 {
            return self.floor_pos;
        }
        if db >= last.0 {
            return last.1;
        }
        let i = self.points.windows(2).position(|w| db < w[1].0).unwrap_or(0);
        let (a, b) = (self.points[i], self.points[i + 1]);
        a.1 + (b.1 - a.1) * (db - a.0) / (b.0 - a.0)
    }

    /// The floor at a knob position (the inverse of `position`); at or below
    /// the first point, the first point's floor.
    pub fn floor_at(&self, pos: f64) -> f64 {
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if pos <= first.1 {
            return first.0;
        }
        if pos >= last.1 {
            return last.0;
        }
        let i = self.points.windows(2).position(|w| pos < w[1].1).unwrap_or(0);
        let (a, b) = (self.points[i], self.points[i + 1]);
        a.0 + (b.0 - a.0) * (pos - a.1) / (b.1 - a.1)
    }

    /// Floor at maximum gain: anything well above it is a signal, not noise.
    pub fn max_floor_db(&self) -> f64 {
        self.points[self.points.len() - 1].0
    }
}

/// Where MONITOR, MIX and PHONES are drawn: the device can't report them, so
/// they are shown at one fixed position rather than a guessed one.
pub const FIXED_POS: f64 = 0.5;

/// Converter noise floor of an input (peak dBFS), until a lower one is seen:
/// the lowest an idle input reads. It differs per input (one MiniFuse 2:
/// input 1 -92.4, input 2 -95.4), so each estimator learns its own.
pub const ADC_FLOOR_DB: f64 = -92.5;
/// The converter floor is learned from medians over windows this long: the
/// lowest of many short windows lands on their noisy tail, 1-2 dB too low.
const ADC_WINDOW_S: f64 = 6.0;
/// A lower floor must hold this many windows in a row, within ADC_AGREE_DB:
/// switching INST, plugging a jack or restarting the stream dips the meter
/// for a few seconds, which one window alone would take for the floor.
const ADC_WINDOWS: usize = 3;
const ADC_AGREE_DB: f64 = 1.5;
/// Floors below this are not converter noise but digital silence.
const LOWEST_ADC_FLOOR_DB: f64 = -104.0;
/// Digital silence (a meter reading of 0), in dBFS.
const SILENCE_DB: f64 = -120.0;
/// Within this of the converter floor, the gain's share of the floor can't be measured.
const MASKED_DB: f64 = 1.5;

/// Preamp gain above the minimum, in dB, at knob positions: the idle floor
/// with a guitar plugged in (INST on, hands off) read -70.6 / -41.4 / -26.6 /
/// -15.8 dBFS at 7:30 / 12:00 / 3:00 / 4:30. About 55 dB in all. 9:00 is from
/// input 2 with nothing plugged in (its converter is quiet enough to show the
/// preamp there): the preamp's noise rose 12.1 dB from 9:00 to 12:00.
const GAIN_POINTS: [(f64, f64); 5] = [(0.0, 0.0), (1.0 / 6.0, 17.1), (0.5, 29.2), (0.833, 44.0), (1.0, 54.8)];

/// Gain above the minimum (dB) at a knob position.
pub fn gain_db(pos: f64) -> f64 {
    let pos = pos.clamp(0.0, 1.0);
    let i = GAIN_POINTS.windows(2).position(|w| pos <= w[1].0).unwrap_or(GAIN_POINTS.len() - 2);
    let (a, b) = (GAIN_POINTS[i], GAIN_POINTS[i + 1]);
    a.1 + (b.1 - a.1) * (pos - a.0) / (b.0 - a.0)
}

/// Knob position for a gain above the minimum (dB); clamped to the travel.
pub fn pos_for_gain(db: f64) -> f64 {
    let (first, last) = (GAIN_POINTS[0], GAIN_POINTS[GAIN_POINTS.len() - 1]);
    if db <= first.1 {
        return first.0;
    }
    if db >= last.1 {
        return last.0;
    }
    let i = GAIN_POINTS.windows(2).position(|w| db <= w[1].1).unwrap_or(0);
    let (a, b) = (GAIN_POINTS[i], GAIN_POINTS[i + 1]);
    a.0 + (b.0 - a.0) * (db - a.1) / (b.1 - a.1)
}

fn db_to_power(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}

const WINDOW_S: f64 = 1.5;
/// the window must span this long before a reading is trusted
const MIN_SPAN_S: f64 = 0.8;
/// time constant of the smoothing of the estimate
const SMOOTH_S: f64 = 0.6;

/// Spread (90th - 10th percentile of the peaks over the window) up to which
/// an input counts as steady, i.e. noise or hum rather than playing.
const STEADY_SPREAD_DB: f64 = 6.0;

/// Estimates a GAIN knob's position from its input meter.
///
/// Without an anchor the idle floor is compared to the table measured with
/// nothing plugged in (`Calibration`). That is wrong as soon as an instrument
/// adds its own noise or hum, which the preamp amplifies. What stays true is
/// that the floor moves dB for dB with the gain: once the user sets the
/// position (`anchor_to`), the instrument's own level is known and the
/// estimate follows the floor from there, through the measured gain curve.
#[derive(Debug, Clone)]
pub struct GainEstimator {
    cal: Calibration,
    window: VecDeque<(f64, f64)>,
    est: Option<f64>,
    quiet: bool,
    last_t: Option<f64>,
    /// median of the window while it is steady and long enough
    steady_floor: Option<f64>,
    /// the input's own noise referred to minimum gain (dBFS), once anchored
    source_db: Option<f64>,
    /// this input's converter noise floor (dBFS)
    adc_db: f64,
    /// readings since the last converter-floor window began
    adc_window: Vec<f64>,
    adc_since: Option<f64>,
    /// medians of the latest windows (None: not steady)
    adc_recent: VecDeque<Option<f64>>,
    /// while the floor is buried in converter noise: the highest position it can be at
    masked_below: Option<f64>,
    /// the user's correction of a learned calibration (dB taken off the floor before the lookup)
    cal_offset: f64,
    /// where the user put the knob while its floor was buried (shown instead of the middle of the range)
    user_low: Option<f64>,
}

impl GainEstimator {
    pub fn new(cal: Calibration) -> GainEstimator {
        GainEstimator {
            cal,
            window: VecDeque::new(),
            est: None,
            quiet: false,
            last_t: None,
            steady_floor: None,
            source_db: None,
            adc_db: ADC_FLOOR_DB,
            adc_window: Vec::new(),
            adc_since: None,
            adc_recent: VecDeque::new(),
            masked_below: None,
            cal_offset: 0.0,
            user_low: None,
        }
    }

    /// Feed one meter reading (peak dBFS, `-inf` for digital silence) taken at
    /// `t` seconds (any monotonic clock).
    pub fn update(&mut self, t: f64, db: f64) {
        let db = if db.is_finite() { db } else { SILENCE_DB };
        self.learn_converter_floor(t, db);
        self.window.push_back((t, db));
        while self.window.front().is_some_and(|(t0, _)| t - t0 > WINDOW_S) {
            self.window.pop_front();
        }
        let span = t - self.window.front().map_or(t, |(t0, _)| *t0);
        if span < MIN_SPAN_S {
            return;
        }
        let mut v: Vec<f64> = self.window.iter().map(|(_, d)| *d).collect();
        v.sort_by(f64::total_cmp);
        let (median, max) = (v[v.len() / 2], v[v.len() - 1]);
        let steady = v[v.len() * 9 / 10] - v[v.len() / 10] <= STEADY_SPREAD_DB;
        self.steady_floor = steady.then_some(median);
        self.quiet = if self.source_db.is_some() || self.cal.learned {
            // the floor may be anywhere: playing shows as a wide spread
            steady
        } else {
            // nothing plugged in: anything above what noise reaches at maximum gain is a signal
            let hi = self.cal.max_floor_db();
            max <= hi + 6.0 && median <= hi + 3.0
        };
        if self.quiet {
            self.masked_below = self.masked_bound(median);
            if self.masked_below.is_none() {
                self.user_low = None;
            }
            let raw = match (self.masked_below, self.source_db) {
                // where the user put it, else the middle of the range the knob can be in
                (Some(top), _) => self.user_low.filter(|p| *p <= top).unwrap_or(top / 2.0),
                (None, Some(n0)) => pos_from_floor(median, n0, self.adc_db),
                (None, None) => self.cal.position(median - self.cal_offset),
            };
            let dt = self.last_t.map_or(0.0, |l| (t - l).max(0.0));
            self.est = Some(match self.est {
                None => raw,
                Some(e) => e + (raw - e) * (1.0 - (-dt / SMOOTH_S).exp()),
            });
        }
        self.last_t = Some(t);
    }

    /// Collect readings in back-to-back windows; when the last few agree and
    /// are lower than the converter floor known so far, the highest of them
    /// becomes the new floor.
    fn learn_converter_floor(&mut self, t: f64, db: f64) {
        let since = *self.adc_since.get_or_insert(t);
        self.adc_window.push(db);
        if t - since < ADC_WINDOW_S {
            return;
        }
        let mut v = std::mem::take(&mut self.adc_window);
        self.adc_since = Some(t);
        if v.len() < 20 {
            return;
        }
        v.sort_by(f64::total_cmp);
        let median = v[v.len() / 2];
        let steady = v[v.len() * 9 / 10] - v[v.len() / 10] <= STEADY_SPREAD_DB;
        self.adc_recent.push_back((steady && median > LOWEST_ADC_FLOOR_DB).then_some(median));
        if self.adc_recent.len() > ADC_WINDOWS {
            self.adc_recent.pop_front();
        }
        let Some(recent) = self.adc_recent.iter().copied().collect::<Option<Vec<f64>>>() else { return };
        if recent.len() < ADC_WINDOWS {
            return;
        }
        let (lo, hi) = recent.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &m| (lo.min(m), hi.max(m)));
        if hi - lo <= ADC_AGREE_DB && hi < self.adc_db {
            self.adc_db = hi;
        }
    }

    /// While `floor` is no more than converter noise (plus a little), the
    /// highest position the knob can be at; None when the floor shows the gain.
    fn masked_bound(&self, floor: f64) -> Option<f64> {
        if let Some(n0) = self.source_db {
            if floor > self.adc_db + MASKED_DB {
                return None;
            }
            // the most the amplified input noise can be and still hide under the converter's
            let hidden = db_to_power(self.adc_db + MASKED_DB) - db_to_power(self.adc_db);
            return Some(pos_for_gain(10.0 * hidden.log10() - n0));
        }
        let first = self.cal.points[0];
        (self.cal.learned && first.1 > self.cal.floor_pos && floor - self.cal_offset <= first.0).then_some(first.1)
    }

    /// The user says the knob is at `pos` right now. If the input is steady
    /// and above the converter floor, the instrument's own level is learned
    /// from it and the estimate follows the floor from here. Returns false
    /// (nothing learned) while playing, or when the floor is only converter
    /// noise and says nothing about the gain.
    pub fn anchor_to(&mut self, pos: f64) -> bool {
        if self.cal.learned {
            return false; // the user's own calibration already knows this input
        }
        let Some(floor) = self.steady_floor else { return false };
        if floor <= self.adc_db + MASKED_DB {
            return false;
        }
        let above = db_to_power(floor) - db_to_power(self.adc_db);
        let pos = pos.clamp(0.0, 1.0);
        self.source_db = Some(10.0 * above.log10() - gain_db(pos));
        self.est = Some(pos);
        self.masked_below = None;
        self.quiet = true;
        true
    }

    /// The user sets the knob to `pos`. Without an anchor or calibration this
    /// anchors (see `anchor_to`). Otherwise it corrects the estimate: a
    /// calibration is shifted so the current floor reads `pos`, an anchor is
    /// learned again; while playing, `pos` is shown until the input is quiet;
    /// while the floor is buried in converter noise, `pos` is shown if it
    /// lies in the range the knob can be in. Returns whether the estimate
    /// follows the knob from here.
    pub fn set_by_user(&mut self, pos: f64) -> bool {
        let pos = pos.clamp(0.0, 1.0);
        if !self.cal.learned && self.source_db.is_none() {
            return self.anchor_to(pos);
        }
        let Some(floor) = self.steady_floor else {
            self.est = Some(pos);
            self.quiet = false;
            return true;
        };
        if let Some(top) = self.masked_bound(floor) {
            self.user_low = Some(pos.min(top));
            self.est = self.user_low;
            return true;
        }
        if self.cal.learned {
            self.cal_offset = floor - self.cal.floor_at(pos);
            self.est = Some(pos);
            self.masked_below = None;
            self.quiet = true;
            return true;
        }
        self.source_db = None;
        self.anchor_to(pos)
    }

    /// The user's correction of the calibration in use (dB), to save it.
    pub fn cal_offset(&self) -> f64 {
        self.cal_offset
    }

    pub fn set_cal_offset(&mut self, db: f64) {
        self.cal_offset = db;
    }

    /// The input's own level learned by `anchor_to`, to save it.
    pub fn anchor(&self) -> Option<f64> {
        self.source_db
    }

    /// Use an anchor saved earlier; the estimate starts with the next steady reading.
    pub fn restore_anchor(&mut self, source_db: f64) {
        if !self.cal.learned {
            self.source_db = Some(source_db);
        }
    }

    /// This input's converter noise floor, as learned so far.
    pub fn converter_floor(&self) -> f64 {
        self.adc_db
    }

    /// Use a converter floor saved earlier (only ever lowers it).
    pub fn restore_converter_floor(&mut self, db: f64) {
        if db < self.adc_db && db > LOWEST_ADC_FLOOR_DB {
            self.adc_db = db;
        }
    }

    /// While the floor is buried in converter noise, the highest position the
    /// knob can be at: the estimate is "below this", not a value.
    pub fn masked_below(&self) -> Option<f64> {
        self.masked_below
    }

    /// Use another calibration (the user's own, or the default again).
    pub fn set_calibration(&mut self, cal: Calibration) {
        self.cal = cal;
        self.source_db = None;
        self.est = None;
        self.masked_below = None;
        self.cal_offset = 0.0;
        self.user_low = None;
    }

    pub fn calibration(&self) -> &Calibration {
        &self.cal
    }

    /// Is a calibration measured by the user in use?
    pub fn is_learned(&self) -> bool {
        self.cal.learned
    }

    /// Back to the table for an input with nothing plugged in.
    pub fn clear_anchor(&mut self) {
        self.source_db = None;
        self.est = None;
        self.masked_below = None;
    }

    pub fn is_anchored(&self) -> bool {
        self.source_db.is_some()
    }

    /// Last good estimate, 0..1.
    pub fn estimate(&self) -> Option<f64> {
        self.est
    }

    /// Is the latest reading usable (the input is quiet), or is the estimate held?
    pub fn is_live(&self) -> bool {
        self.quiet
    }
}

/// Knob position for a floor, given the input's own level at minimum gain and the converter floor.
fn pos_from_floor(floor_db: f64, source_db: f64, adc_db: f64) -> f64 {
    let above = db_to_power(floor_db) - db_to_power(adc_db);
    if above <= 0.0 {
        return 0.0;
    }
    pos_for_gain(10.0 * above.log10() - source_db)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a constant noise floor for `secs` at 25 Hz.
    fn feed(e: &mut GainEstimator, t0: f64, secs: f64, db: f64) -> f64 {
        let mut t = t0;
        while t < t0 + secs {
            e.update(t, db);
            t += 0.04;
        }
        t
    }

    #[test]
    fn angles_cover_the_sweep_and_invert() {
        assert_eq!(angle_deg(0.0), 135.0);
        assert_eq!(angle_deg(1.0), 405.0);
        assert_eq!(angle_deg(0.5), 270.0); // straight up (12 o'clock)
        assert_eq!(angle_deg(7.0), 405.0, "clamped");
        assert!((pos_from_angle(270.0).unwrap() - 0.5).abs() < 1e-9);
        assert!((pos_from_angle(405.0 - 360.0).unwrap() - 1.0).abs() < 1e-9);
        assert!(pos_from_angle(90.0).is_none(), "6 o'clock is the dead zone");
        assert_eq!(percent(0.617), 62);
    }

    #[test]
    fn the_floor_maps_to_a_position_through_the_measured_points() {
        let cal = Calibration::default();
        // the measured points themselves
        for (db, want) in [(-85.5, 0.50), (-72.3, 0.83), (-63.0, 1.00), (-92.0, FLOOR_POS)] {
            assert!((cal.position(db) - want).abs() < 1e-9, "{db}");
        }
        // below the start of the travel the floor does not move: shown as the bottom fifth
        assert_eq!(cal.position(-95.0), FLOOR_POS);
        assert_eq!(cal.position(-92.4), FLOOR_POS);
        // between points: linear, and never decreasing
        assert!((cal.position(-78.9) - 0.665).abs() < 0.01);
        assert_eq!(cal.position(-50.0), 1.0);
        let mut last = 0.0;
        for i in 0..=400 {
            let p = cal.position(-100.0 + i as f64 * 0.1);
            assert!(p >= last, "monotonic at {i}");
            last = p;
        }
        // and the estimator reports them once it has a moment of data
        for (db, want) in [(-85.5, 0.5), (-72.3, 0.83), (-63.0, 1.0)] {
            let mut e = GainEstimator::new(Calibration::default());
            feed(&mut e, 0.0, 2.0, db);
            assert!((e.estimate().unwrap() - want).abs() < 0.01, "{db}: {:?}", e.estimate());
            assert!(e.is_live());
        }
    }

    /// Medians measured with the knob held at 7:30, 9:00, 12:00, 3:00 and 4:30 land on their positions.
    #[test]
    fn the_calibration_session_reads_back_as_the_clock_positions() {
        let cal = Calibration::default();
        for (db, clock_pos) in [(-92.4, 0.0), (-92.8, 1.0 / 6.0), (-85.5, 0.5), (-72.3, 5.0 / 6.0), (-63.0, 1.0)] {
            let est = cal.position(db);
            let tol = if clock_pos < 0.2 { 0.2 } else { 0.02 };
            assert!((est - clock_pos).abs() <= tol, "{db} dBFS: estimated {est:.2}, knob at {clock_pos:.2}");
        }
    }

    #[test]
    fn it_needs_a_moment_of_data_and_ignores_a_single_spike() {
        let mut e = GainEstimator::new(Calibration::default());
        feed(&mut e, 0.0, 0.5, -80.0);
        assert!(e.estimate().is_none(), "not enough data yet");
        let t = feed(&mut e, 0.5, 2.0, -80.0);
        let before = e.estimate().unwrap();
        e.update(t, -75.0); // one jittery peak among steady readings: the median ignores it
        assert!((e.estimate().unwrap() - before).abs() < 0.03);
    }

    #[test]
    fn a_signal_freezes_the_estimate_until_the_input_is_quiet_again() {
        let mut e = GainEstimator::new(Calibration::default());
        let t = feed(&mut e, 0.0, 2.0, -75.0);
        let held = e.estimate().unwrap();
        let t = feed(&mut e, t, 3.0, -20.0); // a voice, far above the noise
        assert!(!e.is_live());
        assert_eq!(e.estimate(), Some(held), "held, not dragged up by the signal");
        feed(&mut e, t, 3.0, -75.0);
        assert!(e.is_live());
        assert!((e.estimate().unwrap() - held).abs() < 0.02);
    }

    #[test]
    fn turning_the_knob_moves_the_estimate_smoothly() {
        let mut e = GainEstimator::new(Calibration::default());
        let t = feed(&mut e, 0.0, 2.0, -90.0);
        let low = e.estimate().unwrap();
        feed(&mut e, t, 4.0, -70.0);
        let high = e.estimate().unwrap();
        assert!(low < 0.35 && high > 0.8 && high < 0.95, "{low} -> {high}");
    }

    #[test]
    fn digital_silence_reads_as_minimum() {
        let mut e = GainEstimator::new(Calibration::default());
        feed(&mut e, 0.0, 2.0, f64::NEG_INFINITY);
        assert_eq!(e.estimate(), Some(FLOOR_POS));
    }

    #[test]
    fn the_gain_curve_round_trips() {
        for pos in [0.0, 0.17, 0.5, 0.7, 0.833, 0.95, 1.0] {
            assert!((pos_for_gain(gain_db(pos)) - pos).abs() < 1e-9, "{pos}");
        }
        assert_eq!(gain_db(0.0), 0.0);
        assert!((gain_db(1.0) - 54.8).abs() < 1e-9);
        assert_eq!(pos_for_gain(-10.0), 0.0);
        assert_eq!(pos_for_gain(80.0), 1.0);
    }

    /// The idle floors measured with a guitar plugged in (INST on, untouched).
    const GUITAR: [(f64, f64); 4] = [(0.0, -70.6), (0.5, -41.4), (0.833, -26.6), (1.0, -15.8)];

    #[test]
    fn without_an_anchor_a_guitar_makes_the_estimate_jump() {
        // the reported failure: at minimum gain the guitar's floor reads like ~80% of the empty-input table
        let mut e = GainEstimator::new(Calibration::default());
        feed(&mut e, 0.0, 3.0, GUITAR[0].1);
        assert!(e.estimate().unwrap() > 0.75, "{:?}", e.estimate());
    }

    #[test]
    fn setting_the_knob_once_teaches_the_instrument_and_then_the_floor_is_followed() {
        for anchor_at in [0, 1, 2, 3] {
            let mut e = GainEstimator::new(Calibration::default());
            let t = feed(&mut e, 0.0, 3.0, GUITAR[anchor_at].1);
            assert!(e.anchor_to(GUITAR[anchor_at].0), "learned at {anchor_at}");
            assert!(e.is_anchored());
            let mut t = t;
            for (pos, floor) in GUITAR {
                t = feed(&mut e, t, 4.0, floor);
                assert!((e.estimate().unwrap() - pos).abs() < 0.03, "anchored at {anchor_at}: floor {floor} -> {:?}, knob at {pos}", e.estimate());
            }
        }
    }

    #[test]
    fn playing_is_held_and_not_learned() {
        let mut e = GainEstimator::new(Calibration::default());
        let t = feed(&mut e, 0.0, 3.0, -41.4);
        assert!(e.anchor_to(0.5));
        // a played phrase: peaks all over the place
        let mut t = t;
        for i in 0..100 {
            e.update(t, if i % 3 == 0 { -12.0 } else { -45.0 - (i % 7) as f64 * 3.0 });
            t += 0.04;
        }
        assert!(!e.is_live(), "not steady");
        assert!((e.estimate().unwrap() - 0.5).abs() < 0.05, "held at the last good value: {:?}", e.estimate());
        // while the input is not steady the knob can't be learned from either
        let mut f = GainEstimator::new(Calibration::default());
        for i in 0..100 {
            f.update(i as f64 * 0.04, if i % 2 == 0 { -12.0 } else { -50.0 });
        }
        assert!(!f.anchor_to(0.5));
    }

    #[test]
    fn a_floor_that_is_only_converter_noise_cannot_teach_anything() {
        let mut e = GainEstimator::new(Calibration::default());
        feed(&mut e, 0.0, 3.0, -92.8);
        assert!(!e.anchor_to(0.1), "nothing to learn from the converter floor");
        assert!(!e.is_anchored());
    }

    #[test]
    fn clearing_the_anchor_goes_back_to_the_table() {
        let mut e = GainEstimator::new(Calibration::default());
        let t = feed(&mut e, 0.0, 3.0, -41.4);
        assert!(e.anchor_to(0.5));
        e.clear_anchor();
        assert!(!e.is_anchored() && e.estimate().is_none());
        feed(&mut e, t, 3.0, -85.5);
        assert!((e.estimate().unwrap() - 0.5).abs() < 0.03);
    }

    #[test]
    fn each_input_learns_its_own_converter_floor() {
        // input 2 of one MiniFuse 2 idles about 3 dB below input 1
        let mut e = GainEstimator::new(Calibration::default());
        let t = feed(&mut e, 0.0, 19.0, -95.4);
        assert!((e.converter_floor() - -95.4).abs() < 1e-9);
        // digital silence and louder floors don't move it
        let t = feed(&mut e, t, 13.0, f64::NEG_INFINITY);
        feed(&mut e, t, 13.0, -80.0);
        assert!((e.converter_floor() - -95.4).abs() < 1e-9);
        // a saved floor only ever lowers it
        let mut f = GainEstimator::new(Calibration::default());
        f.restore_converter_floor(-90.0);
        assert_eq!(f.converter_floor(), ADC_FLOOR_DB);
        f.restore_converter_floor(-95.4);
        assert_eq!(f.converter_floor(), -95.4);
    }

    #[test]
    fn a_short_dip_is_not_taken_for_the_converter_floor() {
        // switching INST mutes the input for a moment: a few seconds well below the floor
        let mut e = GainEstimator::new(Calibration::default());
        let t = feed(&mut e, 0.0, 12.0, -92.1);
        let t = feed(&mut e, t, 6.0, -97.0);
        feed(&mut e, t, 30.0, -92.1);
        assert_eq!(e.converter_floor(), ADC_FLOOR_DB, "neither the dip nor the higher idle level lowers it");
    }

    #[test]
    fn the_converter_floor_is_taken_out_before_the_gain_is_read() {
        // input 2 (converter -95.4), nothing plugged in: anchored at 12:00, then turned to 3:00
        let mut e = GainEstimator::new(Calibration::default());
        let t = feed(&mut e, 0.0, 19.0, -95.4);
        assert!((e.converter_floor() - -95.4).abs() < 1e-9);
        let t = feed(&mut e, t, 3.0, -85.5);
        assert!(e.anchor_to(0.5));
        // the preamp's noise rises by the gain from 12:00 to 3:00 (14.8 dB)
        let preamp = 10.0 * (db_to_power(-85.5) - db_to_power(-95.4)).log10() + 14.8;
        let floor = 10.0 * (db_to_power(preamp) + db_to_power(-95.4)).log10();
        feed(&mut e, t, 4.0, floor);
        assert!((e.estimate().unwrap() - 0.833).abs() < 0.02, "{:?}", e.estimate());
        assert!(e.masked_below().is_none());
    }

    /// What the wizard records: with nothing plugged in (measured), and with a guitar (measured at
    /// 7:30, 12:00, 3:00 and 4:30; the 9:00 value is made up for the test).
    const EMPTY: [(f64, f64); 5] = [(0.0, -92.4), (1.0 / 6.0, -92.8), (0.5, -85.5), (5.0 / 6.0, -72.3), (1.0, -63.0)];
    const GUITAR5: [(f64, f64); 5] = [(0.0, -70.6), (1.0 / 6.0, -62.0), (0.5, -41.4), (5.0 / 6.0, -26.6), (1.0, -15.8)];

    #[test]
    fn a_calibration_measured_with_nothing_plugged_in_matches_the_built_in_table() {
        let c = Calibration::from_measured(&EMPTY).unwrap();
        assert!(c.learned);
        let d = Calibration::default();
        for db in [-95.0, -92.4, -90.0, -85.5, -80.0, -72.3, -65.0, -63.0, -50.0] {
            assert!((c.position(db) - d.position(db)).abs() < 0.06, "{db}: {} vs {}", c.position(db), d.position(db));
        }
        assert!(c.floor_pos < 0.2, "the flat start is the bottom of the travel");
    }

    #[test]
    fn a_calibration_with_a_guitar_has_no_dead_zone_and_reads_every_position() {
        let c = Calibration::from_measured(&GUITAR5).unwrap();
        for (pos, floor) in GUITAR5 {
            assert!((c.position(floor) - pos).abs() < 0.02, "{pos}: {}", c.position(floor));
        }
        assert_eq!(c.floor_pos, 0.0, "below the minimum floor is the minimum");
        // and the estimator uses it without being told where the knob is
        let mut e = GainEstimator::new(c);
        let mut t = 0.0;
        for (pos, floor) in GUITAR5 {
            t = feed(&mut e, t, 3.0, floor);
            assert!((e.estimate().unwrap() - pos).abs() < 0.03, "{pos}: {:?}", e.estimate());
        }
        assert!(!e.anchor_to(0.3), "a learned calibration is not overridden by a drag");
    }

    #[test]
    fn a_calibration_needs_enough_range_and_points() {
        assert!(Calibration::from_measured(&EMPTY[..3]).is_err());
        let flat = [(0.0, -80.0), (0.25, -79.0), (0.5, -78.0), (0.75, -79.5), (1.0, -80.5)];
        let e = Calibration::from_measured(&flat).unwrap_err();
        assert!(e.contains("only changed by"), "{e}");
    }

    #[test]
    fn measurement_noise_cannot_make_the_floor_fall_with_the_knob() {
        // 3:00 read a little lower than 12:00: it is treated as equal, not as a backwards knob
        let m = [(0.0, -70.0), (0.25, -60.0), (0.5, -41.0), (0.75, -41.4), (1.0, -16.0)];
        let c = Calibration::from_measured(&m).unwrap();
        for w in c.points.windows(2) {
            assert!(w[1].0 > w[0].0 && w[1].1 >= w[0].1, "{:?}", c.points);
        }
        let mut last = 0.0;
        for i in 0..600 {
            let p = c.position(-80.0 + i as f64 * 0.1);
            assert!(p >= last - 1e-9);
            last = p;
        }
    }
}
