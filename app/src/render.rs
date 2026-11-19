//! Live device render.
//!
//! Each embedded SVG is parsed twice, as an "everything off" and an
//! "everything lit" variant (button lenses dark/lit with a halo, LEDs
//! hollow/filled with a halo). The off variant is rasterised once per size
//! and used as the background; the live parts that are currently on are then
//! drawn on top as small sprites rasterised from the lit variant. Nothing is
//! re-parsed or re-rasterised while the meters animate.

use resvg::tiny_skia;
use resvg::usvg;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::geom::Rect;

const LENS_OFF_FILL: &str = "#e3e6e9";
const LENS_OFF_RIM: &str = "#c9ced3";
/// An unlit knob light (no signal)
pub const POINTER_OFF: &str = "#b4b9bf";

/// The pointer line of a knob, in SVG user units. The base image has no
/// pointers: the UI draws them, at the angle the knob is shown at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub width: f64,
    /// the knob's center
    pub cx: f64,
    pub cy: f64,
    /// stroke color as drawn in the source SVG
    pub color: [u8; 3],
}

impl Pointer {
    /// Distance from the center to the pointer's inner and outer end.
    pub fn radii(&self) -> (f64, f64) {
        let d = |x: f64, y: f64| ((x - self.cx).powi(2) + (y - self.cy).powi(2)).sqrt();
        (d(self.x1, self.y1), d(self.x2, self.y2))
    }

    /// The pointer's angle as drawn in the source SVG, degrees clockwise from 3 o'clock.
    pub fn angle_deg(&self) -> f64 {
        (self.y2 - self.cy).atan2(self.x2 - self.cx).to_degrees()
    }

    /// The pointer line at `deg`, as (inner end, outer end).
    pub fn at_angle(&self, deg: f64) -> ((f64, f64), (f64, f64)) {
        let (r1, r2) = self.radii();
        let a = deg.to_radians();
        ((self.cx + a.cos() * r1, self.cy + a.sin() * r1), (self.cx + a.cos() * r2, self.cy + a.sin() * r2))
    }
}

fn hex_rgb(v: &str) -> [u8; 3] {
    let h = v.trim().trim_start_matches('#');
    let n = u32::from_str_radix(h, 16).unwrap_or(0xb4b9bf);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

/// Mean of a polygon's points: the center of a regular polygon (a knob).
fn polygon_center(g: &roxmltree::Node) -> Option<(f64, f64)> {
    let pts = g.descendants().find(|n| n.has_tag_name("polygon"))?.attribute("points")?;
    let v: Vec<(f64, f64)> = pts
        .split_whitespace()
        .filter_map(|p| p.split_once(',').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?))))
        .collect();
    (!v.is_empty()).then(|| (v.iter().map(|p| p.0).sum::<f64>() / v.len() as f64, v.iter().map(|p| p.1).sum::<f64>() / v.len() as f64))
}

/// RGBA8 (premultiplied alpha removed) image, ready for the UI.
#[derive(Clone)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct DeviceRender {
    off: usvg::Tree,
    on: usvg::Tree,
    pub buttons: HashSet<String>,
    pub leds: HashSet<String>,
    /// The SVG viewBox, in user units.
    pub viewbox: Rect,
    /// canvas units (usvg) -> user units
    unit: f64,
    bounds: HashMap<String, Rect>,
    /// printed legends ("GAIN", "INPUT" ...)
    texts: Vec<Rect>,
    /// knob light lines (user units), for knobs listed as lights
    pub pointers: HashMap<String, Pointer>,
    lit_cache: std::cell::RefCell<Option<(u32, tiny_skia::Pixmap)>>,
}

fn esc(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

/// Start tag of an element, rebuilt from its attributes with overrides.
fn start_tag(node: roxmltree::Node, set: &[(&str, &str)], self_closing: bool) -> String {
    let mut s = format!("<{}", node.tag_name().name());
    let mut done: Vec<&str> = Vec::new();
    for a in node.attributes() {
        let name = a.name();
        let val = set.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
        if val.is_some() {
            done.push(name);
        }
        let v = val.unwrap_or_else(|| a.value().to_string());
        let _ = write!(s, " {}=\"{}\"", name, esc(&v));
    }
    for (k, v) in set {
        if !done.contains(k) {
            let _ = write!(s, " {}=\"{}\"", k, esc(v));
        }
    }
    s.push_str(if self_closing { "/>" } else { ">" });
    s
}

fn classes<'a>(n: &roxmltree::Node<'a, '_>) -> Vec<&'a str> {
    n.attribute("class").map(|c| c.split_whitespace().collect()).unwrap_or_default()
}

fn is_lens(n: &roxmltree::Node) -> bool {
    n.has_tag_name("circle") && classes(n).contains(&"lens")
}

/// Scan the SVG for live elements: groups with class `btn` holding a `lens`
/// circle, and groups with class `led`.
fn scan(doc: &roxmltree::Document) -> (HashSet<String>, HashSet<String>) {
    let mut buttons = HashSet::new();
    let mut leds = HashSet::new();
    for g in doc.descendants().filter(|n| n.has_tag_name("g")) {
        let Some(id) = g.attribute("id") else { continue };
        let cls = classes(&g);
        if cls.contains(&"btn") && g.descendants().any(|c| is_lens(&c)) {
            buttons.insert(id.to_string());
        } else if cls.contains(&"led") {
            leds.insert(id.to_string());
        }
    }
    (buttons, leds)
}

/// Build the off or lit variant of the SVG source.
pub fn variant(src: &str, lit: bool) -> String {
    variant_with(src, lit, false)
}

/// As `variant`; with `hide_pointers` the knobs' pointer lines are left out
/// (the UI draws them at the angle each knob is shown at).
pub fn variant_with(src: &str, lit: bool, hide_pointers: bool) -> String {
    let doc = roxmltree::Document::parse(src).expect("embedded SVG parses");
    let (buttons, leds) = scan(&doc);
    // (byte range to replace, replacement)
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for g in doc.descendants().filter(|n| n.has_tag_name("g")) {
        let Some(id) = g.attribute("id") else { continue };
        let r = g.range();
        let tag_end = r.start + src[r.start..].find('>').expect("tag end") + 1;
        if hide_pointers
            && let Some(ptr) = g.descendants().find(|n| n.has_tag_name("line") && classes(n).contains(&"pointer"))
        {
            edits.push((ptr.range(), String::new()));
            continue;
        }
        if buttons.contains(id) {
            let class = if lit { "btn on" } else { "btn off" };
            let lens = g.descendants().find(is_lens).expect("lens");
            let mut head = start_tag(g, &[("class", class)], false);
            if lit {
                // halo behind the bezel so a lit button reads at a glance
                let rad: f64 = lens.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(10.0);
                let _ = write!(
                    head,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{:.1}\" fill=\"{}\" fill-opacity=\"0.28\"/>",
                    lens.attribute("cx").unwrap_or("0"),
                    lens.attribute("cy").unwrap_or("0"),
                    rad * 1.55,
                    esc(lens.attribute("stroke").unwrap_or("#7cc8f0"))
                );
            } else {
                edits.push((
                    lens.range(),
                    start_tag(lens, &[("fill", LENS_OFF_FILL), ("stroke", LENS_OFF_RIM)], true),
                ));
            }
            edits.push((r.start..tag_end, head));
        } else if leds.contains(id) {
            let Some(c) = g.descendants().find(|n| n.has_tag_name("circle")) else { continue };
            let color = c.attribute("stroke").or(c.attribute("fill")).unwrap_or("#7cc8f0").to_string();
            if lit {
                let rad: f64 = c.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(6.0);
                let halo = format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{:.1}\" fill=\"{}\" fill-opacity=\"0.25\"/>",
                    c.attribute("cx").unwrap_or("0"),
                    c.attribute("cy").unwrap_or("0"),
                    rad * 1.8,
                    esc(&color)
                );
                edits.push((r.start..tag_end, format!("{}{}", start_tag(g, &[], false), halo)));
                edits.push((c.range(), start_tag(c, &[("fill", &color), ("stroke", &color)], true)));
            } else {
                edits.push((c.range(), start_tag(c, &[("fill", "none")], true)));
            }
        }
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(src.len() + edits.len() * 80);
    let mut pos = 0;
    for (r, rep) in edits {
        if r.start < pos {
            continue; // nested edits never overlap in our renders; be safe anyway
        }
        out.push_str(&src[pos..r.start]);
        out.push_str(&rep);
        pos = r.end;
    }
    out.push_str(&src[pos..]);
    out
}

fn parse(src: &str) -> usvg::Tree {
    let opt = usvg::Options {
        fontdb: crate::fonts::db(),
        font_family: crate::fonts::UI_FAMILY.to_string(),
        ..Default::default()
    };
    usvg::Tree::from_str(src, &opt).expect("embedded SVG renders")
}

fn viewbox_of(src: &str) -> Rect {
    let doc = roxmltree::Document::parse(src).expect("embedded SVG parses");
    let root = doc.root_element();
    if let Some(vb) = root.attribute("viewBox") {
        let v: Vec<f64> = vb
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if v.len() == 4 {
            return Rect::new(v[0], v[1], v[2], v[3]);
        }
    }
    let num = |a: &str| root.attribute(a).and_then(|s| s.trim_end_matches("px").parse().ok()).unwrap_or(100.0);
    Rect::new(0.0, 0.0, num("width"), num("height"))
}

fn walk<'a>(g: &'a usvg::Group, f: &mut impl FnMut(&'a usvg::Node)) {
    for n in g.children() {
        f(n);
        if let usvg::Node::Group(c) = n {
            walk(c, f);
        }
    }
}

impl DeviceRender {
    /// The base image has no knob pointers; see [`Pointer`].
    pub fn new(src: &str) -> DeviceRender {
        let doc = roxmltree::Document::parse(src).expect("embedded SVG parses");
        let (buttons, leds) = scan(&doc);
        let viewbox = viewbox_of(src);
        let mut pointers = HashMap::new();
        for g in doc.descendants().filter(|n| n.has_tag_name("g")) {
            let Some(id) = g.attribute("id") else { continue };
            if let Some(l) = g.descendants().find(|n| n.has_tag_name("line") && classes(n).contains(&"pointer"))
                && let Some((cx, cy)) = polygon_center(&g)
            {
                let f = |a: &str| l.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                pointers.insert(
                    id.to_string(),
                    Pointer {
                        x1: f("x1"),
                        y1: f("y1"),
                        x2: f("x2"),
                        y2: f("y2"),
                        width: f("stroke-width"),
                        cx,
                        cy,
                        color: hex_rgb(l.attribute("stroke").unwrap_or(POINTER_OFF)),
                    },
                );
            }
        }
        let off = parse(&variant_with(src, false, true));
        let on = parse(&variant_with(src, true, true));
        let unit = viewbox.w / off.size().width() as f64;
        let mut bounds = HashMap::new();
        let mut texts = Vec::new();
        walk(off.root(), &mut |n| {
            if let usvg::Node::Text(t) = n {
                let b = t.abs_bounding_box();
                texts.push(Rect::new(
                    viewbox.x + b.x() as f64 * unit,
                    viewbox.y + b.y() as f64 * unit,
                    b.width() as f64 * unit,
                    b.height() as f64 * unit,
                ));
            }
            let id = n.id();
            if !id.is_empty() {
                let b = n.abs_stroke_bounding_box();
                bounds.insert(
                    id.to_string(),
                    Rect::new(
                        viewbox.x + b.x() as f64 * unit,
                        viewbox.y + b.y() as f64 * unit,
                        b.width() as f64 * unit,
                        b.height() as f64 * unit,
                    ),
                );
            }
        });
        DeviceRender { off, on, buttons, leds, viewbox, unit, bounds, texts, pointers, lit_cache: Default::default() }
    }

    pub fn has(&self, id: &str) -> bool {
        self.bounds.contains_key(id)
    }

    /// Bounds of an element in SVG user units (None if missing).
    pub fn element_bounds(&self, id: &str) -> Option<Rect> {
        self.bounds.get(id).copied()
    }

    /// Union of the bounds of several elements.
    pub fn bounds(&self, ids: &[&str]) -> Option<Rect> {
        ids.iter().filter_map(|i| self.element_bounds(i)).reduce(|a, b| a.union(&b))
    }

    pub fn aspect(&self) -> f64 {
        self.viewbox.w / self.viewbox.h
    }

    /// The whole "off" render at `width` x `height` pixels.
    pub fn rasterize_base(&self, width: u32, height: u32) -> Raster {
        let mut pm = tiny_skia::Pixmap::new(width.max(1), height.max(1)).expect("pixmap");
        let sz = self.off.size();
        let t = tiny_skia::Transform::from_scale(
            width as f32 / sz.width(),
            height as f32 / sz.height(),
        );
        resvg::render(&self.off, t, &mut pm.as_mut());
        to_raster(pm)
    }

    /// The lit variant of one element for a render `width_px` pixels wide:
    /// cut from the lit render at the element's layer bounds (halo included).
    /// Returns the sprite and its rectangle in SVG user units.
    pub fn rasterize_lit(&self, id: &str, width_px: u32) -> Option<(Raster, Rect)> {
        let node = self.on.node_by_id(id)?;
        let bb = node.abs_layer_bounding_box()?;
        let sz = self.on.size();
        let s = width_px as f32 / sz.width();
        let height_px = (sz.height() * s).round().max(1.0) as u32;
        let mut cache = self.lit_cache.borrow_mut();
        if cache.as_ref().is_none_or(|(w, _)| *w != width_px) {
            let mut pm = tiny_skia::Pixmap::new(width_px.max(1), height_px)?;
            resvg::render(&self.on, tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
            *cache = Some((width_px, pm));
        }
        let full = &cache.as_ref()?.1;
        let x0 = ((bb.x() * s).floor() as i32).max(0);
        let y0 = ((bb.y() * s).floor() as i32).max(0);
        let x1 = (((bb.x() + bb.width()) * s).ceil() as i32).min(full.width() as i32);
        let y1 = (((bb.y() + bb.height()) * s).ceil() as i32).min(full.height() as i32);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let crop = full.clone_rect(tiny_skia::IntRect::from_ltrb(x0, y0, x1, y1)?)?;
        let u = self.unit / s as f64;
        let rect = Rect::new(
            self.viewbox.x + x0 as f64 * u,
            self.viewbox.y + y0 as f64 * u,
            (x1 - x0) as f64 * u,
            (y1 - y0) as f64 * u,
        );
        Some((to_raster(crop), rect))
    }

    /// Every element with an id, then every printed legend (id ""), in SVG
    /// user units: e.g. for layout checks.
    pub fn all_bounds(&self) -> impl Iterator<Item = (&str, Rect)> {
        self.bounds.iter().map(|(k, r)| (k.as_str(), *r)).chain(self.texts.iter().map(|r| ("", *r)))
    }
}

fn to_raster(pm: tiny_skia::Pixmap) -> Raster {
    let (width, height) = (pm.width(), pm.height());
    let mut rgba = pm.take();
    // tiny-skia stores premultiplied RGBA; images want straight alpha.
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Raster { width, height, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MODELS, View};

    #[test]
    fn part_ids_exist_in_renders() {
        for m in MODELS {
            for view in [View::Front, View::Back] {
                let r = DeviceRender::new(m.svg(view));
                for part in m.parts_for(view) {
                    for id in part.ids {
                        assert!(r.has(id), "{} {}: missing #{id}", m.key, view.name());
                    }
                }
            }
        }
    }

    #[test]
    fn live_elements_are_found_and_restyled() {
        let src = crate::models::MINIFUSE2.svg(View::Front);
        let r = DeviceRender::new(src);
        for b in ["in1-inst", "in2-inst", "phantom-48v", "direct-mono"] {
            assert!(r.buttons.contains(b), "{b}");
        }
        assert_eq!(r.leds.len(), 12);
        let off = variant(src, false);
        assert!(off.contains("class=\"btn off\"") && !off.contains("class=\"btn on\""));
        let on = variant(src, true);
        assert!(on.contains("fill-opacity=\"0.28\"") && on.contains("fill-opacity=\"0.25\""));
        // bounds are in SVG user units (viewBox origin 100,460)
        let b = r.element_bounds("phantom-48v").unwrap();
        assert!((b.center().x - 1010.0).abs() < 1.0 && (b.center().y - 714.0).abs() < 1.0, "{b:?}");
    }

    #[test]
    fn knob_pointers_are_found_and_left_out_of_the_image() {
        let src = crate::models::MINIFUSE2.svg(View::Front);
        let r = DeviceRender::new(src);
        for id in ["in1-gain", "in2-gain", "monitor", "mix", "hp1-level"] {
            let p = r.pointers[id];
            assert!(p.width > 3.0 && (p.x1 - p.x2).abs() + (p.y1 - p.y2).abs() > 5.0, "{id}");
            let (r1, r2) = p.radii();
            assert!(r1 > 5.0 && r2 > r1, "{id}: {r1} {r2}");
            // the drawn angle round-trips through at_angle
            let (a, b) = p.at_angle(p.angle_deg());
            assert!((a.0 - p.x1).abs() < 0.1 && (a.1 - p.y1).abs() < 0.1 && (b.0 - p.x2).abs() < 0.1 && (b.1 - p.y2).abs() < 0.1, "{id}");
        }
        assert!(!variant_with(src, false, true).contains("class=\"pointer\""), "no pointer left in the base image");
        assert!(variant(src, false).contains("class=\"pointer\""));
        for m in MODELS {
            let ids: Vec<&str> = m.meters.knob_lights.iter().map(|k| k.0).collect();
            let r = DeviceRender::new(m.svg(View::Front));
            for id in ids {
                assert!(r.pointers.contains_key(id), "{} {id}", m.key);
            }
        }
    }

    #[test]
    fn sprites_rasterise() {
        let r = DeviceRender::new(crate::models::MINIFUSE2.svg(View::Front));
        let base = r.rasterize_base(900, 205);
        assert_eq!((base.width, base.height), (900, 205));
        assert!(base.rgba.chunks(4).any(|p| p[3] == 255));
        let (sprite, rect) = r.rasterize_lit("in1-inst", 900).unwrap();
        assert!(sprite.width > 10 && sprite.height > 10);
        // the lit lens is blue in the sprite and pale grey in the base image
        let blueish = |p: &[u8]| p[3] > 200 && p[2] as i32 - p[0] as i32 > 40;
        assert!(sprite.rgba.chunks(4).filter(|p| blueish(p)).count() > 50, "lit lens drawn");
        let lit_led = r.rasterize_lit("meter-L-0", 900).unwrap().0; // clip LED: red
        assert!(lit_led.rgba.chunks(4).any(|p| p[3] > 200 && p[0] > 200 && p[1] < 160));
        let b = r.element_bounds("in1-inst").unwrap();
        assert!(rect.contains_rect(&b), "halo sprite covers the button");
    }
}
