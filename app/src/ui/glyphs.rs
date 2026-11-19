//! Small line glyphs drawn as canvas vector paths (window controls, warning
//! sign, close cross). SVG icons are rasterised and resampled at fractional
//! offsets, which makes thin straight lines wobbly; paths stay crisp.

use super::theme;
use iced::widget::canvas::{self, Frame, Geometry, LineCap, LineJoin, Path, Stroke};
use iced::{Color, Element, Point, Rectangle, Size, mouse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
    /// warning triangle
    Warn,
    /// panel close cross (smaller than the window one)
    Cross,
}

#[derive(Debug, Clone, Copy)]
pub enum Tint {
    /// window controls: muted, text colour on hover, white on the red close hover
    Chrome,
    Fixed(Color),
}

pub struct GlyphCanvas {
    pub glyph: Glyph,
    pub tint: Tint,
}

impl<M> canvas::Program<M> for GlyphCanvas {
    type State = ();

    fn draw(&self, _: &(), renderer: &iced::Renderer, th: &iced::Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let t = theme::tokens(th);
        let hovered = cursor.is_over(bounds);
        let color = match self.tint {
            Tint::Fixed(c) => c,
            Tint::Chrome => match (self.glyph, hovered) {
                (Glyph::Close, true) => Color::WHITE,
                (_, true) => t.text,
                _ => t.muted,
            },
        };
        let mut f = Frame::new(renderer, bounds.size());
        let c = f.center();
        let s = Stroke::default().with_width(1.6).with_color(color).with_line_cap(LineCap::Round).with_line_join(LineJoin::Round);
        let square = |x: f32, y: f32, d: f32| Path::rounded_rectangle(Point::new(x, y), Size::new(d, d), 2.0.into());
        match self.glyph {
            Glyph::Minimize => f.stroke(&Path::line(Point::new(c.x - 5.5, c.y), Point::new(c.x + 5.5, c.y)), s),
            Glyph::Maximize => f.stroke(&square(c.x - 4.5, c.y - 4.5, 9.0), s),
            Glyph::Restore => {
                f.stroke(&square(c.x - 5.5, c.y - 3.5, 8.0), s);
                f.stroke(
                    &Path::new(|b| {
                        b.move_to(Point::new(c.x - 2.5, c.y - 5.5));
                        b.line_to(Point::new(c.x + 3.5, c.y - 5.5));
                        b.arc_to(Point::new(c.x + 5.5, c.y - 5.5), Point::new(c.x + 5.5, c.y - 3.5), 2.0);
                        b.line_to(Point::new(c.x + 5.5, c.y + 2.5));
                    }),
                    s,
                );
            }
            Glyph::Close | Glyph::Cross => {
                let d = if self.glyph == Glyph::Close { 4.75 } else { 4.0 };
                f.stroke(&Path::line(Point::new(c.x - d, c.y - d), Point::new(c.x + d, c.y + d)), s);
                f.stroke(&Path::line(Point::new(c.x + d, c.y - d), Point::new(c.x - d, c.y + d)), s);
            }
            Glyph::Warn => {
                // triangle with an exclamation mark, fitted to the frame
                let h = bounds.height.min(bounds.width);
                let r = h / 2.0 - 1.0;
                let s = Stroke { width: (h / 9.0).max(1.2), ..s };
                f.stroke(
                    &Path::new(|b| {
                        b.move_to(Point::new(c.x, c.y - r));
                        b.line_to(Point::new(c.x + r * 1.08, c.y + r * 0.85));
                        b.line_to(Point::new(c.x - r * 1.08, c.y + r * 0.85));
                        b.close();
                    }),
                    s,
                );
                f.stroke(&Path::line(Point::new(c.x, c.y - r * 0.35), Point::new(c.x, c.y + r * 0.2)), s);
                f.fill(&Path::circle(Point::new(c.x, c.y + r * 0.55), s.width * 0.6), color);
            }
        }
        vec![f.into_geometry()]
    }
}

pub fn glyph<'a, M: 'a>(glyph: Glyph, tint: Tint, size: f32) -> Element<'a, M> {
    iced::widget::canvas(GlyphCanvas { glyph, tint }).width(size).height(size).into()
}
