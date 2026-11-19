//! OpenFuse app/tray icon, drawn here as SVG (original artwork, no vendor marks).
//!
//! Glyph: a desktop audio interface seen from the front. Every control sits
//! on the body's horizontal centerline with equal gaps between them and to
//! the body's edges: two combo jacks, a three-LED meter, the monitor knob.
//! Variants: `phantom` adds a red "48" badge, `offline` greys the glyph and
//! strikes it through (simulated / not connected / controls missing).

use crate::render::Raster;
use resvg::{tiny_skia, usvg};

pub const BADGE_RED: &str = "#e5484d";
const INK: &str = "#2f353b";
const BODY: &str = "#f3f4f5";

/// The "48" badge: a small disc in the top-right corner over the knob
/// column, overlapping the knob's top edge a little.
const BADGE_X: f32 = 26.0;
const BADGE_Y: f32 = 7.5;
const BADGE_R: f32 = 5.4;

fn badge_svg(rim: &str) -> String {
    format!(
        "<circle cx=\"{BADGE_X}\" cy=\"{BADGE_Y}\" r=\"{BADGE_R}\" fill=\"{BADGE_RED}\"{rim}/>\
         <text x=\"{BADGE_X}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"6.6\" font-weight=\"bold\" \
         fill=\"#ffffff\" text-anchor=\"middle\">48</text>",
        BADGE_Y + 2.4
    )
}

/// Centerline of every control in the 32x32 viewBox.
const CY: f32 = 16.0;
/// Visual radius (fill + stroke) of the jacks and the knob in the app icon.
/// With the body's inner edge at x 1.5..30.5 this leaves five equal 1.6 gaps:
/// edge | jack | jack | meter (1.8 wide) | knob | edge.
const APP_JACKS: [f32; 2] = [6.3, 14.3];
const APP_METER_X: f32 = 20.0;
const APP_KNOB_X: f32 = 25.7;

pub fn icon_svg(phantom: bool, offline: bool) -> String {
    let op = if offline { " opacity=\"0.5\"" } else { "" };
    let jack = |cx: f32| {
        format!(
            "<circle cx=\"{cx}\" cy=\"{CY}\" r=\"2.6\" fill=\"#0f1216\" stroke=\"#dfe4ea\" stroke-width=\"1.2\"/>\
             <circle cx=\"{cx}\" cy=\"{}\" r=\"0.5\" fill=\"#dfe4ea\"/>\
             <circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"#dfe4ea\"/>\
             <circle cx=\"{}\" cy=\"{}\" r=\"0.5\" fill=\"#dfe4ea\"/>",
            CY - 1.2,
            cx - 1.05,
            CY + 0.75,
            cx + 1.05,
            CY + 0.75
        )
    };
    // meter: clip on top, signal at the bottom, spanning the jacks' height
    let meter = [("#ff6b61", -2.3f32), ("#ffd84a", 0.0), ("#5ec4ff", 2.3)]
        .iter()
        .map(|(c, dy)| format!("<circle cx=\"{APP_METER_X}\" cy=\"{}\" r=\"0.9\" fill=\"{c}\"/>", CY + dy))
        .collect::<String>();
    let k = APP_KNOB_X;
    let knob = format!(
        "<circle cx=\"{k}\" cy=\"{CY}\" r=\"2.8\" fill=\"#f1f3f6\" stroke=\"#9aa3ad\" stroke-width=\"0.8\"/>\
         <line x1=\"{k}\" y1=\"{CY}\" x2=\"{}\" y2=\"{}\" stroke=\"#2b3138\" stroke-width=\"1.1\" stroke-linecap=\"round\"/>",
        k + 1.1,
        CY - 1.9
    );
    let glyph = format!(
        "<defs><linearGradient id=\"b\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
         <stop offset=\"0\" stop-color=\"#515a65\"/><stop offset=\"1\" stop-color=\"#262b32\"/></linearGradient></defs>\
         <g{op}><rect x=\"1\" y=\"8\" width=\"30\" height=\"16\" rx=\"3.6\" fill=\"url(#b)\" stroke=\"#6d757f\" stroke-width=\"1\"/>\
         <rect x=\"3\" y=\"9\" width=\"26\" height=\"0.9\" rx=\"0.45\" fill=\"#ffffff\" opacity=\"0.2\"/>\
         {}{}{meter}{knob}</g>",
        jack(APP_JACKS[0]),
        jack(APP_JACKS[1])
    );
    let slash = if offline {
        format!(
            "<line x1=\"4\" y1=\"28\" x2=\"28\" y2=\"4\" stroke=\"{BODY}\" stroke-width=\"4.6\" stroke-linecap=\"round\"/>\
             <line x1=\"4\" y1=\"28\" x2=\"28\" y2=\"4\" stroke=\"{INK}\" stroke-width=\"2.4\" stroke-linecap=\"round\"/>"
        )
    } else {
        String::new()
    };
    let badge = if phantom {
        badge_svg(" stroke=\"#ffffff\" stroke-width=\"1\"")
    } else {
        String::new()
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 32\" width=\"32\" height=\"32\">{glyph}{slash}{badge}</svg>"
    )
}

/// Monochrome tray glyph in one `ink` colour: a solid silhouette of the
/// interface with the controls knocked out of it (the usual filled-symbol
/// style; outlines get lost at 16-22 px). Jacks are holes with a centre pin,
/// the knob is a disc in a ring-shaped gap with its pointer cut in, all on
/// one centerline at equal pitch. Only the 48 V badge keeps its colour.
/// Gaps (around the badge, along the "offline" slash) are cut out of the
/// glyph too, so nothing depends on the panel colour behind it.
pub fn tray_svg(phantom: bool, offline: bool, ink: &str) -> String {
    const PITCH: f32 = 9.0;
    // one unit below the app icon's centerline, so the badge's gap only
    // clips the top of the knob's ring
    const CY: f32 = 17.0;
    let [j1, j2, kx] = [16.0 - PITCH, 16.0, 16.0 + PITCH];
    // mask: white keeps ink, black cuts. Control holes first, the solid parts
    // inside them next, the badge / slash gaps last so nothing refills them.
    let mut cuts = String::new();
    for cx in [j1, j2] {
        cuts.push_str(&format!(
            "<circle cx=\"{cx}\" cy=\"{CY}\" r=\"3.6\" fill=\"#000\"/><circle cx=\"{cx}\" cy=\"{CY}\" r=\"1.3\" fill=\"#fff\"/>"
        ));
    }
    cuts.push_str(&format!(
        "<circle cx=\"{kx}\" cy=\"{CY}\" r=\"3.8\" fill=\"#000\"/><circle cx=\"{kx}\" cy=\"{CY}\" r=\"2.6\" fill=\"#fff\"/>\
         <line x1=\"{kx}\" y1=\"{CY}\" x2=\"{}\" y2=\"{}\" stroke=\"#000\" stroke-width=\"1.2\" stroke-linecap=\"round\"/>",
        kx + 1.3,
        CY - 2.3
    ));
    if offline {
        cuts.push_str("<line x1=\"4\" y1=\"28\" x2=\"28\" y2=\"4\" stroke=\"#000\" stroke-width=\"5.4\" stroke-linecap=\"round\"/>");
    }
    if phantom {
        cuts.push_str(&format!("<circle cx=\"{BADGE_X}\" cy=\"{BADGE_Y}\" r=\"{}\" fill=\"#000\"/>", BADGE_R + 1.1));
    }
    let op = if offline { " opacity=\"0.6\"" } else { "" };
    let slash = if offline {
        format!("<line x1=\"4\" y1=\"28\" x2=\"28\" y2=\"4\" stroke=\"{ink}\" stroke-width=\"2.4\" stroke-linecap=\"round\"/>")
    } else {
        String::new()
    };
    let badge = if phantom {
        badge_svg("")
    } else {
        String::new()
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 32\" width=\"32\" height=\"32\">\
         <defs><mask id=\"m\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"32\" height=\"32\">\
         <rect width=\"32\" height=\"32\" fill=\"#fff\"/>{cuts}</mask></defs>\
         <g{op}><rect x=\"1\" y=\"6\" width=\"30\" height=\"22\" rx=\"5\" fill=\"{ink}\" mask=\"url(#m)\"/></g>{slash}{badge}</svg>"
    )
}

/// Rasterise an icon variant at `size` x `size` (straight-alpha RGBA).
pub fn render_icon(phantom: bool, offline: bool, size: u32) -> Raster {
    rasterize(&icon_svg(phantom, offline), size)
}

/// The tray glyph in `ink` (a CSS color) at `size` x `size`.
pub fn render_tray_icon(phantom: bool, offline: bool, size: u32, ink: &str) -> Raster {
    rasterize(&tray_svg(phantom, offline, ink), size)
}

fn rasterize(svg: &str, size: u32) -> Raster {
    let opt = usvg::Options { fontdb: crate::fonts::db(), font_family: crate::fonts::UI_FAMILY.into(), ..Default::default() };
    let tree = usvg::Tree::from_str(svg, &opt).expect("icon svg");
    let mut pm = tiny_skia::Pixmap::new(size, size).expect("pixmap");
    let s = size as f32 / 32.0;
    resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    let mut rgba = pm.take();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Raster { width: size, height: size, rgba }
}

/// ARGB32 in network byte order, as StatusNotifierItem wants it.
pub fn argb32(r: &Raster) -> Vec<u8> {
    r.rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_variants_differ() {
        let base = render_icon(false, false, 32);
        let hot = render_icon(true, false, 32);
        let off = render_icon(false, true, 32);
        assert_ne!(base.rgba, hot.rgba);
        assert_ne!(base.rgba, off.rgba);
        // the badge is red in the top-right corner (sampled below the "48")
        let px = |r: &Raster, x: u32, y: u32| r.rgba[((y * 32 + x) * 4) as usize..][..4].to_vec();
        let p = px(&hot, 26, 11);
        assert!(p[0] > 180 && p[1] < 120 && p[3] > 200, "{p:?}");
        assert_eq!(argb32(&base).len(), 32 * 32 * 4);
    }

    fn alpha_at(r: &Raster, x: u32, y: u32) -> u8 {
        r.rgba[((y * r.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn tray_glyph_is_one_ink_and_fills_the_square() {
        for ink in ["#ffffff", "#24292f"] {
            let r = render_tray_icon(false, false, 64, ink);
            let want: Vec<u8> = (0..3).map(|i| u8::from_str_radix(&ink[1 + 2 * i..3 + 2 * i], 16).unwrap()).collect();
            let (mut x0, mut x1, mut y0, mut y1) = (64, 0, 64, 0);
            for y in 0..64 {
                for x in 0..64 {
                    if alpha_at(&r, x, y) > 200 {
                        let p = &r.rgba[((y * 64 + x) * 4) as usize..][..3];
                        // (un-premultiplying a fractional-alpha pixel can be off by one)
                        assert!(p.iter().zip(&want).all(|(a, b)| a.abs_diff(*b) <= 2), "only the ink color is used: {p:?}");
                    }
                    if alpha_at(&r, x, y) > 0 {
                        (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                    }
                }
            }
            // the old outline glyph used 87% x 56% of the square; the solid one 90% x 66%
            assert!(x1 - x0 >= 58 && y1 - y0 >= 42, "{x0}..{x1} x {y0}..{y1}");
            // solid body with the controls knocked out: ink between the jacks,
            // a hole in the jack ring, ink again on its centre pin
            assert!(alpha_at(&r, 23, 34) > 200, "body is filled");
            assert!(alpha_at(&r, 19, 34) < 20, "jack is a hole");
            assert!(alpha_at(&r, 14, 34) > 200, "jack pin is solid");
        }
    }

    #[test]
    fn tray_variants_cut_gaps_instead_of_painting_halos() {
        let hot = render_tray_icon(true, false, 64, "#ffffff");
        let px = |r: &Raster, x: u32, y: u32| r.rgba[((y * 64 + x) * 4) as usize..][..4].to_vec();
        let badge = px(&hot, 52, 22);
        assert!(badge[0] > 180 && badge[1] < 120 && badge[3] > 200, "red badge: {badge:?}");
        // the body's top edge runs through the ring around the badge: solid without it, cut with it
        let plain = render_tray_icon(false, false, 64, "#ffffff");
        assert!(alpha_at(&plain, 40, 13) > 200, "body present");
        assert!(alpha_at(&hot, 40, 13) < 20, "body cut around the badge, not painted over");
        assert_ne!(plain.rgba, hot.rgba);
        assert_ne!(plain.rgba, render_tray_icon(false, true, 64, "#ffffff").rgba);
    }
}
