//! THE mapping from OpenFuse logical control keys to ALSA control names.
//!
//! This is the only file that knows ALSA control names. The kernel quirk
//! (docs/protocol.md §7.1) is still being written, so if its names change,
//! edit them here and nothing else.
//!
//! Each entry: logical key -> [`AlsaName`] { name, index, iface }.
//!  * `name`   ALSA control element name (`amixer -c N controls` shows them)
//!  * `index`  element index (0 unless the driver uses indexed controls)
//!  * `iface`  [`Iface::Mixer`] (default) or [`Iface::Card`]
//!
//! Value conventions used by the rest of the app:
//!  * switches / mutes  -> bool
//!  * loopback volumes  -> f64 dB. Converted with the control's dB TLV when the
//!    driver provides one, else by [`RAW_VOLUME_FALLBACK`] below.
//!  * meters            -> `Level Meter` is a read-only integer array of raw
//!    peak sample values; converted to dBFS by `levels::raw_to_dbfs`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Iface {
    Card,
    Mixer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlsaName {
    pub name: String,
    pub index: u32,
    pub iface: Iface,
}

impl AlsaName {
    fn mixer(name: impl Into<String>) -> Self {
        AlsaName { name: name.into(), index: 0, iface: Iface::Mixer }
    }
}

/// Spec §7.1 writes "Loopback Ch N ..." with N indexing state[0x06+N], i.e.
/// 0-based. Set to 1 if the kernel numbers channels from 1.
pub const LOOPBACK_CHANNEL_BASE: usize = 0;
/// MiniFuse 2.
pub const LOOPBACK_CHANNELS: usize = 6;

/// How to turn a raw volume integer into dB when the control has no dB TLV.
/// The device itself uses Q8.8 (1/256 dB, spec §4.3).
pub struct RawVolumeFallback {
    pub scale_db_per_step: f64,
    pub offset_db: f64,
}
pub const RAW_VOLUME_FALLBACK: RawVolumeFallback =
    RawVolumeFallback { scale_db_per_step: 1.0 / 256.0, offset_db: 0.0 };

fn build() -> BTreeMap<String, AlsaName> {
    let mut m = BTreeMap::new();
    let mut put = |k: &str, n: AlsaName| {
        m.insert(k.to_string(), n);
    };
    // front-panel buttons (state[0x00] bits, [0x04], [0x05])
    put("in1_inst", AlsaName::mixer("Line In 1 Inst Switch"));
    put("in2_inst", AlsaName::mixer("Line In 2 Inst Switch"));
    put("phantom", AlsaName::mixer("Phantom Power Switch"));
    put("direct_mono", AlsaName::mixer("Direct Monitor Mono Switch"));
    // MiniFuse 1 has DIRECT MONITOR instead of DIRECT MONO (not characterised)
    put("direct_monitor", AlsaName::mixer("Direct Monitor Switch"));
    // loopback mixer (state[0x13..0x16])
    put("loopback", AlsaName::mixer("Loopback Switch"));
    put("loopback_to_usb", AlsaName::mixer("Loopback To USB In Switch"));
    put("loopback_to_monitor", AlsaName::mixer("Loopback To Monitor Switch"));
    for n in 0..LOOPBACK_CHANNELS {
        let c = n + LOOPBACK_CHANNEL_BASE;
        put(&format!("lb_vol_l_{n}"), AlsaName::mixer(format!("Loopback Ch {c} Left Volume")));
        put(&format!("lb_vol_r_{n}"), AlsaName::mixer(format!("Loopback Ch {c} Right Volume")));
        put(&format!("lb_mute_{n}"), AlsaName::mixer(format!("Loopback Ch {c} Mute")));
    }
    // read-only integer array (GET_METERS)
    put("meters", AlsaName::mixer("Level Meter"));
    // read-only integer array: peak of what the driver sends, per playback
    // channel, since the last read (same raw format as the meters)
    put("playback_meters", AlsaName::mixer("Playback Level Meter"));
    m
}

/// Logical key -> ALSA name, for every control OpenFuse knows.
pub fn alsa_controls() -> &'static BTreeMap<String, AlsaName> {
    static MAP: OnceLock<BTreeMap<String, AlsaName>> = OnceLock::new();
    MAP.get_or_init(build)
}

pub fn lookup(key: &str) -> Option<&'static AlsaName> {
    alsa_controls().get(key)
}
