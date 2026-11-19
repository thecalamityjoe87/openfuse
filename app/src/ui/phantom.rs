//! Confirmation before enabling 48 V phantom power: a modal sheet over the
//! window. Cancel is the default (focused) button; Escape cancels, Enter
//! activates the focused button, Tab / arrows move the focus.

use super::glyphs::{Glyph, Tint, glyph};
use super::theme::{self, alpha};
use super::{App, ConfirmButton, Message};
use iced::widget::{button, center, column, container, mouse_area, opaque, rich_text, row, space, span, text};
use iced::{Alignment, Border, Color, Element, Length, Shadow, Vector, mouse};

pub const TITLE: &str = "Turn on 48 V phantom power?";
pub const CANCEL: &str = "Cancel";
pub const ENABLE: &str = "Turn on 48 V";
pub const DONT_ASK: &str = "Don't ask again";

fn dialog_button<'a>(label: &'a str, primary: bool, focused: bool, msg: Message) -> Element<'a, Message> {
    button(text(label).size(15).font(theme::MEDIUM))
        .padding([11, 22])
        .on_press(msg)
        .style(move |th, st| {
            let t = theme::tokens(th);
            let mut s = if primary { theme::danger(th, st) } else { theme::secondary(th, st) };
            if focused {
                // visible keyboard focus: the default button carries a ring
                s.border = Border { color: if primary { t.bad } else { t.accent }, width: 2.0, radius: 10.0.into() };
                s.shadow = Shadow { color: alpha(if primary { t.bad } else { t.accent }, 0.22), offset: Vector::ZERO, blur_radius: 0.0 };
            }
            s
        })
        .into()
}

pub fn dialog(app: &App) -> Element<'_, Message> {
    let t = *app.tokens();
    let focus = app.confirm.unwrap_or(ConfirmButton::Cancel);
    let body: iced::widget::text::Rich<'_, (), Message> = rich_text![
        span("This sends 48 volts to "),
        span("both").font(theme::SEMIBOLD).color(t.text),
        span(
            " inputs. Condenser microphones need it, but it can damage some ribbon and vintage microphones \
             and some line-level gear.\n\nConnect the microphone first and turn your speakers down — switching \
             phantom power can cause a loud pop."
        ),
    ]
    .size(15)
    .line_height(1.45)
    .color(t.muted);
    let badge = container(glyph(Glyph::Warn, Tint::Fixed(t.bad), 26.0))
        .center_x(56)
        .center_y(56)
        .style(theme::badge(t.bad, 0.14));
    let dont_ask = iced::widget::checkbox(app.confirm_dont_ask)
        .label(DONT_ASK)
        .on_toggle(Message::ConfirmDontAsk)
        .size(16)
        .spacing(8)
        .text_size(14)
        .style(theme::check);
    let buttons = row![
        dont_ask,
        space::horizontal(),
        dialog_button(CANCEL, false, focus == ConfirmButton::Cancel, Message::Confirm(false)),
        dialog_button(ENABLE, true, focus == ConfirmButton::Enable, Message::Confirm(true)),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let sheet = container(
        column![badge, text(TITLE).size(24).font(theme::BOLD).color(t.text), body, buttons]
            .spacing(14)
            .width(Length::Fill),
    )
    .padding(28)
    .width(500)
    .style(move |_| container::Style {
        background: Some(t.bg.into()),
        border: Border { color: t.border, width: 1.0, radius: 16.0.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.28), offset: Vector::new(0.0, 12.0), blur_radius: 40.0 },
        ..Default::default()
    });
    let radius = app.corner_radius();
    // clicking outside the sheet does nothing (no accidental confirm or cancel)
    opaque(
        mouse_area(center(opaque(sheet)).style(move |_| container::Style {
            background: Some(Color::from_rgba(0.04, 0.05, 0.08, if t.dark { 0.55 } else { 0.32 }).into()),
            border: Border { radius: radius.into(), ..Default::default() },
            ..Default::default()
        }))
        .interaction(mouse::Interaction::Idle),
    )
}
