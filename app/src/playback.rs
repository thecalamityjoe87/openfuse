//! Host-side output metering.
//!
//! The MiniFuse does not meter playback (docs/protocol.md §5), so the output
//! level is measured here: the monitor of the card's PipeWire / PulseAudio
//! sink is captured with `pw-record` (or `parec`) as raw 32-bit float, and
//! the peak of every channel since the last poll is kept, like the device's
//! own input meters. The capture only runs while the window is visible.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHANNELS: usize = 2;
const FRAME_BYTES: usize = CHANNELS * 4;
const RETRY: Duration = Duration::from_secs(2);
const GAIN_REFRESH: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Server {
    PipeWire,
    Pulse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sink {
    server: Server,
    /// node / sink name
    pub name: String,
    /// PipeWire object id (what `wpctl` takes)
    id: Option<u32>,
}

/// Sinks of ALSA card `card` from `pw-dump` output. The properties of a node
/// are printed sorted, so `alsa.card` comes before `media.class` and
/// `node.name`; a node's block ends where the next object's "type" begins.
fn pipewire_sink(dump: &str, card: u32) -> Option<Sink> {
    let (mut cur_card, mut is_sink, mut name) = (None::<u32>, false, None::<String>);
    let (mut prev_id, mut cur_id) = (None::<u32>, None::<u32>);
    let mut found = None;
    let mut check = |cur_card: Option<u32>, is_sink: bool, name: &mut Option<String>, id: Option<u32>| {
        if found.is_none() && is_sink && cur_card == Some(card) {
            found = name.take().map(|n| (n, id));
        }
    };
    for line in dump.lines() {
        let line = line.trim();
        if line.starts_with("\"type\":") {
            check(cur_card, is_sink, &mut name, cur_id);
            (cur_card, is_sink, name, cur_id) = (None, false, None, prev_id);
        } else if let Some(v) = prop(line, "alsa.card") {
            cur_card = v.parse().ok();
        } else if let Some(v) = prop(line, "media.class") {
            is_sink = v == "Audio/Sink";
        } else if let Some(v) = prop(line, "node.name") {
            name = Some(v.to_string());
        }
        // a top-level object's id is the line right before its "type"
        prev_id = prop(line, "id").and_then(|v| v.parse().ok());
    }
    check(cur_card, is_sink, &mut name, cur_id);
    found.map(|(name, id)| Sink { server: Server::PipeWire, name, id })
}

/// `"key": value,` -> value without quotes / comma.
fn prop<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix('"')?.strip_prefix(key)?.strip_prefix("\":")?;
    Some(rest.trim().trim_end_matches(',').trim_matches('"'))
}

/// Sinks of ALSA card `card` from `pactl list sinks` output.
fn pulse_sink(list: &str, card: u32) -> Option<Sink> {
    let (mut name, mut found) = (None::<String>, None);
    for line in list.lines().map(str::trim) {
        if let Some(n) = line.strip_prefix("Name:") {
            name = Some(n.trim().to_string());
        } else if let Some(v) = line.strip_prefix("alsa.card =")
            && v.trim().trim_matches('"').parse() == Ok(card)
            && found.is_none()
        {
            found = name.clone();
        }
    }
    found.map(|name| Sink { server: Server::Pulse, name, id: None })
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn find_sink(card: u32) -> Option<Sink> {
    run("pw-dump", &[])
        .and_then(|d| pipewire_sink(&d, card))
        .or_else(|| run("pactl", &["list", "sinks"]).and_then(|l| pulse_sink(&l, card)))
}

/// Linear gain the sound server applies to the sink before it reaches the
/// device (the MiniFuse has no hardware PCM volume, so the system volume is
/// a software gain). The sink monitor is taken before it, so the meter has to
/// apply it itself to show what actually leaves the interface.
fn sink_gain(sink: &Sink) -> Option<f32> {
    match (sink.server, sink.id) {
        (Server::PipeWire, Some(id)) => parse_wpctl_volume(&run("wpctl", &["get-volume", &id.to_string()])?),
        (Server::Pulse, _) => {
            let vol = parse_pactl_volume(&run("pactl", &["get-sink-volume", &sink.name])?)?;
            let muted = run("pactl", &["get-sink-mute", &sink.name]).is_some_and(|m| m.contains("yes"));
            Some(if muted { 0.0 } else { vol })
        }
        _ => None,
    }
}

/// "Volume: 0.50" / "Volume: 0.50 [MUTED]". wpctl shows the cube root of the
/// linear gain (0.5 is a gain of 0.125).
fn parse_wpctl_volume(out: &str) -> Option<f32> {
    let rest = out.trim().strip_prefix("Volume:")?;
    if rest.contains("[MUTED]") {
        return Some(0.0);
    }
    let v: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some(v.max(0.0).powi(3))
}

/// First channel's dB value of "Volume: front-left: 32768 /  50% / -18.06 dB, ...".
fn parse_pactl_volume(out: &str) -> Option<f32> {
    let db: f32 = out.split('/').nth(2)?.split_whitespace().next()?.trim_end_matches(',').parse().ok()?;
    Some(10f32.powf(db / 20.0))
}

fn spawn_capture(sink: &Sink) -> std::io::Result<Child> {
    let mut cmd = match sink.server {
        Server::PipeWire => {
            let mut c = Command::new("pw-record");
            c.args(["--target", &sink.name, "-P", "stream.capture.sink=true", "--latency", "20ms"])
                .args(["--format", "f32", "--rate", "48000", "--channels", "2", "--raw", "-"]);
            c
        }
        Server::Pulse => {
            let mut c = Command::new("parec");
            c.args(["-d", &format!("{}.monitor", sink.name), "--raw", "--format=float32le"])
                .args(["--rate=48000", "--channels=2", "--latency-msec=20"]);
            c
        }
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
}

/// Peak per channel (linear, as f32 bits) since the last `take`. Non-negative
/// floats order like their bit patterns, so `fetch_max` works on the bits.
#[derive(Default)]
struct Peaks([AtomicU32; CHANNELS]);

impl Peaks {
    fn feed(&self, bytes: &[u8]) {
        let mut peak = [0f32; CHANNELS];
        for frame in bytes.chunks_exact(FRAME_BYTES) {
            for (c, p) in peak.iter_mut().enumerate() {
                let s = f32::from_le_bytes(frame[c * 4..c * 4 + 4].try_into().unwrap());
                if s.is_finite() {
                    *p = p.max(s.abs());
                }
            }
        }
        for (a, p) in self.0.iter().zip(peak) {
            a.fetch_max(p.to_bits(), Ordering::Relaxed);
        }
    }

    fn take(&self) -> [f32; CHANNELS] {
        [0, 1].map(|c| f32::from_bits(self.0[c].swap(0, Ordering::Relaxed)))
    }
}

struct Capture {
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    peaks: Arc<Peaks>,
    /// linear sink gain as f32 bits
    gain: Arc<AtomicU32>,
    failed: Arc<AtomicBool>,
}

impl Capture {
    fn start(card: u32) -> Capture {
        let cap = Capture {
            stop: Arc::default(),
            child: Arc::default(),
            peaks: Arc::default(),
            gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            failed: Arc::default(),
        };
        let (stop, slot, peaks, failed) = (cap.stop.clone(), cap.child.clone(), cap.peaks.clone(), cap.failed.clone());
        let gain = cap.gain.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Some(sink) = find_sink(card) else {
                    std::thread::sleep(RETRY);
                    continue;
                };
                let mut child = match spawn_capture(&sink) {
                    Ok(c) => c,
                    Err(_) => {
                        failed.store(true, Ordering::Relaxed);
                        return;
                    }
                };
                let Some(mut out) = child.stdout.take() else { return };
                *slot.lock().unwrap() = Some(child);
                // stop() may have run between the check above and the store
                if stop.load(Ordering::Relaxed) {
                    kill(&slot);
                    return;
                }
                let (mut buf, mut pending) = ([0u8; 4096], Vec::<u8>::new());
                let mut last_gain = None::<Instant>;
                while let Ok(n) = out.read(&mut buf) {
                    if last_gain.is_none_or(|t| t.elapsed() >= GAIN_REFRESH) {
                        last_gain = Some(Instant::now());
                        if let Some(g) = sink_gain(&sink) {
                            gain.store(g.to_bits(), Ordering::Relaxed);
                        }
                    }
                    if n == 0 {
                        break;
                    }
                    pending.extend_from_slice(&buf[..n]);
                    let whole = pending.len() / FRAME_BYTES * FRAME_BYTES;
                    peaks.feed(&pending[..whole]);
                    pending.drain(..whole);
                }
                kill(&slot);
                if !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(RETRY); // sink vanished or server restarted
                }
            }
        });
        cap
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        kill(&self.child);
    }
}

fn kill(slot: &Mutex<Option<Child>>) {
    if let Some(mut c) = slot.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Peak output level of one card's playback stream, in dBFS per channel.
pub struct PlaybackMeter {
    card: u32,
    capture: Option<Capture>,
    unavailable_reported: bool,
}

impl PlaybackMeter {
    pub fn new(card: u32) -> PlaybackMeter {
        PlaybackMeter { card, capture: None, unavailable_reported: false }
    }

    pub fn set_running(&mut self, on: bool) {
        match (on, self.capture.is_some()) {
            (true, false) => self.capture = Some(Capture::start(self.card)),
            (false, true) => {
                if let Some(c) = self.capture.take() {
                    c.stop();
                }
            }
            _ => {}
        }
    }

    /// Peak dBFS per channel since the previous call (silence when idle).
    pub fn take(&mut self) -> [f64; CHANNELS] {
        match &self.capture {
            Some(c) => {
                let gain = f32::from_bits(c.gain.load(Ordering::Relaxed));
                c.peaks.take().map(|p| crate::levels::raw_to_dbfs(linear_to_raw(p * gain)))
            }
            None => [crate::levels::SILENCE; CHANNELS],
        }
    }

    /// True once if neither pw-record nor parec could be started.
    pub fn take_unavailable(&mut self) -> bool {
        let failed = self.capture.as_ref().is_some_and(|c| c.failed.load(Ordering::Relaxed));
        failed && !std::mem::replace(&mut self.unavailable_reported, true)
    }
}

impl Drop for PlaybackMeter {
    fn drop(&mut self) {
        self.set_running(false);
    }
}

fn linear_to_raw(p: f32) -> i64 {
    (f64::from(p.min(1.0)) * crate::levels::RAW_FULL_SCALE).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_sink_of_a_card_in_pw_dump() {
        let dump = r#"[
  {
    "id": 40,
    "type": "PipeWire:Interface:Node",
    "info": { "props": {
        "alsa.card": 1,
        "media.class": "Audio/Source",
        "node.name": "alsa_input.other"
    } }
  },
  {
    "id": 41,
    "type": "PipeWire:Interface:Node",
    "info": { "props": {
        "alsa.card": 3,
        "media.class": "Audio/Sink",
        "node.name": "alsa_output.usb-Arturia_MiniFuse_2-00.analog-stereo"
    } }
  }
]"#;
        let s = pipewire_sink(dump, 3).unwrap();
        assert_eq!(s.name, "alsa_output.usb-Arturia_MiniFuse_2-00.analog-stereo");
        assert_eq!(s.id, Some(41));
        assert!(pipewire_sink(dump, 1).is_none(), "a source is not a sink");
        assert!(pipewire_sink(dump, 9).is_none());
    }

    #[test]
    fn finds_the_sink_of_a_card_in_pactl_output() {
        let list = "Sink #1\n\tName: alsa_output.pci-0000\n\tProperties:\n\t\talsa.card = \"0\"\n\
                    Sink #2\n\tName: alsa_output.usb-Arturia\n\tProperties:\n\t\talsa.card = \"3\"\n";
        assert_eq!(pulse_sink(list, 3).unwrap().name, "alsa_output.usb-Arturia");
        assert!(pulse_sink(list, 7).is_none());
    }

    #[test]
    fn sink_volume_parses_to_linear_gain() {
        assert_eq!(parse_wpctl_volume("Volume: 1.00\n"), Some(1.0));
        assert!((parse_wpctl_volume("Volume: 0.50\n").unwrap() - 0.125).abs() < 1e-6);
        assert_eq!(parse_wpctl_volume("Volume: 0.77 [MUTED]\n"), Some(0.0));
        assert_eq!(parse_wpctl_volume("garbage"), None);
        let pa = "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB\n";
        assert!((parse_pactl_volume(pa).unwrap() - 0.125).abs() < 1e-3);
    }

    #[test]
    fn peaks_are_kept_per_channel_until_taken() {
        let p = Peaks::default();
        let mut bytes = Vec::new();
        for (l, r) in [(0.1f32, -0.5f32), (-0.25, 0.0), (0.0, 0.0)] {
            bytes.extend(l.to_le_bytes());
            bytes.extend(r.to_le_bytes());
        }
        p.feed(&bytes);
        p.feed(&[]);
        assert_eq!(p.take(), [0.25, 0.5]);
        assert_eq!(p.take(), [0.0, 0.0]);
    }

    #[test]
    fn linear_peaks_convert_to_dbfs() {
        assert!((crate::levels::raw_to_dbfs(linear_to_raw(0.5)) - -6.02).abs() < 0.01);
        assert!(crate::levels::raw_to_dbfs(linear_to_raw(1.0)) > -0.01);
        assert_eq!(crate::levels::raw_to_dbfs(linear_to_raw(0.0)), crate::levels::SILENCE);
    }
}
