//! Callout layout around the device render (pure geometry, no toolkit).
//!
//! Callout cards sit in a row above and/or below the render (side comes from
//! the model table), spread horizontally so they stay in the same order as the
//! parts they point to. Leader lines leave the part (straight up/down, or
//! sideways first where a printed legend sits under a button), run to the
//! edge of the render and then to the card. Because cards keep the parts'
//! order, leaders never cross. `tests` checks this for every model, view and
//! three window sizes.

use crate::geom::{Pt, Rect};
use crate::models::{Exit, Kind, MeterSource, ModelSpec, Part, Side, View};

/// Extra card height for a card meter (one bar per channel).
pub fn card_meter_height(src: Option<MeterSource>) -> f64 {
    match src {
        None => 0.0,
        Some(MeterSource::Mono(_)) => CARD_BAR + CARD_BAR_GAP + 4.0,
        Some(MeterSource::Stereo(..)) => 2.0 * CARD_BAR + 3.0 + CARD_BAR_GAP + 4.0,
    }
}

/// Width reserved for a card's CLIP pill ("CLIP" / "CLIP L R").
pub fn clip_pill_width(src: Option<MeterSource>) -> f64 {
    match src {
        None => 0.0,
        Some(MeterSource::Mono(_)) => text_width("CLIP", 9.0, 700) + 14.0,
        Some(MeterSource::Stereo(..)) => text_width("CLIP L R", 9.0, 700) + 14.0,
    }
}

pub const CARD_BAR: f64 = 5.0;
pub const CARD_BAR_GAP: f64 = 6.0;
use crate::render::DeviceRender;

pub const MARGIN: f64 = 28.0;
/// vertical room between the render and a card row
pub const LEAD_GAP: f64 = 54.0;
pub const PANEL_GAP: f64 = 28.0;
pub const CARD_GAP: f64 = 12.0;
pub const MAX_DEVICE_W: f64 = 1500.0;
pub const MAX_PANEL_W: f64 = 1180.0;

/// Window chrome around the stage (header, SIMULATED notice, status line), in px.
pub const HEADER_H: f64 = 88.0;
pub const BANNER_H: f64 = 46.0;
pub const STATUS_H: f64 = 36.0;

// Loopback panel geometry (ui/loopback.rs builds the panel to these sizes).
pub const PANEL_PAD_TOP: f64 = 18.0;
pub const PANEL_PAD_BOTTOM: f64 = 14.0;
pub const PANEL_HEAD_H: f64 = 50.0;
pub const PANEL_SPACING: f64 = 12.0;
pub const PANEL_GRID_HEAD_H: f64 = 20.0;
pub const PANEL_ROW_H: f64 = 33.0;

/// Meter bars: one row per channel.
pub const METER_ROW_H: f64 = 18.0;
pub fn meter_bars_height(rows: usize) -> f64 {
    METER_ROW_H * rows as f64 - 4.0
}

pub fn panel_height(channels: usize) -> f64 {
    PANEL_PAD_TOP + PANEL_HEAD_H + PANEL_SPACING + PANEL_GRID_HEAD_H + channels as f64 * PANEL_ROW_H + PANEL_PAD_BOTTOM
}

/// Width of a text run in px, measured with the embedded Inter font (the
/// font the UI draws with), so cards are sized to their content. Cached.
pub fn text_width(s: &str, px: f64, weight: u16) -> f64 {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Cache = Mutex<HashMap<(String, u64, u16), f64>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let key = (s.to_string(), px.to_bits(), weight);
    let cache = CACHE.get_or_init(Default::default);
    if let Some(w) = cache.lock().unwrap().get(&key) {
        return *w;
    }
    let esc = s.replace('&', "&amp;").replace('<', "&lt;");
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='4000' height='100'><text x='0' y='50' \
         font-family='Inter' font-size='{px}' font-weight='{weight}'>{esc}</text></svg>"
    );
    let opt = resvg::usvg::Options { fontdb: crate::fonts::db(), ..Default::default() };
    let w = resvg::usvg::Tree::from_str(&svg, &opt)
        .ok()
        .and_then(|t| t.root().abs_bounding_box().width().into())
        .map(|w: f32| w as f64)
        // unknown glyphs / no font: a generous estimate
        .filter(|w| *w > 0.0)
        .unwrap_or_else(|| s.chars().count() as f64 * px * 0.6);
    cache.lock().unwrap().insert(key, w);
    w
}

/// Fixed size of the callout card for a part (the UI builds cards to these sizes).
pub fn card_size(part: &Part, model: &ModelSpec) -> (f64, f64) {
    match part.kind {
        Kind::Toggle | Kind::Phantom => {
            // [title ON] / hint  ...  switch
            let left = (text_width(part.label, 14.0, 600) + 8.0 + 34.0).max(text_width(part.hint, 12.0, 400));
            let left = left.max(if part.kind == Kind::Phantom { 20.0 + text_width("Ask before turning on", 12.0, 400) } else { 0.0 });
            // the 48 V card also carries the "Ask before turning on" box
            ((16.0 + left + 16.0 + 44.0 + 14.0 + 4.0).max(200.0).ceil(), if part.kind == Kind::Phantom { 88.0 } else { 64.0 })
        }
        Kind::Meter => (228.0, 14.0 + 18.0 + 10.0 + meter_bars_height(model.meters.led_columns.len()) + 14.0),
        Kind::Analog => {
            // [title ANALOG] / hint without the "Analog · " prefix
            // the ANALOG tag gives way to the CLIP pill while clipping
            let tag = (text_width("ANALOG", 9.0, 700) + 14.0).max(clip_pill_width(model.meters.card_meter(part.key)));
            let w = (text_width(&analog_hint(part.hint), 12.0, 400) + 28.0 + 4.0)
                .max(text_width(part.label, 14.0, 600) + 6.0 + tag + 28.0 + 4.0)
                .max(110.0);
            (w.ceil(), 54.0 + card_meter_height(model.meters.card_meter(part.key)))
        }
        Kind::Connector => {
            let m = card_meter_height(model.meters.card_meter(part.key));
            let clip = clip_pill_width(model.meters.card_meter(part.key));
            let head = text_width(part.label, 12.0, 600) + if clip > 0.0 { 6.0 + clip + 16.0 } else { 0.0 };
            ((text_width(part.hint, 12.0, 400) + 20.0).max(head).max(if m > 0.0 { 110.0 } else { 100.0 }).ceil(), 44.0 + m)
        }
        Kind::Loopback => {
            let src = model.meters.card_meter(part.key);
            (264.0 + clip_pill_width(src), 82.0 + card_meter_height(src))
        }
    }
}

/// 1-D label placement. `items`: (ideal centre, width), sorted by ideal.
/// Returns the left x of each item: order kept, no overlaps, as close to
/// ideal as possible (overlapping runs are centred on their members' ideals),
/// clamped into [lo, hi].
pub fn spread(items: &[(f64, f64)], lo: f64, hi: f64, gap: f64) -> Vec<f64> {
    let n = items.len();
    if n == 0 {
        return vec![];
    }
    let width = |a: usize, b: usize| items[a..=b].iter().map(|i| i.1).sum::<f64>() + gap * (b - a) as f64;
    let best_left = |a: usize, b: usize| {
        let (mut acc, mut off) = (0.0, 0.0);
        for (c, w) in &items[a..=b] {
            acc += c - (off + w / 2.0);
            off += w + gap;
        }
        acc / (b - a + 1) as f64
    };
    let mut clusters: Vec<(usize, usize, f64)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        clusters.push((i, i, it.0 - it.1 / 2.0));
        while clusters.len() > 1 {
            let b = clusters[clusters.len() - 1];
            let a = clusters[clusters.len() - 2];
            if a.2 + width(a.0, a.1) + gap > b.2 {
                clusters.truncate(clusters.len() - 2);
                clusters.push((a.0, b.1, best_left(a.0, b.1)));
            } else {
                break;
            }
        }
    }
    let mut xs = Vec::with_capacity(n);
    for (a, b, left) in clusters {
        let mut x = left;
        for it in &items[a..=b] {
            xs.push(x);
            x += it.1 + gap;
        }
    }
    for k in 0..n {
        let lo_k = if k == 0 { lo } else { xs[k - 1] + items[k - 1].1 + gap };
        xs[k] = xs[k].max(lo_k);
    }
    for k in (0..n).rev() {
        let hi_k = if k == n - 1 { hi - items[k].1 } else { xs[k + 1] - gap - items[k].1 };
        xs[k] = xs[k].min(hi_k);
    }
    xs
}

#[derive(Debug, Clone)]
pub struct CardBox {
    pub part: &'static Part,
    pub rect: Rect,
}

#[derive(Debug, Clone)]
pub struct Leader {
    pub part: &'static Part,
    /// polyline from the part to the card
    pub points: Vec<Pt>,
    /// bracket spanning a group's members (multi-element parts)
    pub bracket: Option<Vec<Pt>>,
}

impl Leader {
    pub fn start(&self) -> Pt {
        self.points[0]
    }
    pub fn segments(&self) -> impl Iterator<Item = (Pt, Pt)> + '_ {
        self.points.windows(2).map(|w| (w[0], w[1]))
    }
}

/// Maps SVG user units to stage pixels.
#[derive(Debug, Clone, Copy)]
pub struct Mapper {
    pub viewbox: Rect,
    pub target: Rect,
    pub scale: f64,
}

impl Mapper {
    pub fn pt(&self, x: f64, y: f64) -> Pt {
        Pt::new(
            self.target.x + (x - self.viewbox.x) * self.scale,
            self.target.y + (y - self.viewbox.y) * self.scale,
        )
    }
    pub fn rect(&self, r: &Rect) -> Rect {
        let tl = self.pt(r.x, r.y);
        Rect::new(tl.x, tl.y, r.w * self.scale, r.h * self.scale)
    }
}

#[derive(Debug, Clone)]
pub struct StageLayout {
    pub width: f64,
    pub height: f64,
    pub view: View,
    /// where the render is drawn (integer pixels)
    pub device: Rect,
    pub mapper: Mapper,
    pub cards: Vec<CardBox>,
    pub leaders: Vec<Leader>,
    /// part hit areas (render bounds, 5 px slack)
    pub hits: Vec<(&'static Part, Rect)>,
    pub panel: Option<Rect>,
    /// widget-space rectangle of each visible part
    pub part_rects: Vec<(&'static Part, Rect)>,
}

impl StageLayout {
    /// The part under `p`; where enlarged hit areas overlap, the nearest one wins.
    pub fn part_at(&self, p: Pt) -> Option<&'static Part> {
        let dist = |r: &Rect| {
            let dx = (r.x - p.x).max(0.0).max(p.x - r.right());
            let dy = (r.y - p.y).max(0.0).max(p.y - r.bottom());
            dx * dx + dy * dy
        };
        self.hits
            .iter()
            .filter(|(_, r)| r.contains(p))
            .map(|(part, r)| (part, dist(&r.inflate(-HIT_SLACK))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(part, _)| *part)
    }
    pub fn part_rect(&self, key: &str) -> Option<Rect> {
        self.part_rects.iter().find(|(p, _)| p.key == key).map(|(_, r)| *r)
    }
    pub fn card(&self, key: &str) -> Option<&CardBox> {
        self.cards.iter().find(|c| c.part.key == key)
    }
}

/// SVG x at which the leader crosses the render edge.
fn lead_x(part: &Part, b: &Rect) -> f64 {
    match (part.exit, part.via_x) {
        (Some(Exit::Left | Exit::Right), Some(v)) => v,
        _ => b.center().x,
    }
}

/// The analog cards show "ANALOG" as a tag; the rest of the hint is the text.
pub fn analog_hint(hint: &str) -> String {
    let rest = hint.strip_prefix("Analog · ").unwrap_or(hint);
    let mut c = rest.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Height of the render: a moderate, centred hero that grows a little with
/// the window and is capped (the cards and leaders carry the layout).
pub fn device_height(width: f64) -> f64 {
    (205.0 + (width - 920.0) * 0.05).clamp(205.0, 240.0)
}

/// Extra slack around a part for clicking (the render is drawn small).
pub const HIT_SLACK: f64 = 10.0;

pub fn compute(
    model: &ModelSpec,
    render: &DeviceRender,
    view: View,
    width: f64,
    height: f64,
    panel_open: bool,
) -> StageLayout {
    let (w, h) = (width, height);
    let vb = render.viewbox;
    let aspect = vb.w / vb.h;
    let parts: Vec<&'static Part> = model.parts_for(view).collect();
    let sized: Vec<(&'static Part, (f64, f64))> = parts.iter().map(|p| (*p, card_size(p, model))).collect();
    let row_h = |side: Side| {
        sized.iter().filter(|(p, _)| p.side == side).map(|(_, s)| s.1).fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))))
    };
    let top_h = row_h(Side::Top);
    let bot_h = row_h(Side::Bottom);
    let show_panel = panel_open && view == View::Back && model.loopback.is_some();
    let panel_h = model.loopback.map(|lb| panel_height(lb.channels.len())).unwrap_or(0.0);

    let above = top_h.map_or(0.0, |t| t + LEAD_GAP);
    let below = bot_h.map_or(0.0, |b| b + LEAD_GAP);
    let extra = if show_panel { panel_h + PANEL_GAP } else { 0.0 };
    let avail_w = w - 2.0 * MARGIN;
    let avail_h = h - 2.0 * MARGIN - above - below - extra;
    let dev_w = avail_w.min(avail_h.min(device_height(w)) * aspect).clamp(200.0, MAX_DEVICE_W).floor();
    let dev_h = (dev_w / aspect).round();
    let total = above + dev_h + below + extra;
    let y0 = MARGIN.max((h - total) / 2.0);
    let device = Rect::new(((w - dev_w) / 2.0).round(), (y0 + above).round(), dev_w, dev_h);
    let mapper = Mapper { viewbox: vb, target: device, scale: dev_w / vb.w };

    // cards ---------------------------------------------------------------
    let mut cards = Vec::new();
    for side in [Side::Top, Side::Bottom] {
        let mut items: Vec<(f64, f64, f64, &'static Part)> = sized
            .iter()
            .filter(|(p, _)| p.side == side)
            .filter_map(|(p, (cw, ch))| {
                let b = render.bounds(p.ids)?;
                Some((mapper.pt(lead_x(p, &b), 0.0).x, *cw, *ch, *p))
            })
            .collect();
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let xs = spread(&items.iter().map(|i| (i.0, i.1)).collect::<Vec<_>>(), MARGIN, w - MARGIN, CARD_GAP);
        for (x, (_, cw, ch, p)) in xs.into_iter().zip(items) {
            let y = match side {
                Side::Top => device.y - LEAD_GAP - ch,
                Side::Bottom => device.bottom() + LEAD_GAP,
            };
            cards.push(CardBox { part: p, rect: Rect::new(x.round(), y.round(), cw, ch) });
        }
    }

    let panel = if show_panel {
        let pw = (w - 2.0 * MARGIN).min(MAX_PANEL_W);
        let py = device.bottom() + below + PANEL_GAP;
        Some(Rect::new(((w - pw) / 2.0).round(), py.round(), pw.round(), panel_h))
    } else {
        None
    };

    // leaders ---------------------------------------------------------------
    let mut leaders = Vec::new();
    let mut hits = Vec::new();
    let mut part_rects = Vec::new();
    for card in &cards {
        let part = card.part;
        let Some(b) = render.bounds(part.ids) else { continue };
        part_rects.push((part, mapper.rect(&b)));
        hits.push((part, mapper.rect(&b).inflate(HIT_SLACK)));
        let up = part.side == Side::Top;
        let sgn = if up { -1.0 } else { 1.0 };
        let pad = part.pad;
        let mut pts = Vec::new();
        let mut bracket = None;
        match (part.exit, part.via_x) {
            (Some(e @ (Exit::Left | Exit::Right)), Some(via)) => {
                let sx = if e == Exit::Right { b.right() + pad * 0.5 } else { b.x - pad * 0.5 };
                pts.push(mapper.pt(sx, b.center().y));
                pts.push(mapper.pt(via, b.center().y));
            }
            _ => {
                let ey = if up { b.y - pad } else { b.bottom() + pad };
                if part.ids.len() > 1 {
                    // bracket spanning the group's member centres
                    let centres: Vec<f64> =
                        part.ids.iter().filter_map(|i| render.element_bounds(i)).map(|r| r.center().x).collect();
                    let x0 = centres.iter().copied().fold(f64::INFINITY, f64::min);
                    let x1 = centres.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let tick = 10.0 * sgn;
                    bracket = Some(vec![
                        mapper.pt(x0, ey - tick),
                        mapper.pt(x0, ey),
                        mapper.pt(x1, ey),
                        mapper.pt(x1, ey - tick),
                    ]);
                }
                pts.push(mapper.pt(b.center().x, ey));
            }
        }
        let edge_y = if up { device.y } else { device.bottom() };
        let last = *pts.last().unwrap();
        if (last.y - edge_y) * sgn < 0.0 {
            pts.push(Pt::new(last.x, edge_y));
        }
        let r = card.rect;
        let a = Pt::new(r.center().x, if up { r.bottom() + 1.0 } else { r.y });
        let last = *pts.last().unwrap();
        pts.push(Pt::new(last.x, last.y + 10.0 * sgn));
        pts.push(Pt::new(a.x, a.y - 12.0 * sgn));
        pts.push(a);
        leaders.push(Leader { part, points: pts, bracket });
    }

    StageLayout { width: w, height: h, view, device, mapper, cards, leaders, hits, panel, part_rects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{segment_hits_rect, segments_cross};
    use crate::models::MODELS;

    #[test]
    fn spread_keeps_order_and_bounds() {
        let items = [(100.0, 80.0), (110.0, 80.0), (120.0, 80.0), (500.0, 60.0), (990.0, 100.0)];
        let xs = spread(&items, 0.0, 1000.0, 10.0);
        for k in 0..xs.len() - 1 {
            assert!(xs[k] + items[k].1 + 10.0 <= xs[k + 1] + 1e-6);
        }
        assert!(xs[0] >= 0.0 && xs[4] + items[4].1 <= 1000.0);
    }

    /// Small, default-ish and large window sizes (the stage is the window minus its chrome).
    const SIZES: [(f64, f64); 4] = [(1060.0, 600.0), (1200.0, 760.0), (1280.0, 800.0), (1920.0, 1080.0)];

    fn check(model: &ModelSpec, view: View, (ww, wh): (f64, f64), panel: bool) {
        let render = DeviceRender::new(model.svg(view));
        // the mock backend always shows the SIMULATED banner
        let (w, h) = (ww, wh - HEADER_H - BANNER_H - STATUS_H);
        let lay = compute(model, &render, view, w, h, panel);
        let tag = format!("{} {} {ww}x{wh}", model.key, view.name());
        let area = Rect::new(0.0, 0.0, w, h);
        let cards = &lay.cards;
        assert_eq!(cards.len(), model.parts_for(view).count(), "{tag}: every part has a card");
        for (i, c) in cards.iter().enumerate() {
            assert!(area.contains_rect(&c.rect), "{tag}: {} outside the view {:?}", c.part.key, c.rect);
            assert!(!c.rect.intersects(&lay.device), "{tag}: {} covers the render", c.part.key);
            for o in &cards[i + 1..] {
                assert!(!c.rect.intersects(&o.rect), "{tag}: {} overlaps {}", c.part.key, o.part.key);
            }
        }
        if let Some(p) = lay.panel {
            assert!(area.contains_rect(&p), "{tag}: panel outside the view");
            for c in cards {
                assert!(!c.rect.intersects(&p), "{tag}: {} overlaps the panel", c.part.key);
            }
        }
        // the render shows level only through its LEDs: nothing is drawn on it
        // except the leaders; every card meter belongs to a card on this view
        for (key, _) in model.meters.card_meters {
            assert!(model.part(key).is_some(), "{tag}: card meter for unknown part {key}");
        }
        for l in &lay.leaders {
            let key = l.part.key;
            for (other, r) in &lay.part_rects {
                if other.key != key {
                    assert!(
                        !l.segments().any(|(a, b)| segment_hits_rect(a, b, r, 60)),
                        "{tag}: leader {key} crosses part {}",
                        other.key
                    );
                }
            }
            for c in cards {
                if c.part.key != key {
                    let inner = c.rect.inflate(-1.0);
                    assert!(
                        !l.segments().any(|(a, b)| segment_hits_rect(a, b, &inner, 60)),
                        "{tag}: leader {key} crosses card {}",
                        c.part.key
                    );
                }
            }
            for o in &lay.leaders {
                if o.part.key <= key {
                    continue;
                }
                for (a, b) in l.segments() {
                    for (c, d) in o.segments() {
                        assert!(!segments_cross(a, b, c, d), "{tag}: leaders {key} and {} cross", o.part.key);
                    }
                }
            }
        }
    }

    #[test]
    fn layout_clean_for_every_model_view_and_size() {
        let mut n = 0;
        for model in MODELS {
            for view in [View::Front, View::Back] {
                for size in SIZES {
                    check(model, view, size, false);
                    n += 1;
                }
            }
        }
        assert_eq!(n, MODELS.len() * 2 * SIZES.len());
    }

    #[test]
    fn layout_clean_with_loopback_panel_open() {
        for size in [(1200.0, 760.0), (1280.0, 800.0), (1920.0, 1080.0)] {
            check(&crate::models::MINIFUSE2, View::Back, size, true);
        }
    }

    #[test]
    fn hit_testing_finds_parts() {
        let m = &crate::models::MINIFUSE2;
        let render = DeviceRender::new(m.svg(View::Front));
        let lay = compute(m, &render, View::Front, 1280.0, 674.0, false);
        let r = lay.part_rect("phantom-48v").unwrap();
        assert_eq!(lay.part_at(r.center()).unwrap().key, "phantom-48v");
        assert!(lay.part_at(Pt::new(1.0, 1.0)).is_none());
    }
}
