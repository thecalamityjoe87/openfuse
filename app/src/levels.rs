//! Meter value conversion: the one place that knows the raw meter format.
//!
//! Spec §5: each meter is the peak absolute sample value as a 32-bit audio
//! sample (24-bit data, low byte 00), full scale ~ 2^31 (exact value still to
//! confirm, Q5). 0 means silence. `dBFS = 20 * log10(value / 2^31)`.
//!
//! Backends hand the UI dBFS floats (`-inf` for silence); the UI clamps for
//! display and maps dBFS to the front-panel LED ladder (`MeterSpec`).

pub const RAW_FULL_SCALE: f64 = 2147483648.0; // 2^31

/// A meter reading at or above this level counts as clipping. UNVERIFIED:
/// the exact full-scale value is still to be confirmed on hardware (spec §5,
/// Q5); this is the one place to change it.
pub const CLIP_DBFS: f64 = -0.1;

/// How long a clip indicator stays lit after the last clipping reading.
pub const CLIP_LATCH: std::time::Duration = std::time::Duration::from_secs(2);

/// Does this reading (dBFS, from `raw_to_dbfs`) clip?
pub fn is_clip(db: f64) -> bool {
    db >= CLIP_DBFS
}
pub const SILENCE: f64 = f64::NEG_INFINITY;

/// Raw meter word -> dBFS. ALSA integers are signed longs, so only the low
/// 32 bits are used.
pub fn raw_to_dbfs(raw: i64) -> f64 {
    let v = (raw as u64) & 0xFFFF_FFFF;
    if v == 0 {
        return SILENCE;
    }
    20.0 * (v as f64 / RAW_FULL_SCALE).log10()
}

/// Inverse, for the simulator: dBFS -> raw u32 (24-bit data, low byte 00).
pub fn dbfs_to_raw(db: f64) -> u32 {
    if db == SILENCE || db < -200.0 || db.is_nan() {
        return 0;
    }
    let v = (RAW_FULL_SCALE * 10f64.powf(db.min(0.0) / 20.0)).round();
    let v = v.min(f64::from(0xFFFF_FF00u32)) as u64;
    (v as u32) & 0xFFFF_FF00
}

/// "−12.0 dB", "< −60 dB", "−∞ dB" (typographic minus).
pub fn format_dbfs(db: f64, floor: f64) -> String {
    if db == SILENCE {
        return "−∞ dB".into();
    }
    if db < floor {
        return format!("< {floor:.0} dB").replace('-', "−");
    }
    format!("{db:.1} dB").replace('-', "−")
}

/// Display ballistics: instant rise, steady fall, and a peak marker that is
/// held for a moment, so meters read smoothly instead of flickering with
/// every raw peak value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ballistics {
    pub level: f64,
    pub peak: f64,
    hold: f64,
}

/// dB per second
pub const FALL_RATE: f64 = 24.0;
/// seconds the peak marker stays put
pub const PEAK_HOLD: f64 = 1.0;
const BOTTOM: f64 = -150.0;

impl Default for Ballistics {
    fn default() -> Self {
        Ballistics { level: BOTTOM, peak: BOTTOM, hold: 0.0 }
    }
}

impl Ballistics {
    /// Feed the latest reading after `dt` seconds.
    pub fn update(&mut self, db: f64, dt: f64) {
        let db = if db.is_finite() { db.max(BOTTOM) } else { BOTTOM };
        let dt = dt.clamp(0.0, 10.0);
        self.level = db.max(self.level - FALL_RATE * dt);
        if db >= self.peak {
            self.peak = db;
            self.hold = PEAK_HOLD;
        } else if self.hold > dt {
            self.hold -= dt;
        } else {
            let fall = dt - self.hold.max(0.0);
            self.hold = 0.0;
            self.peak = (self.peak - FALL_RATE * fall).max(self.level);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_conversion() {
        assert_eq!(raw_to_dbfs(0), SILENCE);
        assert!((raw_to_dbfs(1 << 31) - 0.0).abs() < 1e-9);
        // spec §5 example: 0x1A6900 is about -62 dBFS
        assert!((raw_to_dbfs(0x1A6900) - -61.9).abs() < 0.1);
        // ALSA hands out signed longs: -1 is 0xFFFFFFFF
        assert_eq!(raw_to_dbfs(-1), raw_to_dbfs(0xFFFF_FFFF));
        assert!((raw_to_dbfs(dbfs_to_raw(-12.0) as i64) - -12.0).abs() < 0.01);
        assert_eq!(dbfs_to_raw(-30.0) & 0xFF, 0); // 24-bit data
        assert_eq!(dbfs_to_raw(SILENCE), 0);
        // idle noise from the spec is around -97..-91 dBFS
        let idle = raw_to_dbfs(0xF300);
        assert!(idle < -90.0 && idle > -100.0);
    }

    #[test]
    fn ballistics_rise_fast_fall_slow_and_hold_peaks() {
        let mut b = Ballistics::default();
        b.update(-6.0, 0.04);
        assert_eq!((b.level, b.peak), (-6.0, -6.0));
        b.update(-60.0, 0.5); // signal stops: falls at FALL_RATE, peak held
        assert!((b.level - (-6.0 - FALL_RATE * 0.5)).abs() < 1e-9);
        assert_eq!(b.peak, -6.0);
        b.update(-60.0, 0.75); // hold expired 0.25 s ago
        assert!((b.peak - (-6.0 - FALL_RATE * 0.25)).abs() < 1e-9);
        b.update(SILENCE, 10.0);
        assert!(b.level < -100.0 && b.peak < -100.0);
        assert!(b.peak >= b.level);
    }

    #[test]
    fn clip_threshold() {
        assert!(is_clip(raw_to_dbfs(0x8000_0000))); // full scale
        assert!(is_clip(raw_to_dbfs(0x7FFF_FF00))); // largest 24-bit sample
        assert!(is_clip(raw_to_dbfs(dbfs_to_raw(-0.05) as i64)));
        assert!(!is_clip(raw_to_dbfs(dbfs_to_raw(-0.2) as i64)));
        assert!(!is_clip(raw_to_dbfs(0)));
        assert!(!is_clip(SILENCE));
    }

    #[test]
    fn formatting() {
        assert_eq!(format_dbfs(SILENCE, -60.0), "−∞ dB");
        assert_eq!(format_dbfs(-90.0, -60.0), "< −60 dB");
        assert_eq!(format_dbfs(-9.04, -60.0), "−9.0 dB");
    }
}
