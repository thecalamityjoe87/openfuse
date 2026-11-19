//! GAIN calibration wizard dialog (see calibrate.rs for the logic): a modal
//! sheet like About, opened from the Audio panel.

use super::theme;
use super::{App, Message};
use crate::calibrate::{Phase, RECORD_S, SETTLE_S};
use crate::knobs::CAL_STEPS;
use iced::alignment::Horizontal;
use iced::widget::{button, center, column, container, mouse_area, opaque, progress_bar, row, space, text};
use iced::{Alignment, Border, Color, Element, Length, Shadow, Vector, mouse};

fn db(v: f64) -> String {
    format!("{v:.1} dBFS").replace('-', "−")
}

fn on_off(v: Option<bool>) -> &'static str {
    if v == Some(true) { "on" } else { "off" }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

fn btn<'a>(label: &'a str, msg: Option<Message>, primary: bool) -> Element<'a, Message> {
    let b = button(text(label).size(15).font(theme::MEDIUM)).padding([10, 22]).on_press_maybe(msg);
    if primary { b.style(theme::primary) } else { b.style(theme::secondary) }.into()
}

pub fn dialog(app: &App) -> Element<'_, Message> {
    let t = *app.tokens();
    let Some(w) = app.cal_wizard.as_ref() else { return space::vertical().height(0).into() };
    let label = app.model.part(w.knob).map_or("GAIN", |p| p.label);
    let n = w.steps();
    let now = app.started_at_secs();

    let (clock, pointing) = (CAL_STEPS[w.step].1, CAL_STEPS[w.step].2);
    let live: Element<'_, Message> = match w.live_db {
        Some(v) => text(format!("Input now: {}", db(v))).size(13).color(t.muted).into(),
        None => text("Waiting for the interface's meters…").size(13).color(t.muted).into(),
    };
    let rules = text(
        "Set the input up the way you use it, and keep it still: don't touch the instrument or its cable \
         while recording. Nothing plays through the speakers.",
    )
    .size(13)
    .color(t.muted);
    // an empty input hides the bottom of the travel under converter noise; an open plug on a Hi-Z input lifts it
    let tip = text(
        "Nothing plugged in? Plug in an empty jack (an adapter will do) and turn INST on for the full range, \
         and leave them that way.",
    )
    .size(13)
    .color(t.text);

    let mut body = column![].spacing(12).width(Length::Fill);
    let mut buttons = row![btn("Cancel", Some(Message::CalCancel), false)].spacing(10).align_y(Alignment::Center);
    let can_back = !w.is_recording() && (w.step > 0 || matches!(w.phase, Phase::Summary | Phase::Failed(_)));
    if can_back {
        buttons = buttons.push(btn("Back", Some(Message::CalBack), false));
    }
    buttons = buttons.push(space::horizontal());

    match &w.phase {
        Phase::Summary => {
            body = body.push(text(format!("{label}: all {n} positions recorded")).size(20).font(theme::SEMIBOLD).color(t.text));
            let mut table = column![].spacing(4);
            for (i, step) in CAL_STEPS.iter().enumerate() {
                let v = w.floors[i].map(db).unwrap_or_else(|| "—".into());
                table = table.push(
                    row![
                        text(format!("{} o'clock", step.1)).size(14).color(t.text).width(110),
                        text(v).size(14).font(theme::MEDIUM).color(t.text),
                    ]
                    .spacing(8),
                );
            }
            body = body
                .push(table)
                .push(text(format!("Once saved, {label} follows this input's level. You can reset it from the Audio panel.")).size(13).color(t.muted));
            buttons = buttons.push(btn("Save", Some(Message::CalSave), true));
        }
        Phase::Failed(e) => {
            body = body
                .push(text("Can't use this calibration").size(20).font(theme::SEMIBOLD).color(t.bad))
                .push(text(e.clone()).size(14).color(t.text));
        }
        phase => {
            body = body
                .push(text(format!("Step {} of {}", w.step + 1, n)).size(13).font(theme::SEMIBOLD).color(t.muted))
                .push(
                    column![
                        text(format!("Turn {label} to {clock} o'clock")).size(20).font(theme::SEMIBOLD).color(t.text),
                        text(capitalize(pointing)).size(14).color(t.text),
                    ]
                    .spacing(2),
                )
                .push(rules);
            if w.step == 0 && matches!(phase, Phase::Ready) {
                body = body.push(tip);
            }
            body = body.push(live);
            // the calibration is for the INST setting the wizard opened with
            let inst_changed = w.inst.is_some() && app.inst_on(w.knob) != w.inst;
            if inst_changed && !matches!(phase, Phase::Recording { .. }) {
                body = body.push(
                    text(format!("INST was switched. Turn it back {} to go on: this calibration is for INST {}.", on_off(w.inst), on_off(w.inst)))
                        .size(14)
                        .color(t.warn),
                );
            }
            let record = (!inst_changed).then_some(Message::CalRecord);
            match phase {
                Phase::Ready => buttons = buttons.push(btn("Record", record, true)),
                Phase::Recording { .. } => {
                    let left = ((SETTLE_S + RECORD_S) * (1.0 - w.progress(now))).ceil().max(0.0);
                    body = body
                        .push(progress_bar(0.0..=1.0, w.progress(now) as f32).girth(8).style(theme::progress))
                        .push(text(format!("Recording… keep still ({left:.0} s)")).size(13).color(t.text));
                    buttons = buttons.push(btn("Recording…", None, true));
                }
                Phase::Done(v) => {
                    body = body.push(text(format!("Recorded {}", db(*v))).size(14).font(theme::SEMIBOLD).color(t.good));
                    buttons = buttons.push(btn("Redo", record, false));
                    buttons = buttons.push(btn(if w.step + 1 < n { "Next" } else { "Finish" }, Some(Message::CalNext), true));
                }
                Phase::Unsteady(spread) => {
                    body = body.push(
                        text(format!("The level wasn't steady (it varied by {spread:.0} dB). Keep the instrument still and try again."))
                            .size(14)
                            .color(t.warn),
                    );
                    buttons = buttons.push(btn("Try again", record, true));
                }
                Phase::Summary | Phase::Failed(_) => {}
            }
        }
    }

    let sheet = container(
        column![
            text(match w.inst {
                Some(_) => format!("Calibrate {label} · INST {}", on_off(w.inst)),
                None => format!("Calibrate {label}"),
            })
            .size(24).font(theme::BOLD).color(t.text).width(Length::Fill).align_x(Horizontal::Left),
            body,
            buttons,
        ]
        .spacing(18)
        .width(Length::Fill),
    )
    .padding(28)
    .width(500)
    .style(move |_| container::Style {
        background: Some(t.bg.into()),
        border: Border { color: t.border, width: 1.0, radius: 18.0.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.28), offset: Vector::new(0.0, 12.0), blur_radius: 40.0 },
        ..Default::default()
    });
    let radius = app.corner_radius();
    // a click on the backdrop does nothing: a calibration is not thrown away by accident
    opaque(
        mouse_area(center(opaque(sheet)).style(move |_| container::Style {
            background: Some(Color::from_rgba(0.04, 0.05, 0.08, if t.dark { 0.55 } else { 0.32 }).into()),
            border: Border { radius: radius.into(), ..Default::default() },
            ..Default::default()
        }))
        .interaction(mouse::Interaction::Idle),
    )
}
