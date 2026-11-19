//! Loopback mixer panel (no physical counterpart; opened from the USB-C callout).
//!
//! Labels, ranges and the "off" value all come from `LoopbackSpec` in
//! models.rs, because most of it is still unverified (spec §4.3, §4.4).

use super::glyphs::{Glyph, Tint, glyph};
use super::meterbars::{self, tip_bubble};
use super::theme;
use super::{App, Message};
use crate::layout;
use crate::models::LoopbackSpec;
use iced::widget::{button, column, container, row, slider, space, text, toggler, tooltip};
use iced::{Alignment, Element, Length, Padding};

/// Slider positions: 0 = off (bottom notch), then `gain_step_db` steps from min to max.
pub fn steps(spec: &LoopbackSpec) -> u32 {
    ((spec.gain_max_db - spec.gain_min_db) / spec.gain_step_db).round() as u32
}

pub fn db_for(spec: &LoopbackSpec, pos: f64) -> f64 {
    if pos <= 0.0 { spec.gain_off_db } else { spec.gain_min_db + (pos.round() - 1.0) * spec.gain_step_db }
}

pub fn pos_for(spec: &LoopbackSpec, db: f64) -> f64 {
    if db < spec.gain_min_db - 1e-6 {
        0.0
    } else {
        ((db.min(spec.gain_max_db) - spec.gain_min_db) / spec.gain_step_db).round() + 1.0
    }
}

pub fn format_gain(db: f64, spec: &LoopbackSpec) -> String {
    if db < spec.gain_min_db - 1e-6 {
        return "off".into();
    }
    let s = if db == 0.0 { "0.0 dB".to_string() } else { format!("{db:+.1} dB") };
    s.replace('-', "−")
}

const NAME_W: f32 = 48.0;
const NOTE_W: f32 = 168.0;
const VAL_W: f32 = 64.0;
const MUTE_W: f32 = 64.0;

fn section<'a>(s: &'a str, w: Length) -> Element<'a, Message> {
    text(s.to_uppercase()).size(12).font(theme::SEMIBOLD).style(theme::muted).width(w).into()
}

pub fn panel<'a>(app: &'a App, spec: &'static LoopbackSpec, width: f32) -> Element<'a, Message> {
    let t = *app.tokens();
    let avail = |k: &str| app.available.contains(k);

    // ---- head: title + subtitle, loopback meters, the three switches, close
    let title = text("Loopback mixer").size(18).font(theme::BOLD).color(t.text);
    let sub = text("Mixes inputs and computer playback into one stereo pair.")
        .size(13)
        .color(t.muted);
    let mut head = row![column![title, sub].spacing(4).width(Length::Fill)].spacing(20).align_y(Alignment::Center);
    let ms = &app.model.meters;
    if ms.loopback_indices.len() == 2 {
        let vals = ms.loopback_indices.iter().map(|i| app.meter_level(*i)).collect();
        head = head.push(meterbars::bars(
            vec!["L", "R"],
            vals,
            ms.floor_db,
            t,
            Length::Fixed(210.0),
            "Loopback mix L/R, peak dBFS",
            ms.loopback_indices.iter().map(|i| app.is_clipping(*i)).collect(),
        ));
    }
    for (key, label, hint) in spec.switches {
        let ok = avail(key);
        let sw = toggler(app.value_bool(key))
            .size(22)
            .on_toggle_maybe(ok.then_some(move |v| Message::Toggle(key.to_string(), v)))
            .style(theme::switch(false));
        let lab = text(*label).size(14).font(theme::MEDIUM).color(t.text);
        head = head.push(tooltip(
            row![lab, sw].spacing(10).align_y(Alignment::Center),
            tip_bubble(if ok { hint } else { "Not exposed by the driver" }),
            tooltip::Position::Bottom,
        ));
    }
    let close = button(glyph(Glyph::Cross, Tint::Chrome, 30.0))
        .width(30)
        .height(30)
        .padding(0)
        .on_press(Message::SetPanel(false))
        .style(theme::chrome_button(false));
    let head = container(head.push(close)).height(layout::PANEL_HEAD_H as f32).center_y(layout::PANEL_HEAD_H as f32);

    // ---- channel grid
    let mut grid = column![
        row![
            section("Channel", Length::Fixed(NAME_W + NOTE_W + 14.0)),
            section("Left", Length::Fill),
            space::horizontal().width(VAL_W),
            section("Right", Length::Fill),
            space::horizontal().width(VAL_W + MUTE_W + 14.0),
        ]
        .spacing(14)
        .height(layout::PANEL_GRID_HEAD_H as f32)
    ];
    let max_pos = steps(spec) as f64 + 1.0;
    for (n, ch) in spec.channels.iter().enumerate() {
        let mut r = row![
            text(ch.label).size(14).font(theme::SEMIBOLD).color(t.text).width(NAME_W),
            text(ch.note).size(12).color(t.muted).width(NOTE_W),
        ]
        .spacing(14)
        .align_y(Alignment::Center)
        .height(layout::PANEL_ROW_H as f32);
        for side in ["l", "r"] {
            let key = format!("lb_vol_{side}_{n}");
            let ok = avail(&key);
            let db = app.value_db(&key).unwrap_or(spec.gain_off_db);
            let k2 = key.clone();
            let s = slider(0.0..=max_pos, pos_for(spec, db), move |p| {
                if ok { Message::SetGain(k2.clone(), db_for(spec, p)) } else { Message::Tick }
            })
            .step(1.0)
            .height(18)
            .width(Length::Fill)
            .style(theme::fader(ok));
            let off = db < spec.gain_min_db - 1e-6;
            r = r.push(s);
            r = r.push(
                container(text(format_gain(db, spec)).size(13).font(theme::MEDIUM).color(if off { t.muted } else { t.text }))
                    .width(VAL_W)
                    .align_x(Alignment::End),
            );
        }
        let mkey = format!("lb_mute_{n}");
        let muted = app.value_bool(&mkey);
        let ok = avail(&mkey);
        let mut m = button(
            container(text(if muted { "Muted" } else { "Mute" }).size(12).font(theme::SEMIBOLD))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(MUTE_W)
        .height(26)
        .padding(0)
        .style(theme::pill_toggle(muted, |t| t.bad));
        if ok {
            m = m.on_press(Message::Toggle(mkey, !muted));
        }
        grid = grid.push(r.push(m));
    }

    container(column![head, grid].spacing(layout::PANEL_SPACING as f32))
        .padding(Padding {
            top: layout::PANEL_PAD_TOP as f32,
            right: 18.0,
            bottom: layout::PANEL_PAD_BOTTOM as f32,
            left: 22.0,
        })
        .width(width)
        .height(layout::panel_height(spec.channels.len()) as f32)
        .style(theme::card)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MINIFUSE2;

    #[test]
    fn gain_positions() {
        let lb = MINIFUSE2.loopback.unwrap();
        assert_eq!(steps(lb), 144);
        assert_eq!(db_for(lb, 0.0), -63.0);
        assert_eq!(db_for(lb, 1.0), -60.0);
        assert_eq!(pos_for(lb, -63.0), 0.0);
        assert_eq!(db_for(lb, pos_for(lb, -6.0)), -6.0);
        assert_eq!(db_for(lb, pos_for(lb, 99.0)), 12.0);
        assert_eq!(format_gain(-3.0, lb), "−3.0 dB");
        assert_eq!(format_gain(6.0, lb), "+6.0 dB");
        assert_eq!(format_gain(0.0, lb), "0.0 dB");
        assert_eq!(format_gain(-63.0, lb), "off");
    }
}
