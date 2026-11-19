//! Sample rate and buffer size (PipeWire's "quantum") of the audio graph.
//!
//! The MiniFuse is a class-compliant USB audio device, so on Linux the rate
//! and buffer size are chosen by the audio server, not by a vendor control
//! (the Windows ASIO driver's panel has no counterpart). DAWs and plugins
//! that run through PipeWire / JACK follow the server clock; hosts that open
//! `hw:` directly ignore it.
//!
//! Changes are made at run time through the server's settings metadata
//! (`pw-metadata -n settings 0 clock.force-rate|clock.force-quantum`). Nothing
//! is written to disk: PipeWire forgets them when it restarts. The clock is
//! global to the graph, not per device.

use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The rates the MiniFuse supports (UAC2 clock source).
pub const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
/// Buffer sizes offered, in frames.
pub const QUANTA: [u32; 7] = [32, 64, 128, 256, 512, 1024, 2048];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// default rate of the graph
    pub rate: u32,
    /// default buffer size of the graph
    pub quantum: u32,
    /// 0 = not forced
    pub force_rate: u32,
    /// 0 = not forced
    pub force_quantum: u32,
    pub min_quantum: u32,
    pub max_quantum: u32,
}

impl Default for Clock {
    fn default() -> Self {
        Clock { rate: 48_000, quantum: 1024, force_rate: 0, force_quantum: 0, min_quantum: 32, max_quantum: 2048 }
    }
}

impl Clock {
    pub fn effective_rate(&self) -> u32 {
        if self.force_rate > 0 { self.force_rate } else { self.rate }
    }

    pub fn effective_quantum(&self) -> u32 {
        if self.force_quantum > 0 { self.force_quantum } else { self.quantum }
    }

    /// One buffer, in milliseconds.
    pub fn latency_ms(&self) -> f64 {
        latency_ms(self.effective_quantum(), self.effective_rate())
    }

    pub fn forced(&self) -> bool {
        self.force_rate > 0 || self.force_quantum > 0
    }
}

pub fn latency_ms(quantum: u32, rate: u32) -> f64 {
    if rate == 0 { 0.0 } else { f64::from(quantum) * 1000.0 / f64::from(rate) }
}

/// "44.1 kHz", "48 kHz"
pub fn format_rate(rate: u32) -> String {
    let k = f64::from(rate) / 1000.0;
    if rate % 1000 == 0 { format!("{k:.0} kHz") } else { format!("{k:.1} kHz") }
}

/// Parse the output of `pw-metadata -n settings`.
pub fn parse_metadata(out: &str) -> Option<Clock> {
    let mut c = Clock::default();
    let mut seen = false;
    for line in out.lines() {
        let Some(key) = between(line, "key:'", "'") else { continue };
        let Some(value) = between(line, "value:'", "'") else { continue };
        let n: u32 = value.trim().parse().unwrap_or(0);
        seen = true;
        match key {
            "clock.rate" if n > 0 => c.rate = n,
            "clock.quantum" if n > 0 => c.quantum = n,
            "clock.force-rate" => c.force_rate = n,
            "clock.force-quantum" => c.force_quantum = n,
            "clock.min-quantum" if n > 0 => c.min_quantum = n,
            "clock.max-quantum" if n > 0 => c.max_quantum = n,
            _ => {}
        }
    }
    seen.then_some(c)
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = s.find(start)? + start.len();
    let len = s[from..].find(end)?;
    Some(&s[from..from + len])
}

pub trait ClockControl {
    fn read(&mut self) -> Result<Clock, String>;
    /// 0 releases the forced rate
    fn set_rate(&mut self, rate: u32) -> Result<(), String>;
    /// 0 releases the forced buffer size
    fn set_quantum(&mut self, quantum: u32) -> Result<(), String>;
}

/// The running PipeWire server.
pub struct PipeWire;

fn pw_metadata(args: &[&str]) -> Result<String, String> {
    let out = Command::new("pw-metadata")
        .arg("-n")
        .arg("settings")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("pw-metadata is needed (PipeWire): {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("pw-metadata failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

impl ClockControl for PipeWire {
    fn read(&mut self) -> Result<Clock, String> {
        parse_metadata(&pw_metadata(&[])?).ok_or_else(|| "PipeWire reported no clock settings".to_string())
    }

    fn set_rate(&mut self, rate: u32) -> Result<(), String> {
        pw_metadata(&["0", "clock.force-rate", &rate.to_string()]).map(|_| ())
    }

    fn set_quantum(&mut self, quantum: u32) -> Result<(), String> {
        pw_metadata(&["0", "clock.force-quantum", &quantum.to_string()]).map(|_| ())
    }
}

/// In-memory clock for the simulator and tests: never touches the real server.
#[derive(Default)]
pub struct Memory(pub Clock);

impl ClockControl for Memory {
    fn read(&mut self) -> Result<Clock, String> {
        Ok(self.0)
    }

    fn set_rate(&mut self, rate: u32) -> Result<(), String> {
        self.0.force_rate = rate;
        Ok(())
    }

    fn set_quantum(&mut self, quantum: u32) -> Result<(), String> {
        self.0.force_quantum = quantum;
        Ok(())
    }
}

/// How often the main page re-reads the server clock.
const POLL_EVERY: Duration = Duration::from_secs(2);
/// While the window is hidden only the tray shows the clock: check it less often.
pub const POLL_HIDDEN: Duration = Duration::from_secs(10);

type Reading = Result<Clock, String>;

/// What the panel and the main page show, plus the control behind it.
pub struct AudioClock {
    ctl: Arc<Mutex<Box<dyn ClockControl + Send>>>,
    /// None until the first successful read
    pub clock: Option<Clock>,
    pub error: Option<String>,
    /// a background read in flight (reading spawns a process, so it must not
    /// block the UI thread)
    pending: Option<Receiver<Reading>>,
    last_poll: Option<Instant>,
}

impl AudioClock {
    pub fn new(ctl: Box<dyn ClockControl + Send>) -> AudioClock {
        AudioClock { ctl: Arc::new(Mutex::new(ctl)), clock: None, error: None, pending: None, last_poll: None }
    }

    fn store(&mut self, r: Reading) {
        match r {
            Ok(c) => (self.clock, self.error) = (Some(c), None),
            Err(e) => (self.clock, self.error) = (None, Some(e)),
        }
    }

    pub fn refresh(&mut self) {
        self.pending = None;
        let r = self.ctl.lock().map_err(|_| "audio clock unavailable".to_string()).and_then(|mut c| c.read());
        self.store(r);
        self.last_poll = Some(Instant::now());
    }

    /// Called from the UI tick: pick up a finished background read and start
    /// the next one when it is due.
    pub fn poll(&mut self) {
        self.poll_every(POLL_EVERY);
    }

    /// [`poll`](Self::poll) with a custom interval between reads.
    pub fn poll_every(&mut self, every: Duration) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(r) => {
                    self.pending = None;
                    self.store(r);
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => self.pending = None,
            }
        }
        if self.last_poll.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        self.last_poll = Some(Instant::now());
        let (tx, rx) = channel();
        let ctl = self.ctl.clone();
        std::thread::spawn(move || {
            let r = ctl.lock().map_err(|_| "audio clock unavailable".to_string()).and_then(|mut c| c.read());
            let _ = tx.send(r);
        });
        self.pending = Some(rx);
    }

    pub fn set_rate(&mut self, rate: u32) {
        self.apply(|c| c.set_rate(rate));
    }

    pub fn set_quantum(&mut self, quantum: u32) {
        self.apply(|c| c.set_quantum(quantum));
    }

    /// Back to the server's own rate and buffer size.
    pub fn reset(&mut self) {
        self.apply(|c| c.set_rate(0).and_then(|_| c.set_quantum(0)));
    }

    fn apply(&mut self, f: impl FnOnce(&mut dyn ClockControl) -> Result<(), String>) {
        let r = match self.ctl.lock() {
            Ok(mut c) => f(c.as_mut()),
            Err(_) => Err("audio clock unavailable".to_string()),
        };
        if let Err(e) = r {
            self.error = Some(e);
            return;
        }
        self.refresh(); // read back: the server is the source of truth
    }
}

/// One line for the main page: what the audio is running at right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// "Lithium Studio" (an app on the card directly) or "PipeWire"
    pub source: String,
    /// "48 kHz · 64 frames · 1.3 ms"
    pub text: String,
    /// set by OpenFuse (forced) rather than left to the server / the app
    pub forced: bool,
    /// an app is playing right now
    pub live: bool,
}

/// An app that opened the card itself decides its own settings, so what it
/// opened is what is real; otherwise the sound server's clock is.
pub fn summary(clock: Option<&Clock>, stream: Option<&crate::alsa_stream::StreamInfo>) -> Option<Summary> {
    let line = |rate: u32, frames: u32| {
        format!("{} · {} frames · {:.1} ms", format_rate(rate), frames, latency_ms(frames, rate))
    };
    if let Some(s) = stream.filter(|s| !s.via_server) {
        return Some(Summary { source: s.owner.clone(), text: line(s.rate, s.period), forced: false, live: true });
    }
    let c = clock?;
    Some(Summary {
        source: "PipeWire".into(),
        text: line(c.effective_rate(), c.effective_quantum()),
        forced: c.forced(),
        live: stream.is_some(),
    })
}

#[cfg(test)]
mod summary_tests {
    use super::*;
    use crate::alsa_stream::StreamInfo;

    fn stream(via_server: bool) -> StreamInfo {
        StreamInfo { owner: "Lithium Studio".into(), via_server, rate: 48_000, period: 64, buffer: 256, channels: 4 }
    }

    #[test]
    fn a_direct_app_wins_over_the_server_clock() {
        let c = Clock::default();
        let s = summary(Some(&c), Some(&stream(false))).unwrap();
        assert_eq!((s.source.as_str(), s.text.as_str()), ("Lithium Studio", "48 kHz · 64 frames · 1.3 ms"));
        assert!(s.live && !s.forced);
    }

    #[test]
    fn otherwise_the_server_clock_forced_or_not() {
        let mut c = Clock::default();
        let s = summary(Some(&c), None).unwrap();
        assert_eq!((s.source.as_str(), s.text.as_str()), ("PipeWire", "48 kHz · 1024 frames · 21.3 ms"));
        assert!(!s.forced && !s.live);
        c.force_rate = 96_000;
        c.force_quantum = 128;
        let s = summary(Some(&c), Some(&stream(true))).unwrap();
        assert_eq!(s.text, "96 kHz · 128 frames · 1.3 ms");
        assert!(s.forced && s.live, "a server-owned stream shows the server's clock");
        assert!(summary(None, None).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "Found \"settings\" metadata 32\n\
        update: id:0 key:'log.level' value:'2' type:''\n\
        update: id:0 key:'clock.rate' value:'48000' type:''\n\
        update: id:0 key:'clock.allowed-rates' value:'[ 48000 ]' type:''\n\
        update: id:0 key:'clock.quantum' value:'1024' type:''\n\
        update: id:0 key:'clock.min-quantum' value:'32' type:''\n\
        update: id:0 key:'clock.max-quantum' value:'2048' type:''\n\
        update: id:0 key:'clock.force-quantum' value:'128' type:''\n\
        update: id:0 key:'clock.force-rate' value:'96000' type:''\n";

    #[test]
    fn parses_pw_metadata_output() {
        let c = parse_metadata(DUMP).unwrap();
        assert_eq!((c.rate, c.quantum), (48_000, 1024));
        assert_eq!((c.force_rate, c.force_quantum), (96_000, 128));
        assert_eq!((c.effective_rate(), c.effective_quantum()), (96_000, 128));
        assert!(c.forced());
        assert!(parse_metadata("not metadata").is_none());
    }

    #[test]
    fn forced_values_of_zero_mean_defaults() {
        let c = parse_metadata(&DUMP.replace("'128'", "'0'").replace("'96000'", "'0'")).unwrap();
        assert_eq!((c.effective_rate(), c.effective_quantum()), (48_000, 1024));
        assert!(!c.forced());
    }

    #[test]
    fn latency_and_rate_labels() {
        assert!((latency_ms(128, 48_000) - 2.667).abs() < 0.001);
        assert!((latency_ms(1024, 44_100) - 23.22).abs() < 0.01);
        assert_eq!(latency_ms(256, 0), 0.0);
        assert_eq!(format_rate(44_100), "44.1 kHz");
        assert_eq!(format_rate(96_000), "96 kHz");
        assert_eq!(format_rate(176_400), "176.4 kHz");
    }

    #[test]
    fn set_reads_back_and_reset_releases() {
        let mut a = AudioClock::new(Box::new(Memory::default()));
        a.refresh();
        a.set_rate(96_000);
        a.set_quantum(64);
        let c = a.clock.unwrap();
        assert_eq!((c.effective_rate(), c.effective_quantum()), (96_000, 64));
        a.reset();
        let c = a.clock.unwrap();
        assert_eq!((c.effective_rate(), c.effective_quantum()), (48_000, 1024));
        assert!(a.error.is_none());
    }

    struct Failing;
    impl ClockControl for Failing {
        fn read(&mut self) -> Result<Clock, String> {
            Err("no server".into())
        }
        fn set_rate(&mut self, _: u32) -> Result<(), String> {
            Err("no server".into())
        }
        fn set_quantum(&mut self, _: u32) -> Result<(), String> {
            Err("no server".into())
        }
    }

    #[test]
    fn errors_are_kept_for_the_panel() {
        let mut a = AudioClock::new(Box::new(Failing));
        a.refresh();
        assert!(a.clock.is_none() && a.error.as_deref() == Some("no server"));
        a.set_quantum(128);
        assert_eq!(a.error.as_deref(), Some("no server"));
    }
}
