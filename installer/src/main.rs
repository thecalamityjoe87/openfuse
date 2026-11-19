//! OpenFuse Installer: a graphical installer for the OpenFuse driver and app.

mod engine;
mod helper;
mod packagekit;
mod secureboot;
mod sysinfo;
mod theme;
mod ui;

use std::path::PathBuf;

const USAGE: &str = "\
OpenFuse Installer

Usage: openfuse-installer [options]

  --simulate          Try the installer without changing anything
  --resume            Finish the setup after a restart (used automatically)
  --payload DIR       Install the .deb packages from DIR (default: the installer's own folder)
  --theme light|dark  Override the desktop's light/dark preference
  --check             Print the system checks and exit
  --resolve NAME...   Ask the package system about NAME (diagnostic) and exit
  --help              Show this help
";

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--check") {
        print_checks();
        return Ok(());
    }
    if let Some(pos) = args.iter().position(|a| a == "--resolve") {
        let names: Vec<&str> = args[pos + 1..].iter().map(String::as_str).collect();
        match packagekit::Client::connect().and_then(|c| c.resolve(&names, 0)) {
            Ok(pkgs) => pkgs.iter().for_each(|p| println!("info={} {} — {}", p.info, p.id, p.summary)),
            Err(e) => eprintln!("error: {e}"),
        }
        return Ok(());
    }

    let opts = ui::Options {
        simulate: args.iter().any(|a| a == "--simulate"),
        resume: args.iter().any(|a| a == "--resume"),
        payload: value("--payload").map(PathBuf::from),
        screenshot: value("--screenshot").map(|s| {
            let (page, path) = s.split_once(':').unwrap_or(("welcome", s.as_str()));
            (page.to_string(), PathBuf::from(path))
        }),
        dark: value("--theme").map(|t| t == "dark"),
    };
    ui::run(opts)
}

fn print_checks() {
    let info = sysinfo::gather();
    for c in info.checks() {
        let mark = match c.status {
            sysinfo::Status::Good => "ok  ",
            sysinfo::Status::Note => "note",
            sysinfo::Status::Blocker => "FAIL",
        };
        println!("[{mark}] {:30} {}", c.label, c.value);
        if !c.detail.is_empty() {
            println!("       {}", c.detail);
        }
    }
    println!(
        "installed: dkms={:?} app={:?} controls_present={} mok_cert={:?} packages_found={}",
        info.installed_dkms,
        info.installed_app,
        info.controls_present,
        info.mok_cert,
        engine::Payload::locate(None).is_ok()
    );
}
