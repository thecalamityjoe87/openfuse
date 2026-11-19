//! Tiny 2-D geometry helpers (layout, hit testing, layout tests).

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn center(&self) -> Pt {
        Pt::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    /// Grow (positive) or shrink (negative) on every side.
    pub fn inflate(&self, d: f64) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    pub fn contains_rect(&self, o: &Rect) -> bool {
        o.x >= self.x - 1e-9
            && o.y >= self.y - 1e-9
            && o.right() <= self.right() + 1e-9
            && o.bottom() <= self.bottom() + 1e-9
    }
    /// Overlap with a non-zero area.
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
}

/// Do segments a-b and c-d cross (proper or touching intersection)?
pub fn segments_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    fn orient(p: Pt, q: Pt, r: Pt) -> f64 {
        (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
    }
    fn on_seg(p: Pt, q: Pt, r: Pt) -> bool {
        r.x >= p.x.min(q.x) - 1e-9
            && r.x <= p.x.max(q.x) + 1e-9
            && r.y >= p.y.min(q.y) - 1e-9
            && r.y <= p.y.max(q.y) + 1e-9
    }
    let d1 = orient(c, d, a);
    let d2 = orient(c, d, b);
    let d3 = orient(a, b, c);
    let d4 = orient(a, b, d);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return true;
    }
    (d1 == 0.0 && on_seg(c, d, a))
        || (d2 == 0.0 && on_seg(c, d, b))
        || (d3 == 0.0 && on_seg(a, b, c))
        || (d4 == 0.0 && on_seg(a, b, d))
}

/// Does segment a-b pass through the rectangle (sampled along its length)?
pub fn segment_hits_rect(a: Pt, b: Pt, r: &Rect, steps: usize) -> bool {
    (0..=steps).any(|k| {
        let t = k as f64 / steps as f64;
        r.contains(Pt::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t))
    })
}
