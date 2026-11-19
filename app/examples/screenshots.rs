//! Render the real UI headless (iced's tiny-skia renderer via iced_test) with
//! the mock backend and save screenshots.
//!
//!     cargo run --example screenshots [-- OUTDIR]     (default: screenshots/)

use iced::{Size, window};
use iced_test::simulator::Simulator;
use openfuse::autostart::Settings;
use openfuse::backend::mock::MockBackend;
use openfuse::models::{self, ModelSpec, View};
use openfuse::ui::{App, Message, ThemeMode};
use std::path::{Path, PathBuf};

const SIZE: (f32, f32) = (1280.0, 800.0);
/// Signal for the screenshots: inputs 1/2 (voice, guitar), then the output pair.
fn levels(model: &ModelSpec) -> Vec<f64> {
    let mut ins = [-9.0, -20.0, -15.0, -26.0].into_iter();
    let mut outs = [-5.0, -8.0].into_iter().cycle();
    model.meters.labels.iter().map(|l| if l.starts_with("Input") { ins.next().unwrap_or(-30.0) } else { outs.next().unwrap() }).collect()
}

fn make(model: &'static ModelSpec, theme: ThemeMode, settings_dir: &Path) -> App {
    let be = MockBackend::still(model);
    let mut app = App::new(Box::new(be), theme, Settings::at(settings_dir.join("settings.ini")));
    app.window_size = Size::new(SIZE.0, SIZE.1);
    app.scale = 2.0; // the simulator renders at 2x
    app.rounded = true; // as on Wayland
    app.set_visible(true);
    app.start();
    app.mock().set_meter_levels(&levels(model));
    let _ = app.update(Message::Tick);
    app
}

fn settings() -> iced::Settings {
    use openfuse::ui::theme;
    iced::Settings {
        default_font: theme::INTER,
        fonts: theme::FONTS.iter().map(|f| std::borrow::Cow::Borrowed(*f)).collect(),
        ..Default::default()
    }
}

/// Clear the pixels outside the rounded window corners (what the compositor shows).
fn round_corners(w: u32, h: u32, rgba: &mut [u8], r: f32) {
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = if fx < r { r } else if fx > w as f32 - r { w as f32 - r } else { continue };
            let cy = if fy < r { r } else if fy > h as f32 - r { h as f32 - r } else { continue };
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let cover = (r + 0.5 - d).clamp(0.0, 1.0);
            let i = ((y * w + x) * 4 + 3) as usize;
            rgba[i] = (rgba[i] as f32 * cover).round() as u8;
        }
    }
}

/// Snapshot the window and write it as `name.png` (1x) and `name@2x.png`.
fn shoot(app: &App, out: &Path, name: &str, hidpi: bool) {
    let tmp = out.join(format!(".tmp-{name}.png"));
    let theme = app.theme(window::Id::unique());
    {
        let mut sim = Simulator::with_size(settings(), Size::new(SIZE.0, SIZE.1), app.view(window::Id::unique()));
        let snap = sim.snapshot(&theme).expect("snapshot");
        let _ = std::fs::remove_file(&tmp);
        snap.matches_image(&tmp).expect("write png");
    }
    // matches_image appends the renderer name to the file name
    let written: PathBuf = std::fs::read_dir(out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with(&format!(".tmp-{name}-")))
        .expect("snapshot file");
    let (w, h, mut rgba) = read_png(&written);
    round_corners(w, h, &mut rgba, openfuse::ui::CORNER * 2.0);
    if hidpi {
        write_png(&out.join(format!("{name}@2x.png")), w, h, &rgba);
    }
    let (w1, h1, small) = half(w, h, &rgba);
    write_png(&out.join(format!("{name}.png")), w1, h1, &small);
    std::fs::remove_file(written).unwrap();
    println!("  {name}.png");
}

fn read_png(p: &Path) -> (u32, u32, Vec<u8>) {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).unwrap()));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn write_png(p: &Path, w: u32, h: u32, rgba: &[u8]) {
    let mut enc = png::Encoder::new(std::fs::File::create(p).unwrap(), w, h);
    enc.set_color(png::ColorType::Rgba);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(rgba).unwrap();
    wr.finish().unwrap();
}

/// 2x2 box downscale.
fn half(w: u32, h: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w2, h2) = (w / 2, h / 2);
    let mut out = vec![0u8; (w2 * h2 * 4) as usize];
    for y in 0..h2 {
        for x in 0..w2 {
            for c in 0..4 {
                let px = |dx: u32, dy: u32| rgba[(((y * 2 + dy) * w + x * 2 + dx) * 4 + c) as usize] as u32;
                out[((y * w2 + x) * 4 + c) as usize] = ((px(0, 0) + px(1, 0) + px(0, 1) + px(1, 1) + 2) / 4) as u8;
            }
        }
    }
    (w2, h2, out)
}

fn main() {
    // deterministic software rendering, like the tiny-skia fallback
    unsafe { std::env::set_var("ICED_TEST_BACKEND", "tiny-skia") };
    let out = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("screenshots"));
    std::fs::create_dir_all(&out).unwrap();
    let cfg = tempfile::tempdir().unwrap();
    println!("saving to {}", out.display());

    for (theme, sfx) in [(ThemeMode::Light, ""), (ThemeMode::Dark, "-dark")] {
        let mut app = make(&models::MINIFUSE2, theme, cfg.path());
        // two buttons "pressed on the hardware": cards flash, status bar names the change
        app.mock().simulate_hardware_press("in1_inst");
        app.mock().simulate_hardware_press("direct_mono");
        let _ = app.update(Message::Tick);
        if theme == ThemeMode::Light {
            shoot(&app, &out, "hardware-press", false);
        }
        app.clear_transients();
        shoot(&app, &out, &format!("front{sfx}"), theme == ThemeMode::Light);

        // clipping: input 1 and output L hit full scale; input 2 has signal
        app.mock().set_meter_levels(&[0.0, -18.0, 0.0, -8.0]);
        let _ = app.update(Message::Tick);
        shoot(&app, &out, &format!("clipping{sfx}"), theme == ThemeMode::Light);
        let _ = app.update(Message::ClearClip(vec![0, 1, 2, 3]));
        // idle: no signal, the GAIN knob lights are dark
        app.mock().set_meter_raw(&[0, 0, 0, 0]);
        let _ = app.update(Message::Tick);
        app.advance_meters(10.0);
        if theme == ThemeMode::Light {
            shoot(&app, &out, "idle", false);
        }
        app.mock().set_meter_levels(&levels(&models::MINIFUSE2));
        let _ = app.update(Message::Tick);
        let _ = app.update(Message::SetView(View::Back));
        shoot(&app, &out, &format!("back{sfx}"), false);

        for (k, v) in [("loopback", true), ("loopback_to_usb", true), ("lb_mute_2", true)] {
            let _ = app.update(Message::Toggle(k.into(), v));
        }
        let _ = app.update(Message::SetGain("lb_vol_l_0".into(), -6.0));
        let _ = app.update(Message::SetGain("lb_vol_r_0".into(), -6.0));
        let _ = app.update(Message::PartClicked("usb-host"));
        app.mock().set_meter_levels(&levels(&models::MINIFUSE2));
        let _ = app.update(Message::Tick);
        shoot(&app, &out, &format!("loopback{sfx}"), false);
        let _ = app.update(Message::SetPanel(false));

        let _ = app.update(Message::SetView(View::Front));
        let _ = app.update(Message::SetAudioPanel(true));
        let _ = app.update(Message::SetClockRate(96_000));
        let _ = app.update(Message::SetClockQuantum(128));
        shoot(&app, &out, &format!("audio-settings{sfx}"), false);
        let _ = app.update(Message::SetAudioPanel(false));

        // virtual GAIN knobs set by hand (the device can't report them); the others are fixed
        for (k, v) in [("in1-gain", 0.55), ("in2-gain", 0.25)] {
            let _ = app.update(Message::SetKnob(k, v));
        }
        shoot(&app, &out, &format!("knobs{sfx}"), false);
        for k in ["in1-gain", "in2-gain"] {
            let _ = app.update(Message::ResetKnob(k));
        }

        // GAIN calibration: the Audio panel section, and a wizard step being recorded
        let _ = app.update(Message::SetAudioPanel(true));
        shoot(&app, &out, &format!("audio-calibration{sfx}"), false);
        let _ = app.update(Message::CalStart("in1-gain"));
        let _ = app.update(Message::CalNext); // (ignored until a step is recorded)
        app.cal_record_at(0.0);
        for i in 0..80 {
            app.feed_gain_estimators(i as f64 * 0.04, &[-41.4, -95.0, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
        }
        shoot(&app, &out, &format!("calibration{sfx}"), false);
        let _ = app.update(Message::CalCancel);
        let _ = app.update(Message::SetAudioPanel(false));
        let _ = app.update(Message::PartClicked("phantom-48v"));
        assert!(app.confirm.is_some(), "48 V asks first");
        shoot(&app, &out, &format!("phantom-confirm{sfx}"), false);
        let _ = app.update(Message::Confirm(true));
        assert!(app.value_bool("phantom"));
        app.clear_transients();
        shoot(&app, &out, &format!("phantom-on{sfx}"), false);
    }

    for key in ["minifuse1", "minifuse4"] {
        let mut app = make(models::model_by_key(key).unwrap(), ThemeMode::Light, cfg.path());
        shoot(&app, &out, &format!("preview-{key}-front"), false);
        let _ = app.update(Message::SetView(View::Back));
        shoot(&app, &out, &format!("preview-{key}-back"), false);
    }
}
