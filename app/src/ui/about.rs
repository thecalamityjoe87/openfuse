//! About dialog: a modal sheet over the window, opened from the header's
//! "i" button. Escape, Enter or the Close button dismiss it.

use super::theme;
use super::{App, Message};
use iced::alignment::Horizontal;
use iced::widget::{button, center, column, container, image, mouse_area, opaque, text};
use iced::{Border, Color, Element, Length, Shadow, Vector, mouse};

pub const NAME: &str = "OpenFuse";
pub const TAGLINE: &str = "An open-source control panel for the Arturia MiniFuse on Linux";
pub const CREDIT: &str = "Isaac Joseph, Calamity Audio";
pub const LICENSE: &str = "Free software. Clean-room, independent project; not affiliated with Arturia.";
pub const NOTICES: &str = "Arturia and MiniFuse are trademarks of Arturia.\nInterface typeface: Inter (SIL Open Font License 1.1).";

pub fn version() -> String {
    format!("Version {}", env!("CARGO_PKG_VERSION"))
}

fn line<'a>(s: impl text::IntoFragment<'a>, size: u32) -> iced::widget::Text<'a> {
    text(s).size(size).width(Length::Fill).align_x(Horizontal::Center)
}

pub fn dialog(app: &App) -> Element<'_, Message> {
    let t = *app.tokens();
    let sheet = container(
        column![
            image(app.logo.clone()).width(84).height(84),
            line(NAME, 28).font(theme::BOLD).color(t.text),
            line(version(), 13).color(t.muted),
            line(TAGLINE, 15).color(t.text),
            container(iced::widget::space::vertical().height(1)).width(Length::Fill).height(1).style(theme::divider),
            line("CREATED BY", 11).font(theme::SEMIBOLD).color(t.muted),
            line(CREDIT, 19).font(theme::SEMIBOLD).color(t.text),
            line(LICENSE, 13).color(t.muted),
            line(NOTICES, 12).color(theme::alpha(t.muted, 0.85)),
            container(
                button(text("Close").size(15).font(theme::MEDIUM))
                    .padding([10, 34])
                    .on_press(Message::SetAbout(false))
                    .style(theme::primary)
            )
            .width(Length::Fill)
            .center_x(Length::Fill),
        ]
        .spacing(12)
        .align_x(iced::Alignment::Center)
        .width(Length::Fill),
    )
    .padding(32)
    .width(440)
    .style(move |_| container::Style {
        background: Some(t.bg.into()),
        border: Border { color: t.border, width: 1.0, radius: 18.0.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.28), offset: Vector::new(0.0, 12.0), blur_radius: 40.0 },
        ..Default::default()
    });
    let radius = app.corner_radius();
    // a click on the dimmed backdrop closes it; a click on the sheet does not
    opaque(
        mouse_area(center(opaque(sheet)).style(move |_| container::Style {
            background: Some(Color::from_rgba(0.04, 0.05, 0.08, if t.dark { 0.55 } else { 0.32 }).into()),
            border: Border { radius: radius.into(), ..Default::default() },
            ..Default::default()
        }))
        .on_press(Message::SetAbout(false))
        .interaction(mouse::Interaction::Idle),
    )
}
