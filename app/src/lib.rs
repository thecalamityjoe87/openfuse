//! OpenFuse: control panel for Arturia MiniFuse audio interfaces on Linux.
//!
//! The app only changes device settings, and only through the ALSA controls
//! added by the OpenFuse kernel quirk (docs/protocol.md §7.1). It never talks
//! to USB.

pub mod alsa_controls;
pub mod alsa_stream;
pub mod audioclock;
pub mod autostart;
pub mod backend;
pub mod calibrate;
pub mod cli;
pub mod fonts;
pub mod geom;
pub mod icons;
pub mod knobs;
pub mod layout;
pub mod levels;
pub mod models;
pub mod playback;
pub mod render;
pub mod single_instance;
pub mod tray;
pub mod ui;
