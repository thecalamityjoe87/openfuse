//! The device stage: the live render with callout leaders (a canvas), the
//! callout cards pinned around it and the loopback panel.

use super::{App, Message, cards, loopback, theme::{Tokens, alpha}};
use crate::geom::{Pt, Rect};
use crate::layout::StageLayout;
use crate::models::Kind;
use iced::widget::canvas::{self, Frame, Geometry, LineCap, LineDash, LineJoin, Path, Stroke};
use iced::widget::{canvas as canvas_widget, pin, stack};
use iced::{Color, Element, Length, Point, Rectangle, Size, mouse};

pub fn stage(app: &App, size: Size) -> Element<'_, Message> {
    let lay = app.layout_for(size);
    let mut layers: Vec<Element<'_, Message>> = Vec::new();
    // images (render + lit parts) first, vector overlays (meters, leaders)
    // in a layer above: iced draws a layer's images after its shapes
    layers.push(canvas_widget(RenderCanvas { app, layout: lay.clone() }).width(Length::Fill).height(Length::Fill).into());
    layers.push(canvas_widget(StageCanvas { app, layout: lay.clone() }).width(Length::Fill).height(Length::Fill).into());
    for c in &lay.cards {
        layers.push(pin(cards::card(app, c.part, c.rect.w as f32, c.rect.h as f32)).x(c.rect.x as f32).y(c.rect.y as f32).into());
    }
    if let (Some(p), Some(spec)) = (lay.panel, app.model.loopback) {
        layers.push(pin(loopback::panel(app, spec, p.w as f32)).x(p.x as f32).y(p.y as f32).into());
    }
    stack(layers).width(Length::Fill).height(Length::Fill).into()
}

/// The device render with its lit parts (images only).
pub struct RenderCanvas<'a> {
    app: &'a App,
    layout: StageLayout,
}

impl canvas::Program<Message> for RenderCanvas<'_> {
    type State = ();

    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &iced::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let (app, lay) = (self.app, &self.layout);
        let mut frame = Frame::new(renderer, bounds.size());
        let dev = lay.device;
        // a soft accent glow behind the device (many faint rounded rects, so no
        // visible steps), never larger than the room around the device
        let t = app.tokens();
        let core = Rectangle { x: dev.x as f32 + dev.w as f32 * 0.03, y: dev.y as f32 + dev.h as f32 * 0.16, width: dev.w as f32 * 0.94, height: dev.h as f32 * 0.68 };
        let room = core.x.min(core.y).min(bounds.width - core.x - core.width).min(bounds.height - core.y - core.height);
        let spread = (room - 2.0).clamp(0.0, 84.0);
        const LAYERS: usize = 28;
        for i in (0..LAYERS).rev() {
            let g = 4.0 + (spread - 4.0).max(0.0) * (i as f32 + 1.0) / LAYERS as f32;
            let path = Path::rounded_rectangle(
                Point::new(core.x - g, core.y - g),
                Size::new(core.width + 2.0 * g, core.height + 2.0 * g),
                (22.0 + g).into(),
            );
            frame.fill(&path, alpha(t.accent, if t.dark { 0.0085 } else { 0.0060 }));
        }
        frame.draw_image(rect(&dev), canvas::Image::new(app.base_image(&dev)));
        for id in app.lit_elements() {
            if let Some((h, r)) = app.sprite(&id, &dev) {
                frame.draw_image(rect(&lay.mapper.rect(&r)), canvas::Image::new(h));
            }
        }
        vec![frame.into_geometry()]
    }
}

pub struct StageCanvas<'a> {
    app: &'a App,
    layout: StageLayout,
}

fn p(pt: Pt) -> Point {
    Point::new(pt.x as f32, pt.y as f32)
}

fn rect(r: &Rect) -> Rectangle {
    Rectangle { x: r.x as f32, y: r.y as f32, width: r.w as f32, height: r.h as f32 }
}

fn polyline(pts: &[Pt]) -> Path {
    Path::new(|b| {
        b.move_to(p(pts[0]));
        for q in &pts[1..] {
            b.line_to(p(*q));
        }
    })
}

fn stroke<'a>(color: Color, width: f32, dash: &'a [f32]) -> Stroke<'a> {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        line_dash: LineDash { segments: dash, offset: 0 },
        ..Stroke::default().with_color(color)
    }
}

const DASH: &[f32] = &[3.0, 3.0];

impl StageCanvas<'_> {
    fn draw_ring(&self, frame: &mut Frame, t: &Tokens, kind: Kind, r: Rect) {
        let r = r.inflate(6.0);
        let rad = (r.w.min(r.h) / 2.0) as f32;
        let path = Path::rounded_rectangle(p(Pt::new(r.x, r.y)), Size::new(r.w as f32, r.h as f32), rad.into());
        match kind {
            Kind::Toggle | Kind::Loopback | Kind::Meter => frame.stroke(&path, stroke(t.accent, 2.0, &[])),
            Kind::Phantom => frame.stroke(&path, stroke(t.bad, 2.0, &[])),
            _ => frame.stroke(&path, stroke(t.inert(), 1.4, DASH)),
        }
    }
}

/// Pointer state of the stage.
#[derive(Default)]
pub struct StageState {
    /// last hovered part, so hover messages are only sent on change
    hover: Option<&'static str>,
    /// a knob being dragged: (part, cursor y at the start, position at the start)
    drag: Option<(&'static str, f32, f64)>,
    /// last press on a knob, for double-click
    last_press: Option<(&'static str, std::time::Instant)>,
}

/// Cursor travel for a knob's full range, in px.
const KNOB_DRAG_PX: f32 = 220.0;

impl canvas::Program<Message> for StageCanvas<'_> {
    type State = StageState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let at = || cursor.position_in(bounds).and_then(|q| self.layout.part_at(Pt::new(q.x as f64, q.y as f64)));
        let knob_at = || at().filter(|part| self.app.is_knob(part.key));
        match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                if let Some((key, y0, pos0)) = state.drag {
                    let y = cursor.position()?.y;
                    let pos = (pos0 + f64::from(y0 - y) / f64::from(KNOB_DRAG_PX)).clamp(0.0, 1.0);
                    return Some(canvas::Action::publish(Message::SetKnob(key, pos)).and_capture());
                }
                let key = at().map(|part| part.key);
                if key != state.hover {
                    state.hover = key;
                    return Some(canvas::Action::publish(Message::HoverPart(key)));
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => match at() {
                Some(part) if self.app.is_knob(part.key) => {
                    let now = std::time::Instant::now();
                    let double = state.last_press.is_some_and(|(k, t)| k == part.key && now.duration_since(t).as_millis() < 400);
                    state.last_press = Some((part.key, now));
                    if double {
                        state.drag = None;
                        state.last_press = None;
                        return Some(canvas::Action::publish(Message::ResetKnob(part.key)).and_capture());
                    }
                    state.drag = cursor.position().map(|q| (part.key, q.y, self.app.knob_start(part.key)));
                    Some(canvas::Action::publish(Message::PartClicked(part.key)).and_capture())
                }
                Some(part) => Some(canvas::Action::publish(Message::PartClicked(part.key)).and_capture()),
                // empty stage: move the window (no title bar of its own)
                None if cursor.position_in(bounds).is_some() => Some(canvas::Action::publish(Message::Drag)),
                None => None,
            },
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag.take().map(|_| canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let part = knob_at()?;
                let step = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => f64::from(*y) * 0.04,
                    mouse::ScrollDelta::Pixels { y, .. } => f64::from(*y) / 400.0,
                };
                let pos = (self.app.knob_start(part.key) + step).clamp(0.0, 1.0);
                Some(canvas::Action::publish(Message::SetKnob(part.key, pos)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let app = self.app;
        let t = app.tokens();
        let lay = &self.layout;
        let mut frame = Frame::new(renderer, bounds.size());

        // knob pointers, at the angle each knob is shown at (see App::knob_shown);
        // the GAIN pointers are also the input meters: grey without signal,
        // blue rising with level, red while clipping
        let render = app.render(lay.view);
        for (id, ptr) in &render.pointers {
            let shown = app.knob_shown(id);
            let mut color = match app.knob_light(id) {
                Some(super::KnobLight::Signal(b)) => {
                    let (off, on) = (Color::from_rgb8(0xb4, 0xb9, 0xbf), Color::from_rgb8(0x2f, 0xa4, 0xf5));
                    let k = 0.35 + 0.65 * b;
                    Color::from_rgb(off.r + (on.r - off.r) * k, off.g + (on.g - off.g) * k, off.b + (on.b - off.b) * k)
                }
                Some(super::KnobLight::Clip) => Color::from_rgb8(0xe5, 0x48, 0x4d),
                Some(super::KnobLight::Off) => Color::from_rgb8(0xb4, 0xb9, 0xbf),
                None => Color::from_rgb8(ptr.color[0], ptr.color[1], ptr.color[2]),
            };
            if shown.is_some_and(|k| k.source == super::KnobSource::Held) {
                color = alpha(color, 0.55);
            }
            let (a, b) = match shown {
                Some(k) => ptr.at_angle(crate::knobs::angle_deg(k.pos)),
                None => ((ptr.x1, ptr.y1), (ptr.x2, ptr.y2)),
            };
            let (a, b) = (lay.mapper.pt(a.0, a.1), lay.mapper.pt(b.0, b.1));
            let w = (ptr.width * lay.mapper.scale) as f32;
            frame.stroke(&Path::line(p(a), p(b)), Stroke { width: w, line_cap: LineCap::Round, ..Stroke::default().with_color(color) });
        }

        // dashed frames of the analog cards (cards themselves are widgets)
        for c in &lay.cards {
            if c.part.kind == Kind::Analog {
                let r = c.rect.inflate(-0.5);
                let path = Path::rounded_rectangle(p(Pt::new(r.x, r.y)), Size::new(r.w as f32, r.h as f32), 14.0.into());
                let col = if app.card_hot(c.part.key) { t.muted } else { t.inert() };
                frame.stroke(&path, stroke(col, 1.0, DASH));
            }
        }

        // leaders
        for l in &lay.leaders {
            let part = l.part;
            let hot = app.card_hot(part.key);
            let path = polyline(&l.points);
            let live = matches!(part.kind, Kind::Toggle | Kind::Phantom | Kind::Loopback | Kind::Meter);
            let acc = if part.kind == Kind::Phantom { t.bad } else { t.accent };
            let s = match part.kind {
                _ if live => stroke(acc, if hot { 2.2 } else { 1.5 }, &[]),
                Kind::Analog => stroke(if hot { t.muted } else { t.inert() }, 1.2, DASH),
                _ => stroke(if hot { t.muted } else { t.inert() }, 1.1, &[]),
            };
            frame.stroke(&path, s);
            if let Some(b) = &l.bracket {
                frame.stroke(&polyline(b), s);
            }
            let start = p(l.start());
            if live {
                frame.fill(&Path::circle(start, 4.5), alpha(acc, 0.25));
                frame.fill(&Path::circle(start, 3.0), acc);
            } else if l.bracket.is_none() {
                frame.fill(&Path::circle(start, 2.6), if part.kind == Kind::Analog { t.bg } else { t.inert() });
                frame.stroke(&Path::circle(start, 2.6), stroke(t.inert(), 1.2, &[]));
            }
            if hot && let Some(r) = lay.part_rect(part.key) {
                self.draw_ring(&mut frame, t, part.kind, r);
            }
        }

        // connector from the USB-C card to the open loopback panel
        if let Some(panel) = lay.panel
            && let Some(card) = lay.cards.iter().find(|c| c.part.kind == Kind::Loopback)
        {
            let a = Point::new(card.rect.center().x as f32, card.rect.bottom() as f32 + 1.0);
            let b = Point::new(a.x, panel.y as f32);
            frame.stroke(&Path::line(a, b), stroke(t.accent, 1.4, &[]));
            frame.fill(&Path::circle(b, 3.2), t.accent);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &Self::State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        let over = cursor.position_in(bounds).and_then(|q| self.layout.part_at(Pt::new(q.x as f64, q.y as f64)));
        match over {
            _ if state.drag.is_some() => mouse::Interaction::Grabbing,
            Some(part) if part.interactive() => mouse::Interaction::Pointer,
            Some(part) if self.app.is_knob(part.key) => mouse::Interaction::Grab,
            _ => mouse::Interaction::None,
        }
    }
}

/// Colour of a level: accent, warn near full scale, bad at clip.
pub fn level_color(t: &Tokens, db: f64) -> Color {
    if db > -1.0 {
        t.bad
    } else if db > -6.0 {
        t.warn
    } else {
        t.accent
    }
}

/// Fraction of a meter for a level (floor at the model's meter floor).
pub fn fraction(db: f64, floor: f64) -> f32 {
    ((db - floor) / (0.0 - floor)).clamp(0.0, 1.0) as f32
}

