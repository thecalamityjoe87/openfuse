//! Audio settings panel: sample rate and buffer size of the audio server
//! (see audioclock.rs). A slide-out on the right of the main window.

use super::{App, Message, theme};
use crate::audioclock::{self, Clock, QUANTA, RATES};
use iced::widget::{button, column, container, pick_list, row, scrollable, space, text};
use iced::{Alignment, Element, Length};

pub const WIDTH: f32 = 360.0;

/// Entry of the sample-rate drop-down; 0 = leave it to the server.
#[derive(Debug, Clone)]
pub struct RateChoice {
    pub hz: u32,
    /// the server's own rate, shown on the automatic entry
    pub default_hz: u32,
}

impl PartialEq for RateChoice {
    fn eq(&self, o: &Self) -> bool {
        self.hz == o.hz
    }
}

impl std::fmt::Display for RateChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.hz {
            0 => write!(f, "Automatic ({})", audioclock::format_rate(self.default_hz)),
            hz => f.write_str(&audioclock::format_rate(hz)),
        }
    }
}

/// Entry of the buffer-size drop-down; 0 = leave it to the server.
#[derive(Debug, Clone)]
pub struct QuantumChoice {
    pub frames: u32,
    /// rate the latency is worked out at
    pub rate: u32,
    pub default_frames: u32,
}

impl PartialEq for QuantumChoice {
    fn eq(&self, o: &Self) -> bool {
        self.frames == o.frames
    }
}

impl std::fmt::Display for QuantumChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.frames {
            0 => write!(f, "Automatic ({} frames)", self.default_frames),
            n => write!(f, "{n} frames · {:.1} ms", audioclock::latency_ms(n, self.rate)),
        }
    }
}

pub fn rate_choices(clock: &Clock) -> Vec<RateChoice> {
    std::iter::once(0).chain(RATES).map(|hz| RateChoice { hz, default_hz: clock.rate }).collect()
}

pub fn quantum_choices(clock: &Clock) -> Vec<QuantumChoice> {
    let rate = clock.effective_rate();
    std::iter::once(0)
        .chain(QUANTA.into_iter().filter(|q| (clock.min_quantum..=clock.max_quantum).contains(q)))
        .map(|frames| QuantumChoice { frames, rate, default_frames: clock.quantum })
        .collect()
}

fn section<'a>(s: &'a str) -> Element<'a, Message> {
    text(s.to_uppercase()).size(12).font(theme::SEMIBOLD).style(theme::muted).into()
}

fn dropdown<'a, T>(
    options: Vec<T>,
    selected: T,
    on_pick: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message>
where
    T: std::fmt::Display + PartialEq + Clone + 'a,
{
    pick_list(options, Some(selected), on_pick)
        .width(Length::Fill)
        .padding([9, 12])
        .text_size(14)
        .style(theme::pick)
        .menu_style(theme::pick_menu)
        .into()
}

fn body<'a>(clock: &Clock, t: theme::Tokens) -> Element<'a, Message> {
    let rate = RateChoice { hz: clock.force_rate, default_hz: clock.rate };
    let frames = QuantumChoice { frames: clock.force_quantum, rate: clock.effective_rate(), default_frames: clock.quantum };
    let state = if clock.forced() {
        format!(
            "Now: {} · {} frames · {:.1} ms per buffer",
            audioclock::format_rate(clock.effective_rate()),
            clock.effective_quantum(),
            clock.latency_ms()
        )
    } else {
        "Not set by OpenFuse: the audio server decides.".to_string()
    };
    column![
        section("Sample rate"),
        dropdown(rate_choices(clock), rate, |c| Message::SetClockRate(c.hz)),
        section("Buffer size"),
        dropdown(quantum_choices(clock), frames, |c| Message::SetClockQuantum(c.frames)),
        text(state).size(13).color(t.text),
    ]
    .spacing(10)
    .into()
}

/// The GAIN knob calibration rows (see calibrate.rs).
fn calibration_section<'a>(app: &'a App, t: theme::Tokens) -> Element<'a, Message> {
    let knobs = app.model.meters.knob_lights;
    if knobs.is_empty() {
        return space::vertical().height(0).into();
    }
    let mut col = column![section("GAIN knob position")].spacing(10);
    col = col.push(
        text(
            "OpenFuse can't tell an instrument's hum from a high GAIN setting, so it follows the knob only after \
             you set it once (drag the knob on the picture) or calibrate it here. A calibration is kept for each \
             INST setting, and the one that matches INST is used.",
        )
        .size(12)
        .style(theme::muted),
    );
    for (knob, _) in knobs {
        let label = app.model.part(knob).map_or(*knob, |p| p.label);
        let calibrated = app.is_calibrated(knob);
        let mut r = row![
            column![
                text(label).size(14).font(theme::SEMIBOLD).color(t.text),
                text(match (app.calibrated_for(knob, false), app.calibrated_for(knob, true)) {
                    (false, false) => "Not calibrated",
                    (true, false) => "Calibrated with INST off",
                    (false, true) => "Calibrated with INST on",
                    (true, true) => "Calibrated with INST off and on",
                })
                .size(12)
                .style(theme::muted),
            ]
            .spacing(2)
            .width(Length::Fill),
            button(text("Calibrate…").size(13).font(theme::MEDIUM)).padding([7, 14]).on_press(Message::CalStart(knob)).style(theme::secondary),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if calibrated {
            r = r.push(button(text("Reset").size(13).font(theme::MEDIUM)).padding([7, 14]).on_press(Message::CalClear(knob)).style(theme::secondary));
        }
        col = col.push(r);
    }
    col.into()
}

/// What is playing to the card right now, from /proc/asound.
fn stream_note<'a>(app: &App, t: theme::Tokens) -> Element<'a, Message> {
    let Some(s) = app.backend().playback_stream() else {
        return space::vertical().height(0).into();
    };
    let (title, detail, color) = if s.via_server {
        (
            format!("{} is playing through the audio server", s.owner),
            "The settings below apply to it.".to_string(),
            t.good,
        )
    } else {
        (
            format!("{} is using the MiniFuse directly (ALSA)", s.owner),
            format!(
                "Its rate and buffer size are set in {}'s own audio settings, not here. The controls below only affect apps \
                 that play through PipeWire or JACK.",
                s.owner
            ),
            t.warn,
        )
    };
    container(
        column![
            text(title).size(13).font(theme::SEMIBOLD).color(t.text),
            text(s.summary()).size(13).color(t.text),
            text(detail).size(12).style(theme::muted),
        ]
        .spacing(4),
    )
    .padding(12)
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(theme::alpha(color, 0.10).into()),
        border: iced::Border { color: theme::alpha(color, 0.35), width: 1.0, radius: 10.0.into() },
        ..Default::default()
    })
    .into()
}

pub fn panel(app: &App) -> Element<'_, Message> {
    let t = *app.tokens();
    let head = row![
        text("Audio settings").size(18).font(theme::BOLD).color(t.text),
        space::horizontal(),
        button(text("Close").size(13).font(theme::MEDIUM))
            .padding([5, 12])
            .on_press(Message::SetAudioPanel(false))
            .style(theme::secondary),
    ]
    .align_y(Alignment::Center);

    let content: Element<'_, Message> = match (&app.audio.clock, &app.audio.error) {
        (Some(c), _) => body(c, t),
        (None, Some(e)) => text(e.clone()).size(13).color(t.bad).into(),
        (None, None) => text("Reading the audio server…").size(13).style(theme::muted).into(),
    };
    let error: Element<'_, Message> = match (&app.audio.clock, &app.audio.error) {
        (Some(_), Some(e)) => text(e.clone()).size(13).color(t.bad).into(),
        _ => space::vertical().height(0).into(),
    };

    let notes = column![
        text("Used by DAWs and plugins that run through PipeWire or JACK. A host that opens the card directly (hw:) uses its own settings.")
            .size(12)
            .style(theme::muted),
        text("Applies to the whole audio server, not only the MiniFuse. Audio may drop out for a moment while it changes. Resets when PipeWire restarts; nothing is saved.")
            .size(12)
            .style(theme::muted),
    ]
    .spacing(6);

    container(scrollable(column![head, stream_note(app, t), content, error, notes, calibration_section(app, t)].spacing(16).padding(20)))
        .width(WIDTH)
        .height(Length::Fill)
        .style(theme::card)
        .into()
}
