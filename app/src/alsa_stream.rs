//! Who is playing to the card, and with what settings.
//!
//! An app that opens the card's ALSA device (`hw:`) directly picks its own
//! sample rate and buffer size, and its audio never passes through the sound
//! server. OpenFuse can neither change those settings nor meter that stream,
//! but /proc/asound shows what was opened, so the UI can say so instead of
//! offering controls that would not apply.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub owner: String,
    /// the sound server (PipeWire / PulseAudio) owns the device: its clock
    /// settings apply and its sink can be metered
    pub via_server: bool,
    pub rate: u32,
    pub period: u32,
    pub buffer: u32,
    pub channels: u32,
}

impl StreamInfo {
    pub fn period_ms(&self) -> f64 {
        crate::audioclock::latency_ms(self.period, self.rate)
    }

    /// "48 kHz · 64-frame period (1.3 ms) · 256-frame buffer"
    pub fn summary(&self) -> String {
        format!(
            "{} · {}-frame period ({:.1} ms) · {}-frame buffer",
            crate::audioclock::format_rate(self.rate),
            self.period,
            self.period_ms(),
            self.buffer
        )
    }
}

const SERVERS: [&str; 4] = ["pipewire", "wireplumber", "pipewire-pulse", "pulseaudio"];

fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

fn number(text: &str, key: &str) -> u32 {
    field(text, key).and_then(|v| v.split_whitespace().next()?.parse().ok()).unwrap_or(0)
}

/// `status` and `hw_params` are the contents of the stream's /proc files;
/// `owner` is the owning process' name.
pub fn parse(status: &str, hw_params: &str, owner: String) -> Option<StreamInfo> {
    if status.trim() == "closed" || hw_params.trim() == "closed" {
        return None;
    }
    let rate = number(hw_params, "rate");
    (rate > 0).then(|| StreamInfo {
        via_server: SERVERS.contains(&owner.as_str()),
        owner,
        rate,
        period: number(hw_params, "period_size"),
        buffer: number(hw_params, "buffer_size"),
        channels: number(hw_params, "channels"),
    })
}

fn owner_name(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("process {pid}"))
}

/// The open playback stream of card `card`, if any.
pub fn playback_stream(proc_root: &Path, card: u32) -> Option<StreamInfo> {
    let dir = proc_root.join(format!("card{card}"));
    let mut pcms: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("pcm") && e.file_name().to_string_lossy().ends_with('p')).collect();
    pcms.sort_by_key(|e| e.file_name());
    pcms.into_iter().find_map(|pcm| {
        let sub = pcm.path().join("sub0");
        let status = std::fs::read_to_string(sub.join("status")).ok()?;
        let hw = std::fs::read_to_string(sub.join("hw_params")).ok()?;
        let owner = owner_name(number(&status, "owner_pid"));
        parse(&status, &hw, owner)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "state: RUNNING\nowner_pid   : 29930\ntrigger_time: 2645.29\ndelay       : 207\n";
    const HW: &str = "access: RW_INTERLEAVED\nformat: S32_LE\nsubformat: STD\nchannels: 4\nrate: 48000 (48000/1)\nperiod_size: 64\nbuffer_size: 256\n";

    #[test]
    fn parses_an_open_stream() {
        let s = parse(STATUS, HW, "Lithium Studio".into()).unwrap();
        assert_eq!((s.rate, s.period, s.buffer, s.channels), (48_000, 64, 256, 4));
        assert!(!s.via_server);
        assert_eq!(s.summary(), "48 kHz · 64-frame period (1.3 ms) · 256-frame buffer");
    }

    #[test]
    fn server_owned_and_closed_streams() {
        assert!(parse(STATUS, HW, "pipewire".into()).unwrap().via_server);
        assert!(parse("closed\n", "closed\n", "x".into()).is_none());
    }

    #[test]
    fn reads_a_proc_tree() {
        let t = tempfile::tempdir().unwrap();
        let sub = t.path().join("card3/pcm0p/sub0");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("status"), "closed\n").unwrap();
        std::fs::write(sub.join("hw_params"), "closed\n").unwrap();
        assert!(playback_stream(t.path(), 3).is_none());
        std::fs::write(sub.join("status"), STATUS.replace("29930", &std::process::id().to_string())).unwrap();
        std::fs::write(sub.join("hw_params"), HW).unwrap();
        let s = playback_stream(t.path(), 3).unwrap();
        assert_eq!(s.period, 64);
        assert!(!s.owner.is_empty() && !s.via_server);
        assert!(playback_stream(t.path(), 9).is_none());
    }
}
