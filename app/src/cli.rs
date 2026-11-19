//! Command line.

use crate::alsa_controls;
use crate::backend::alsa::list_card_controls;
use crate::backend::detect::find_cards;
use crate::models::MODELS;
use crate::ui::ThemeMode;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub mock: bool,
    pub model: Option<String>,
    pub card: Option<u32>,
    pub theme: ThemeMode,
    pub tray: bool,
    pub no_tray: bool,
    pub list_controls: bool,
    pub verbose: bool,
    pub help: bool,
    pub version: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            mock: false,
            model: None,
            card: None,
            theme: ThemeMode::Auto,
            tray: false,
            no_tray: false,
            list_controls: false,
            verbose: false,
            help: false,
            version: false,
        }
    }
}

pub fn usage() -> String {
    let models: Vec<&str> = MODELS.iter().map(|m| m.key).collect();
    format!(
        "Usage: openfuse [OPTIONS]\n\n\
         Control panel for Arturia MiniFuse interfaces.\n\n\
         Options:\n  \
         --mock               use the simulated device even if a MiniFuse is connected\n  \
         --model MODEL        model to simulate with --mock ({}; default: minifuse2)\n  \
         --card N             ALSA card number to use (default: first MiniFuse found)\n  \
         --theme THEME        auto | light | dark (default: auto, follows the desktop)\n  \
         --tray               start hidden in the system tray (used by start-at-login);\n                       \
         shows the window anyway if there is no tray\n  \
         --no-tray            never use the system tray (closing the window quits)\n  \
         --list-controls      print the card's ALSA controls and which OpenFuse names resolve, then exit\n  \
         -v, --verbose        log diagnostics to stderr\n  \
         -h, --help           show this help\n  \
         -V, --version        show the version\n",
        models.join(" | ")
    )
}

pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |what: &str| -> Result<String, String> {
            inline.clone().or_else(|| it.next()).ok_or_else(|| format!("{what} needs a value"))
        };
        match name.as_str() {
            "--mock" => a.mock = true,
            "--model" => {
                let v = value("--model")?;
                if !MODELS.iter().any(|m| m.key == v) {
                    return Err(format!("unknown model '{v}'"));
                }
                a.model = Some(v);
            }
            "--card" => a.card = Some(value("--card")?.parse().map_err(|_| "--card needs a number".to_string())?),
            "--theme" => {
                a.theme = match value("--theme")?.as_str() {
                    "auto" => ThemeMode::Auto,
                    "light" => ThemeMode::Light,
                    "dark" => ThemeMode::Dark,
                    v => return Err(format!("unknown theme '{v}' (auto, light, dark)")),
                }
            }
            "--tray" => a.tray = true,
            "--no-tray" => a.no_tray = true,
            "--list-controls" => a.list_controls = true,
            "-v" | "--verbose" => a.verbose = true,
            "-h" | "--help" => a.help = true,
            "-V" | "--version" => a.version = true,
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    Ok(a)
}

/// `--list-controls`: show the card's controls and the OpenFuse mapping.
pub fn list_controls(card: Option<u32>) -> i32 {
    let cards: Vec<_> = find_cards(Path::new("/proc/asound")).into_iter().filter(|c| card.is_none_or(|n| c.index == n)).collect();
    let Some(c) = cards.first() else {
        println!("No MiniFuse card found in /proc/asound.");
        return 1;
    };
    println!("{} on card {} ({}), USB {}", c.model.name, c.index, c.hw(), c.usbid);
    match list_card_controls(&c.hw()) {
        Err(e) => {
            println!("Cannot open {}: {e}", c.hw());
            1
        }
        Ok((present, mapping)) => {
            println!("\nControls on the card:");
            for (iface, name, idx) in present {
                println!("  {iface:6} '{name}' index {idx}");
            }
            println!("\nOpenFuse mapping (src/alsa_controls.rs):");
            for (key, ok) in mapping {
                let name = &alsa_controls::lookup(&key).expect("key").name;
                println!("  {} {key:22} -> '{name}'", if ok { "ok     " } else { "MISSING" });
            }
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn flags() {
        let a = p("--mock --model minifuse4 --theme=dark --card 3 --tray").unwrap();
        assert!(a.mock && a.tray && !a.no_tray);
        assert_eq!(a.model.as_deref(), Some("minifuse4"));
        assert_eq!(a.theme, ThemeMode::Dark);
        assert_eq!(a.card, Some(3));
        assert!(p("--model nope").is_err());
        assert!(p("--theme blue").is_err());
        assert!(p("--card").is_err());
        assert!(p("--frobnicate").is_err());
        assert_eq!(p("").unwrap(), Args::default());
    }
}
