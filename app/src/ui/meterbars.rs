//! Slim rounded peak meters (label, bar, dBFS value), used by the meter card
//! and the loopback panel.

use super::theme::{self, Tokens};
use crate::layout::{METER_ROW_H, meter_bars_height};
use crate::levels::format_dbfs;
use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Frame, Geometry, Path, Text};
use iced::widget::{canvas as canvas_widget, container, text, tooltip};
use iced::{Element, Length, Point, Rectangle, Size, mouse};

pub struct MeterBars {
    pub labels: Vec<&'static str>,
    pub values: Vec<f64>,
    pub floor: f64,
    pub tokens: Tokens,
    /// latched clip per row: the bar's end turns red
    pub clips: Vec<bool>,
}

impl<M> canvas::Program<M> for MeterBars {
    type State = ();

    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &iced::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let t = &self.tokens;
        let mut f = Frame::new(renderer, bounds.size());
        let (lab_w, val_w) = (14.0, 64.0);
        let row = METER_ROW_H as f32;
        for (i, lab) in self.labels.iter().enumerate() {
            let v = self.values.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let cy = i as f32 * row + 7.0;
            f.fill_text(Text {
                content: lab.to_string(),
                position: Point::new(0.0, cy),
                color: t.muted,
                size: 11.0.into(),
                font: theme::SEMIBOLD,
                align_y: Vertical::Center,
                ..Text::default()
            });
            let well = Rectangle::new(Point::new(lab_w + 4.0, cy - 3.0), Size::new(bounds.width - lab_w - val_w - 12.0, 6.0));
            f.fill(&Path::rounded_rectangle(well.position(), well.size(), 3.0.into()), t.surface_hover);
            let frac = ((v - self.floor) / (0.0 - self.floor)).clamp(0.0, 1.0) as f32;
            if frac > 0.0 {
                let col = if v > -1.0 {
                    t.bad
                } else if v > -6.0 {
                    t.warn
                } else {
                    t.accent
                };
                let w = (well.width * frac).max(6.0);
                f.fill(&Path::rounded_rectangle(well.position(), Size::new(w, well.height), 3.0.into()), col);
            }
            if self.clips.get(i).copied().unwrap_or(false) {
                let cap = 10.0f32.min(well.width);
                f.fill(
                    &Path::rounded_rectangle(Point::new(well.x + well.width - cap, well.y), Size::new(cap, well.height), 3.0.into()),
                    t.bad,
                );
            }
            f.fill_text(Text {
                content: format_dbfs(v, self.floor),
                position: Point::new(bounds.width, cy),
                color: t.text,
                size: 12.0.into(),
                font: theme::MEDIUM,
                align_x: Horizontal::Right.into(),
                align_y: Vertical::Center,
                ..Text::default()
            });
        }
        vec![f.into_geometry()]
    }
}

pub fn bars<'a, M: 'a>(
    labels: Vec<&'static str>,
    values: Vec<f64>,
    floor: f64,
    tokens: Tokens,
    width: Length,
    tip: &'static str,
    clips: Vec<bool>,
) -> Element<'a, M> {
    let rows = labels.len();
    let c = canvas_widget(MeterBars { labels, values, floor, tokens, clips }).width(width).height(meter_bars_height(rows) as f32);
    tooltip(c, tip_bubble(tip), tooltip::Position::Bottom).into()
}

pub fn tip_bubble<'a, M: 'a>(tip: &'a str) -> Element<'a, M> {
    container(text(tip).size(13).line_height(1.4))
        .padding([8, 12])
        .max_width(380)
        .style(theme::tooltip_box)
        .into()
}
