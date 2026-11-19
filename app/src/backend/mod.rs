//! Backend interface.
//!
//! The UI only talks to a [`Backend`]: it reads values with `get`, requests
//! changes with `set` and observes the device through [`Event`]s. A backend
//! never assumes a requested change happened: the UI is updated only from
//! events (a read-back after `set`, an ALSA change event, or a simulated
//! hardware press).
//!
//! Events returned by `set` are the read-back of the UI's own request; events
//! returned by `poll` happened on the device by itself (hardware buttons).

pub mod alsa;
mod alsa_ffi;
pub mod detect;
pub mod mock;

use crate::models::ModelSpec;
use std::any::Any;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Bool(bool),
    /// loopback gains, dB
    Db(f64),
}

impl Value {
    pub fn as_bool(self) -> bool {
        match self {
            Value::Bool(b) => b,
            Value::Db(d) => d != 0.0,
        }
    }
    pub fn as_db(self) -> f64 {
        match self {
            Value::Db(d) => d,
            Value::Bool(b) => f64::from(u8::from(b)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Connected,
    Disconnected,
    /// --mock
    Simulated,
    /// no MiniFuse found, simulating
    NoDevice,
    /// MiniFuse present, OpenFuse ALSA controls absent, simulating
    ControlsMissing,
}

impl Status {
    pub fn text(self) -> &'static str {
        match self {
            Status::Connected => "connected",
            Status::Disconnected => "disconnected",
            Status::Simulated => "simulated",
            Status::NoDevice => "not connected",
            Status::ControlsMissing => "controls missing · simulated",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// (control key, new value)
    ValueChanged(String, Value),
    /// dBFS in the order of `model.meters.labels`
    Meters(Vec<f64>),
    /// human-readable problem (write failed, device vanished ...)
    Error(String),
    StatusChanged(Status),
}

pub trait Backend {
    fn model(&self) -> &'static ModelSpec;
    fn simulated(&self) -> bool;
    fn status(&self) -> Status;
    /// Shown in the banner/header, e.g. "card 3 (hw:3)" or why we simulate.
    fn description(&self) -> String;

    /// Begin listening. Values are readable after this.
    fn start(&mut self) -> Vec<Event>;
    /// Release resources.
    fn stop(&mut self);

    /// Control keys this backend can read/write.
    fn keys(&self) -> Vec<String>;
    fn available(&self, key: &str) -> bool {
        self.keys().iter().any(|k| k == key)
    }
    fn get(&self, key: &str) -> Option<Value>;
    /// Request a change. Implementations write, read the value back and
    /// return the resulting events (empty if nothing changed).
    fn set(&mut self, key: &str, value: Value) -> Vec<Event>;
    /// Last meter reading (dBFS).
    fn meters(&self) -> Vec<f64>;

    /// Window visible (true) or hidden (false). While hidden a backend stops
    /// meter polling and keeps only control-change notifications.
    fn set_live(&mut self, on: bool);
    fn meters_polling(&self) -> bool;

    /// Can the output level be shown whichever application plays? False for a
    /// driver without the playback meter: only the sound server's sink is
    /// measured then, which an app on `hw:` bypasses.
    fn output_metered(&self) -> bool {
        true
    }

    /// The playback stream currently open on the card, if any (see
    /// alsa_stream.rs). Only the real device backend knows.
    fn playback_stream(&self) -> Option<crate::alsa_stream::StreamInfo> {
        None
    }

    /// Called on a timer (25 Hz while visible, slower while hidden): device
    /// events, meters, resync.
    fn poll(&mut self) -> Vec<Event>;

    fn as_any_mut(&mut self) -> &mut dyn Any;
}
