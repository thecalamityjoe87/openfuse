//! Callout cards placed around the device render.
//!
//! Live cards (card style: surface, hairline border, 14 px radius) operate a
//! control; analog cards (dashed outline drawn by the stage canvas, muted
//! text) only name a part that cannot be read or set over USB; connector
//! cards are plain captions.

use super::meterbars::{self, tip_bubble};
use super::theme::{self, Tokens, alpha};
use super::{ANALOG_TIP, App, Message};
use crate::models::{Kind, Part};
use iced::widget::{button, column, container, mouse_area, row, space, text, toggler, tooltip};
use iced::{Alignment, Border, Color, Element, Length, Padding};

/// Small uppercase status pill ("ON", "LOOPBACK OFF" ...).
pub fn state_pill<'a>(label: &'a str, color: Color) -> Element<'a, Message> {
    container(text(label).size(10).font(theme::BOLD).color(color))
        .padding([3, 8])
        .style(theme::pill(color))
        .into()
}

fn title<'a>(part: &'static Part, t: Tokens, inert: bool) -> Element<'a, Message> {
    text(part.label).size(14).font(theme::SEMIBOLD).color(if inert { t.muted } else { t.text }).into()
}

fn hint<'a>(s: &'a str, t: Tokens) -> iced::widget::Text<'a> {
    text(s).size(12).color(t.muted)
}

/// Card style of a live callout (hover / hardware-press flash aware).
fn live_style(accent: fn(&Tokens) -> Color, hot: bool, flash: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |th| {
        let t = theme::tokens(th);
        let acc = accent(&t);
        let mut s = theme::card(th);
        if flash {
            s.background = Some(alpha(acc, if t.dark { 0.16 } else { 0.08 }).into());
        }
        s.border = Border {
            color: if hot || flash { acc } else { t.border },
            width: if hot || flash { 1.5 } else { 1.0 },
            radius: 14.0.into(),
        };
        s
    }
}

fn hover_wrap<'a>(part: &'static Part, content: Element<'a, Message>) -> Element<'a, Message> {
    mouse_area(content).on_enter(Message::HoverCard(Some(part.key))).on_exit(Message::HoverCard(None)).into()
}

pub fn card<'a>(app: &'a App, part: &'static Part, w: f32, h: f32) -> Element<'a, Message> {
    let t = *app.tokens();
    match part.kind {
        Kind::Toggle | Kind::Phantom => toggle_card(app, part, t, w, h),
        Kind::Meter => meter_card(app, part, t, w, h),
        Kind::Analog => analog_card(app, part, t, w, h),
        Kind::Connector => connector_card(app, part, t, w, h),
        Kind::Loopback => loopback_card(app, part, t, w, h),
    }
}

fn toggle_card<'a>(app: &'a App, part: &'static Part, t: Tokens, w: f32, h: f32) -> Element<'a, Message> {
    let key = part.control.expect("toggle control");
    let available = app.available.contains(key);
    let on = app.value_bool(key);
    let danger = part.kind == Kind::Phantom;
    let acc = if danger { t.bad } else { t.accent };
    let lit = on && available;
    let sw = toggler(on)
        .size(22)
        .on_toggle_maybe(available.then_some(move |v| Message::Toggle(key.to_string(), v)))
        .style(theme::switch(danger));
    let head = row![title(part, t, false), state_pill(if lit { "ON" } else { "OFF" }, if lit { acc } else { t.muted })]
        .spacing(8)
        .align_y(Alignment::Center);
    let mut left = column![head, hint(if available { part.hint } else { "Not exposed by the driver" }, t)].spacing(4);
    if danger {
        // reversible "Don't ask again" (the tray may not exist)
        left = left.push(
            iced::widget::checkbox(app.ask_phantom())
                .label("Ask before turning on")
                .on_toggle(Message::SetAskPhantom)
                .size(14)
                .spacing(6)
                .text_size(12)
                .style(theme::check),
        );
    }
    let body = row![left, space::horizontal(), sw].spacing(12).align_y(Alignment::Center);
    let accent: fn(&Tokens) -> Color = if danger { |t| t.bad } else { |t| t.accent };
    let c = container(body)
        .padding(Padding { top: 0.0, right: 14.0, bottom: 0.0, left: 16.0 })
        .width(w)
        .height(h)
        .center_y(h)
        .style(live_style(accent, app.card_hot(part.key), app.is_flashing(part.key)));
    hover_wrap(part, c.into())
}

fn meter_card<'a>(app: &'a App, part: &'static Part, t: Tokens, w: f32, h: f32) -> Element<'a, Message> {
    let ms = &app.model.meters;
    let labels: Vec<&'static str> = ms.led_columns.iter().map(|(c, _)| *c).collect();
    let values: Vec<f64> = ms.led_columns.iter().map(|(_, i)| app.meter_level(*i)).collect();
    let bars = meterbars::bars(
        labels,
        values,
        ms.floor_db,
        t,
        Length::Fill,
        "Peak level in dBFS. Inputs are measured after GAIN; outputs from what the computer plays.",
        ms.led_columns.iter().map(|(_, i)| app.is_clipping(*i)).collect(),
    );
    let mut head = row![title(part, t, false)].spacing(8).align_y(Alignment::Center);
    let idx: Vec<usize> = ms.led_columns.iter().map(|(_, i)| *i).collect();
    let names: Vec<&'static str> = if idx.len() == 2 { vec!["L", "R"] } else { ms.led_columns.iter().map(|(c, _)| *c).collect() };
    if let Some(p) = clip_pill_for(app, &idx, &names, t) {
        head = head.push(space::horizontal()).push(p);
    }
    let c = container(column![head, bars].spacing(10))
        .padding(Padding { top: 14.0, right: 16.0, bottom: 14.0, left: 16.0 })
        .width(w)
        .height(h)
        .style(live_style(|t| t.accent, app.card_hot(part.key), app.is_flashing(part.key)));
    hover_wrap(part, c.into())
}

fn analog_card<'a>(app: &'a App, part: &'static Part, t: Tokens, w: f32, h: f32) -> Element<'a, Message> {
    let meter = app.model.meters.card_meter(part.key);
    let app_levels = app;
    let tag: Element<'a, Message> = container(text("ANALOG").size(9).font(theme::BOLD).color(t.muted))
        .padding([2, 7])
        .style(theme::pill(t.muted))
        .into();
    // the ANALOG tag gives way to the CLIP pill while the meter clips
    let head = match meter.and_then(|src| clip_pill(app, src, t)) {
        Some(p) => row![title(part, t, true), p],
        None => row![title(part, t, true), tag],
    }
    .spacing(6)
    .align_y(Alignment::Center);
    // where the knob is shown, once it is (set by the user, or the GAIN estimate)
    let hint_text = app.knob_note(part.key).unwrap_or_else(|| crate::layout::analog_hint(part.hint));
    let mut body = column![head, text(hint_text).size(12).color(t.muted)].spacing(3);
    if let Some(src) = meter {
        // the knob's position is not reported, but its effect shows up as level
        body = body.push(iced::widget::Space::new().height(crate::layout::CARD_BAR_GAP as f32 - 3.0)).push(slim_meter(app_levels, src, t));
    }
    let c = container(body)
        .padding([0, 14])
        .width(w)
        .height(h)
        .center_y(h);
    let tip: &'static str = if part.key.contains("gain") {
        "Emulated: the MiniFuse doesn't report this knob, so OpenFuse estimates it from the input's idle \
         level. Drag or scroll it on the picture to match the device, or calibrate it in Audio settings (A) \
         under GAIN knob position."
    } else {
        ANALOG_TIP
    };
    hover_wrap(part, tooltip(c, tip_bubble(tip), tooltip::Position::Bottom).into())
}

fn connector_card<'a>(app: &'a App, part: &'static Part, t: Tokens, w: f32, h: f32) -> Element<'a, Message> {
    let label = text(part.label).size(12).font(theme::SEMIBOLD).color(t.muted);
    let head: Element<'a, Message> = match app.model.meters.card_meter(part.key).and_then(|src| clip_pill(app, src, t)) {
        Some(p) => row![label, p].spacing(6).align_y(Alignment::Center).into(),
        None => label.into(),
    };
    let mut col = column![head, hint(part.hint, t)].spacing(2).align_x(Alignment::Center);
    if let Some(src) = app.model.meters.card_meter(part.key) {
        col = col
            .push(iced::widget::Space::new().height(crate::layout::CARD_BAR_GAP as f32 - 2.0))
            .push(container(slim_meter(app, src, t)).padding([0, 8]));
    }
    let c = container(col)
    .width(w)
    .height(h)
    .center_x(w)
    .center_y(h);
    hover_wrap(part, c.into())
}

fn loopback_card<'a>(app: &'a App, part: &'static Part, t: Tokens, w: f32, h: f32) -> Element<'a, Message> {
    let on = app.value_bool("loopback");
    let open = app.panel_open;
    let status = state_pill(if on { "LOOPBACK ON" } else { "LOOPBACK OFF" }, if on { t.accent } else { t.muted });
    let btn = button(text(if open { "Hide mixer" } else { "Loopback mixer" }).size(13).font(theme::MEDIUM))
        .padding([5, 12])
        .on_press(Message::SetPanel(!open))
        .style(if open { theme::primary } else { theme::secondary });
    let mut body = column![
        {
            let mut r = row![title(part, t, false), space::horizontal()].spacing(6).align_y(Alignment::Center);
            if let Some(p) = app.model.meters.card_meter(part.key).and_then(|src| clip_pill(app, src, t)) {
                r = r.push(p);
            }
            r.push(status)
        },
        row![hint("USB-C", t), space::horizontal(), btn].align_y(Alignment::Center),
    ]
    .spacing(8);
    if let Some(src) = app.model.meters.card_meter(part.key) {
        body = body.push(slim_meter(app, src, t));
    }
    let c = container(body)
        .padding(Padding { top: 12.0, right: 14.0, bottom: 12.0, left: 16.0 })
        .width(w)
        .height(h)
        .style(live_style(|t| t.accent, app.card_hot(part.key), app.is_flashing(part.key)));
    hover_wrap(part, c.into())
}

/// A slim horizontal level bar with peak hold (inside a callout card).
pub struct SlimMeter {
    level: f64,
    peak: f64,
    floor: f64,
    tokens: Tokens,
    clip: bool,
}

impl<M> iced::widget::canvas::Program<M> for SlimMeter {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &iced::Theme,
        bounds: iced::Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path};
        use iced::{Point, Size};
        let t = &self.tokens;
        let mut f = Frame::new(renderer, bounds.size());
        let h = bounds.height;
        f.fill(&Path::rounded_rectangle(Point::ORIGIN, bounds.size(), (h / 2.0).into()), t.surface_hover);
        let fr = super::stage::fraction(self.level, self.floor);
        if fr > 0.0 {
            f.fill(
                &Path::rounded_rectangle(Point::ORIGIN, Size::new((bounds.width * fr).max(h), h), (h / 2.0).into()),
                super::stage::level_color(t, self.level),
            );
        }
        let pf = super::stage::fraction(self.peak, self.floor);
        if pf > 0.0 {
            let x = (bounds.width * pf - 2.0).max(0.0);
            f.fill_rectangle(Point::new(x, 0.0), Size::new(2.0, h), super::stage::level_color(t, self.peak));
        }
        if self.clip {
            // latched clip: the end of the bar turns red
            let cap = (h * 3.0).min(bounds.width);
            f.fill(&Path::rounded_rectangle(Point::new(bounds.width - cap, 0.0), Size::new(cap, h), (h / 2.0).into()), t.bad);
        }
        vec![f.into_geometry()]
    }
}

/// Card meter: one slim bar per channel (L above R for a stereo pair).
pub fn slim_meter<'a>(app: &App, src: crate::models::MeterSource, t: Tokens) -> Element<'a, Message> {
    let mut col = column![].spacing(3);
    for i in src.indices() {
        col = col.push(
            iced::widget::canvas(SlimMeter {
                level: app.meter_level(i),
                peak: app.meter_peak(i),
                floor: app.model.meters.floor_db,
                tokens: t,
                clip: app.is_clipping(i),
            })
                .width(Length::Fill)
                .height(crate::layout::CARD_BAR as f32),
        );
    }
    col.into()
}

/// Red "CLIP" pill for a meter source while it is latched; clicking clears it.
pub fn clip_pill<'a>(app: &App, src: crate::models::MeterSource, t: Tokens) -> Option<Element<'a, Message>> {
    let idx = src.indices();
    let names: Vec<&'static str> = if idx.len() == 2 { vec!["L", "R"] } else { vec![""] };
    clip_pill_for(app, &idx, &names, t)
}

fn clip_pill_for<'a>(app: &App, idx: &[usize], names: &[&'static str], t: Tokens) -> Option<Element<'a, Message>> {
    let on: Vec<&str> = idx.iter().zip(names.iter().chain(std::iter::repeat(&""))).filter(|(i, _)| app.is_clipping(**i)).map(|(_, n)| *n).collect();
    if on.is_empty() {
        return None;
    }
    let mut label = String::from("CLIP");
    for n in on.iter().filter(|n| !n.is_empty()) {
        label.push(' ');
        label.push_str(n);
    }
    let pill = button(text(label).size(9).font(theme::BOLD))
        .padding([2, 7])
        .on_press(Message::ClearClip(idx.to_vec()))
        .style(theme::pill_toggle(true, |t| t.bad));
    let _ = t;
    Some(tooltip(pill, tip_bubble(CLIP_TIP), tooltip::Position::Bottom).into())
}

const CLIP_TIP: &str = "The level reached full scale. Turn the GAIN or the source down. Click to clear.";
