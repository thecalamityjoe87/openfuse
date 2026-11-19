#!/usr/bin/env python3
"""Generate original SVG renders of the MiniFuse 1 / 2 / 4 (front and back).

The drawings are our own vector line-art; layout positions were traced from
reference photos so callouts line up with the real hardware. No vendor logos
or branding are reproduced.

Every user-facing part is an SVG element with a stable ``id`` (see CONTROL IDS
below) so the UI can anchor callout lines to it and light LEDs / buttons.

Usage:  python3 tools/render_devices.py [outdir]   (default: assets/render)

CONTROL IDS
  front:  in{N}-jack in{N}-gain in{N}-inst phantom-48v meter-{CH}-{ROW}
          monitor mix direct-mono | direct-monitor  hp{N}-level hp{N}-jack
          func-button
  back:   out-{CH} in{N}-jack midi-in midi-out usb-hub{N} usb-host dc-in
"""
import math
import os
import sys

# ---------------------------------------------------------------- palette
BODY = "#f3f4f5"
BODY_EDGE = "#d5d9dd"
PANEL_EDGE = "#c9ced3"
INK = "#3b4148"
DARK = "#16181b"
DARK2 = "#2a2d31"
METAL = "#b9bdc2"
METAL_HI = "#e6e8ea"
KNOB = "#fbfbfc"
KNOB_EDGE = "#d8dce0"
BLUE = "#7cc8f0"
BLUE_GLOW = "#bfe6fb"
RED = "#f07a7a"
RED_GLOW = "#fbc4c4"
YELLOW = "#e9e26a"
FONT = "'Barlow Condensed','Roboto Condensed','DIN Condensed','Arial Narrow',sans-serif"


class Svg:
    def __init__(self, vb):
        self.vb = vb
        self.parts = []

    def add(self, s):
        self.parts.append(s)

    def group(self, gid, inner, cls=None):
        c = f' class="{cls}"' if cls else ""
        self.add(f'<g id="{gid}"{c}>{inner}</g>')

    def render(self):
        x, y, w, h = self.vb
        defs = f"""<defs>
  <radialGradient id="g-knob" cx="40%" cy="35%" r="75%">
    <stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="{KNOB}"/></radialGradient>
  <radialGradient id="g-metal" cx="45%" cy="40%" r="60%">
    <stop offset="0" stop-color="{METAL_HI}"/><stop offset="1" stop-color="{METAL}"/></radialGradient>
  <radialGradient id="g-blue" cx="50%" cy="45%" r="55%">
    <stop offset="0" stop-color="{BLUE_GLOW}"/><stop offset="1" stop-color="{BLUE}"/></radialGradient>
  <radialGradient id="g-red" cx="50%" cy="45%" r="55%">
    <stop offset="0" stop-color="{RED_GLOW}"/><stop offset="1" stop-color="{RED}"/></radialGradient>
  <filter id="f-shadow" x="-20%" y="-20%" width="140%" height="150%">
    <feDropShadow dx="0" dy="10" stdDeviation="11" flood-color="#000" flood-opacity="0.42"/></filter>
  <style>
    text{{font-family:{FONT};fill:{INK};text-anchor:middle;dominant-baseline:central}}
    .led-off{{opacity:.35}}
    .btn.off .lens{{fill:#e7eaed}}
  </style>
</defs>"""
        body = "\n".join(self.parts)
        return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x} {y} {w} {h}" '
                f'width="{w}" height="{h}">\n{defs}\n{body}\n</svg>\n')


# ---------------------------------------------------------------- primitives
def text(x, y, s, size=30, weight=500, rot=None, spacing=1):
    t = f' transform="rotate({rot} {x} {y})"' if rot is not None else ""
    return (f'<text x="{x}" y="{y}" font-size="{size}" font-weight="{weight}" '
            f'letter-spacing="{spacing}"{t}>{s}</text>')


def body(svg, x0, y0, x1, y1, r=20, inner=None):
    svg.add(f'<rect x="{x0}" y="{y0}" width="{x1-x0}" height="{y1-y0}" rx="{r}" '
            f'fill="{BODY}" stroke="{BODY_EDGE}" stroke-width="4" filter="url(#f-shadow)"/>')
    if inner:
        ix0, iy0, ix1, iy1 = inner
        svg.add(f'<rect x="{ix0}" y="{iy0}" width="{ix1-ix0}" height="{iy1-iy0}" rx="8" '
                f'fill="none" stroke="{PANEL_EDGE}" stroke-width="3"/>')


def vline(svg, x, y0, y1, w=3):
    svg.add(f'<line x1="{x}" y1="{y0}" x2="{x}" y2="{y1}" stroke="{INK}" stroke-width="{w}"/>')


def hline(svg, x0, x1, y, w=2):
    svg.add(f'<line x1="{x0}" y1="{y}" x2="{x1}" y2="{y}" stroke="{INK}" stroke-width="{w}"/>')


def screw(cx, cy, r):
    k = r * 0.5
    return (f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="url(#g-metal)" stroke="#8d9297" stroke-width="2"/>'
            f'<path d="M{cx-k} {cy}H{cx+k}M{cx} {cy-k}V{cy+k}" stroke="#6b7076" stroke-width="{r*0.25}" '
            f'stroke-linecap="round"/>')


def combo_jack(svg, gid, cx, cy, r, screws):
    s = [f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{DARK}"/>',
         f'<circle cx="{cx}" cy="{cy}" r="{r*0.78}" fill="{DARK2}" stroke="#000" stroke-width="2"/>']
    # XLR pin holes (1, 2, 3) + centre 1/4" socket
    for ang in (200, 340, 90):
        a = math.radians(ang)
        s.append(f'<circle cx="{cx+math.cos(a)*r*0.48:.1f}" cy="{cy+math.sin(a)*r*0.48:.1f}" '
                 f'r="{r*0.09:.1f}" fill="#000"/>')
    s.append(f'<circle cx="{cx}" cy="{cy}" r="{r*0.24:.1f}" fill="#000" stroke="#45494e" stroke-width="2"/>')
    s.append(f'<rect x="{cx-r*0.05:.1f}" y="{cy-r*0.36:.1f}" width="{r*0.1:.1f}" height="{r*0.14:.1f}" '
             f'fill="{METAL}"/>')
    for sx, sy in screws:
        s.append(screw(sx, sy, r * 0.19))
    svg.group(gid, "".join(s), "jack combo")


def knob(svg, gid, cx, cy, r, angle_deg, accent=BLUE, sides=10):
    pts = " ".join(f"{cx+math.cos(math.radians(i*360/sides+9))*r:.1f},"
                   f"{cy+math.sin(math.radians(i*360/sides+9))*r:.1f}" for i in range(sides))
    a = math.radians(angle_deg)
    x0, y0 = cx + math.cos(a) * r * 0.55, cy + math.sin(a) * r * 0.55
    x1, y1 = cx + math.cos(a) * r * 0.92, cy + math.sin(a) * r * 0.92
    inner = (f'<polygon points="{pts}" fill="url(#g-knob)" stroke="{KNOB_EDGE}" stroke-width="3" '
             f'filter="url(#f-shadow)"/>'
             f'<line class="pointer" x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" '
             f'stroke="{accent}" stroke-width="{max(5, r*0.13):.1f}" stroke-linecap="round"/>')
    svg.group(gid, inner, "knob")


def button(svg, gid, cx, cy, r, color="blue", on=True):
    fill = "url(#g-blue)" if color == "blue" else "url(#g-red)"
    rim = BLUE if color == "blue" else RED
    inner = (f'<circle cx="{cx}" cy="{cy}" r="{r*1.12:.1f}" fill="#e9ecef" stroke="{BODY_EDGE}" stroke-width="2"/>'
             f'<circle class="lens" cx="{cx}" cy="{cy}" r="{r}" fill="{fill}" stroke="{rim}" stroke-width="3"/>')
    svg.group(gid, inner, "btn " + ("on" if on else "off"))


def dark_button(svg, gid, cx, cy, r):
    inner = (f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{DARK2}" stroke="#000" stroke-width="3"/>'
             f'<circle cx="{cx}" cy="{cy}" r="{r*0.72:.1f}" fill="none" stroke="#55595e" stroke-width="2"/>')
    svg.group(gid, inner, "btn")


def led(svg, gid, cx, cy, r, color, lit=False):
    if lit:
        inner = f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{color}" stroke="{color}" stroke-width="2"/>'
    else:
        inner = f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{color}" stroke-width="{max(2, r*0.3):.1f}"/>'
    svg.group(gid, inner, "led" + (" lit" if lit else ""))


def meter(svg, prefix, cols, rows, r, labels, label_y, label_size):
    """cols: list of (channel_name, x); rows top->bottom; top = clip."""
    for (ch, x), lab in zip(cols, labels):
        lx = x
        if isinstance(lab, tuple):
            lab, lx = lab
        svg.add(text(lx, label_y, lab, label_size))
        for i, y in enumerate(rows):
            color = "#5a5f65" if i == 0 else ("#5a5f65" if i < 3 else BLUE)
            if i == 0:
                color = RED
            elif i == 1:
                color = YELLOW
            led(svg, f"{prefix}-{ch}-{i}", x, y, r, color)


def trs_jack(svg, gid, cx, cy, r):
    inner = (f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="url(#g-metal)" stroke="#7d8287" stroke-width="2"/>'
             f'<circle cx="{cx}" cy="{cy}" r="{r*0.8:.1f}" fill="{DARK}"/>'
             f'<circle cx="{cx}" cy="{cy}" r="{r*0.5:.1f}" fill="#000"/>')
    svg.group(gid, inner, "jack")


def midi_din(svg, gid, cx, cy, r):
    s = [f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{DARK}"/>',
         f'<circle cx="{cx}" cy="{cy}" r="{r*0.82:.1f}" fill="{DARK2}" stroke="#000" stroke-width="2"/>',
         f'<rect x="{cx-r*0.1:.1f}" y="{cy-r*0.8:.1f}" width="{r*0.2:.1f}" height="{r*0.16:.1f}" fill="#000"/>']
    # 5-pin DIN: pins on a 180-degree arc around the lower half, key notch at the top
    for ang in (180, 135, 90, 45, 0):
        a = math.radians(ang)
        s.append(f'<circle cx="{cx+math.cos(a)*r*0.5:.1f}" cy="{cy+math.sin(a)*r*0.5 - r*0.1:.1f}" '
                 f'r="{r*0.08:.1f}" fill="#000" stroke="#55595e" stroke-width="1.5"/>')
    svg.group(gid, "".join(s), "jack midi")


def usb_a(svg, gid, x, y, w, h):
    inner = (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="3" fill="url(#g-metal)" stroke="#6f7479" stroke-width="2"/>'
             f'<rect x="{x+w*0.14:.1f}" y="{y+h*0.05:.1f}" width="{w*0.72:.1f}" height="{h*0.9:.1f}" fill="{DARK2}"/>'
             f'<rect x="{x+w*0.18:.1f}" y="{y+h*0.08:.1f}" width="{w*0.3:.1f}" height="{h*0.84:.1f}" fill="#efe9dc"/>')
    svg.group(gid, inner, "port usb-a")


def usb_c(svg, gid, cx, cy, w, h):
    inner = (f'<rect x="{cx-w/2}" y="{cy-h/2}" width="{w}" height="{h}" rx="{h/2}" fill="url(#g-metal)" '
             f'stroke="#6f7479" stroke-width="2"/>'
             f'<rect x="{cx-w*0.4:.1f}" y="{cy-h*0.3:.1f}" width="{w*0.8:.1f}" height="{h*0.6:.1f}" rx="{h*0.3:.1f}" fill="{DARK}"/>'
             f'<rect x="{cx-w*0.28:.1f}" y="{cy-h*0.06:.1f}" width="{w*0.56:.1f}" height="{h*0.12:.1f}" fill="{METAL}"/>')
    svg.group(gid, inner, "port usb-c")


def dc_jack(svg, gid, cx, cy, r):
    inner = (f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="url(#g-metal)" stroke="#6f7479" stroke-width="2"/>'
             f'<circle cx="{cx}" cy="{cy}" r="{r*0.62:.1f}" fill="{DARK2}"/>'
             f'<circle cx="{cx}" cy="{cy}" r="{r*0.2:.1f}" fill="{METAL}"/>')
    svg.group(gid, inner, "port dc")


def lock_slot(svg, x, y, w, h):
    svg.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{w*0.25}" fill="{DARK}"/>')


# icons (stroked line art, drawn around a centre point, unit size ~1)
def guitar_icon(cx, cy, s):
    """Electric guitar line icon, about `s` long, centred on (cx, cy).

    Drawn lying horizontally (body left, headstock right) in unit coordinates and
    then tilted up to the right, like the icon printed under the INST buttons.
    """
    def pt(x, y):
        return f"{x:.3f},{y:.3f}"

    # Double-cutaway body: big lower bout on the left, waist, two horns towards the neck.
    body = (
        f"M{pt(-0.07, -0.045)} "
        f"C{pt(-0.03, -0.10)} {pt(-0.04, -0.165)} {pt(-0.11, -0.165)} "
        f"C{pt(-0.17, -0.165)} {pt(-0.18, -0.105)} {pt(-0.24, -0.105)} "
        f"C{pt(-0.31, -0.105)} {pt(-0.33, -0.175)} {pt(-0.41, -0.175)} "
        f"C{pt(-0.50, -0.175)} {pt(-0.54, -0.09)} {pt(-0.54, 0.0)} "
        f"C{pt(-0.54, 0.09)} {pt(-0.49, 0.175)} {pt(-0.40, 0.175)} "
        f"C{pt(-0.32, 0.175)} {pt(-0.30, 0.105)} {pt(-0.23, 0.105)} "
        f"C{pt(-0.17, 0.105)} {pt(-0.15, 0.145)} {pt(-0.105, 0.145)} "
        f"C{pt(-0.05, 0.145)} {pt(-0.04, 0.095)} {pt(-0.07, 0.045)} Z"
    )
    # Neck lies on top of the body (filled, so the body outline under it is hidden).
    neck = f"M{pt(-0.22, -0.032)} H0.40 V0.032 H-0.22 Z"
    # Angled headstock.
    head = (f"M{pt(0.40, -0.036)} L{pt(0.53, -0.062)} Q{pt(0.565, -0.066)} {pt(0.565, -0.032)} "
            f"L{pt(0.565, 0.012)} Q{pt(0.565, 0.040)} {pt(0.530, 0.040)} L{pt(0.40, 0.036)} Z")
    w = 0.03  # stroke width in icon units
    return (
        f'<g transform="translate({cx} {cy}) rotate(-27) scale({s})" fill="none" stroke="{INK}" '
        f'stroke-width="{w}" stroke-linejoin="round" stroke-linecap="round">'
        f'<path d="{body}"/>'
        f'<path d="{neck}" fill="{BODY}"/>'
        f'<path d="{head}" fill="{BODY}"/>'
        # pickup and bridge
        f'<rect x="-0.305" y="-0.065" width="0.045" height="0.13" rx="0.012" fill="{BODY}"/>'
        f'<path d="M-0.40 -0.06 V0.06"/>'
        f'</g>'
    )


def headphones_icon(cx, cy, s):
    return (f'<path d="M{cx-s*0.4},{cy+s*0.3} V{cy} a{s*0.4},{s*0.4} 0 0 1 {s*0.8},0 V{cy+s*0.3}" '
            f'fill="none" stroke="{INK}" stroke-width="{s*0.12:.1f}"/>'
            f'<rect x="{cx-s*0.5:.1f}" y="{cy+s*0.05:.1f}" width="{s*0.2:.1f}" height="{s*0.35:.1f}" rx="{s*0.05:.1f}" fill="{INK}"/>'
            f'<rect x="{cx+s*0.3:.1f}" y="{cy+s*0.05:.1f}" width="{s*0.2:.1f}" height="{s*0.35:.1f}" rx="{s*0.05:.1f}" fill="{INK}"/>')


def laptop_icon(cx, cy, s):
    return (f'<rect x="{cx-s*0.36:.1f}" y="{cy-s*0.32:.1f}" width="{s*0.72:.1f}" height="{s*0.48:.1f}" rx="{s*0.03:.1f}" '
            f'fill="none" stroke="{INK}" stroke-width="{s*0.04:.1f}"/>'
            f'<path d="M{cx-s*0.48},{cy+s*0.3} L{cx-s*0.4},{cy+s*0.2} H{cx+s*0.4} L{cx+s*0.48},{cy+s*0.3} Z" '
            f'fill="none" stroke="{INK}" stroke-width="{s*0.04:.1f}" stroke-linejoin="round"/>')


def usb_icon(cx, cy, s):
    w = s * 0.05
    return (f'<g stroke="{INK}" stroke-width="{w:.1f}" fill="none" stroke-linecap="round">'
            f'<path d="M{cx-s*0.45},{cy} H{cx+s*0.35}"/>'
            f'<path d="M{cx-s*0.15},{cy} L{cx},{cy-s*0.18} H{cx+s*0.15}"/>'
            f'<path d="M{cx-s*0.05},{cy} L{cx+s*0.08},{cy+s*0.16} H{cx+s*0.2}"/></g>'
            f'<circle cx="{cx-s*0.45:.1f}" cy="{cy}" r="{s*0.08:.1f}" fill="{INK}"/>'
            f'<path d="M{cx+s*0.35},{cy-s*0.08} L{cx+s*0.5},{cy} L{cx+s*0.35},{cy+s*0.08} Z" fill="{INK}"/>'
            f'<circle cx="{cx+s*0.17:.1f}" cy="{cy-s*0.18:.1f}" r="{s*0.05:.1f}" fill="{INK}"/>'
            f'<rect x="{cx+s*0.18:.1f}" y="{cy+s*0.11:.1f}" width="{s*0.1:.1f}" height="{s*0.1:.1f}" fill="{INK}"/>')


def dc_polarity_icon(cx, cy, s):
    return (f'<g stroke="{INK}" stroke-width="{s*0.05:.1f}" fill="none">'
            f'<circle cx="{cx-s*0.45:.1f}" cy="{cy}" r="{s*0.12:.1f}"/><circle cx="{cx}" cy="{cy}" r="{s*0.12:.1f}"/>'
            f'<circle cx="{cx+s*0.45:.1f}" cy="{cy}" r="{s*0.12:.1f}"/>'
            f'<path d="M{cx-s*0.52},{cy}H{cx-s*0.38}M{cx-s*0.33},{cy}H{cx-s*0.12}M{cx+s*0.12},{cy}H{cx+s*0.33}'
            f'M{cx+s*0.38},{cy}H{cx+s*0.52}M{cx+s*0.45},{cy-s*0.07}V{cy+s*0.07}"/></g>'
            f'<circle cx="{cx}" cy="{cy}" r="{s*0.04:.1f}" fill="{INK}"/>')


def model_name(svg, x, y, name, size):
    svg.add(text(x, y, name, size, 600, rot=-90, spacing=3))


# ---------------------------------------------------------------- MiniFuse 1
def mf1_front():
    s = Svg((100, 400, 1800, 520))
    body(s, 137, 432, 1863, 873, 24)
    combo_jack(s, "in1-jack", 345, 640, 125, [(236, 529), (458, 752)])
    s.add(text(345, 830, "INPUT", 44, 600, spacing=2))
    knob(s, "in1-gain", 625, 550, 66, 235)
    s.add(text(622, 655, "GAIN", 34))
    button(s, "in1-inst", 622, 724, 30)
    s.add(guitar_icon(633, 812, 100))
    vline(s, 718, 458, 853)
    button(s, "phantom-48v", 787, 724, 30, "red")
    s.add(text(787, 806, "48 V", 34))
    rows = [523, 573, 624, 675, 725, 776]
    meter(s, "meter", [("L", 900), ("R", 938)], rows, 8, ["L", "R"], 489, 34)
    knob(s, "monitor", 1165, 650, 150, 245, accent=DARK2)
    s.add(text(1310, 488, "MONITOR", 34))
    button(s, "direct-monitor", 1443, 724, 30)
    s.add(text(1443, 795, "DIRECT", 32))
    s.add(text(1443, 828, "MONITOR", 32))
    s.add(headphones_icon(1580, 482, 30))
    knob(s, "hp1-level", 1675, 550, 66, 165)
    vline(s, 1675, 616, 655, 2)
    trs_jack(s, "hp1-jack", 1675, 710, 57)
    model_name(s, 1812, 650, "MINIFUSE 1", 54)
    return s


def mf1_back():
    s = Svg((130, 400, 1740, 520))
    body(s, 160, 437, 1837, 863, 24, inner=(178, 454, 1818, 840))
    lock_slot(s, 361, 608, 36, 80)
    s.add(text(672, 512, "OUTPUTS", 34))
    hline(s, 481, 863, 538)
    s.add(text(560, 586, "L", 32))
    s.add(text(782, 586, "R", 32))
    trs_jack(s, "out-L", 560, 707, 58)
    trs_jack(s, "out-R", 780, 707, 58)
    s.add(usb_icon(1327, 511, 60))
    hline(s, 1142, 1512, 538)
    s.add(text(1216, 586, "HUB", 32))
    usb_a(s, "usb-hub1", 1187, 627, 58, 141)
    s.add(laptop_icon(1433, 685, 90))
    usb_c(s, "usb-host", 1433, 759, 100, 34)
    return s


# ---------------------------------------------------------------- MiniFuse 2
def mf2_front():
    s = Svg((100, 460, 1800, 410))
    body(s, 125, 487, 1875, 830, 20)
    for n, (jx, kx, s1, s2, lx) in enumerate([(260, 452, (175, 562), (347, 737), 165),
                                              (690, 882, (603, 562), (776, 737), 580)], start=1):
        combo_jack(s, f"in{n}-jack", jx, 650, 97, [s1, s2])
        knob(s, f"in{n}-gain", kx, 580, 52, 200 if n == 1 else 280)
        s.add(text(kx, 654, "GAIN", 30))
        button(s, f"in{n}-inst", kx, 714, 24)
        s.add(guitar_icon(kx + 8, 778, 80))
        s.add(text(lx, 785, str(n), 44, 500))
    vline(s, 540, 500, 808)
    vline(s, 962, 500, 808)
    button(s, "phantom-48v", 1010, 714, 24, "red")
    s.add(text(1010, 773, "48 V", 30))
    rows = [557, 597, 637, 677, 717, 757]
    meter(s, "meter", [("L", 1089), ("R", 1119)], rows, 6, ["L", "R"], 525, 28)
    knob(s, "monitor", 1278, 657, 118, 350, accent=DARK2)
    s.add(text(1395, 528, "MONITOR", 30))
    knob(s, "mix", 1515, 580, 52, 60)
    s.add(text(1448, 638, "INPUT", 20))
    s.add(text(1574, 638, "USB", 20))
    s.add(text(1515, 657, "MIX", 30))
    button(s, "direct-mono", 1512, 714, 24)
    s.add(text(1512, 766, "DIRECT", 28))
    s.add(text(1512, 792, "MONO", 28))
    s.add(headphones_icon(1655, 522, 26))
    knob(s, "hp1-level", 1727, 580, 52, 160)
    vline(s, 1727, 632, 662, 2)
    trs_jack(s, "hp1-jack", 1725, 707, 45)
    model_name(s, 1840, 657, "MINIFUSE 2", 44)
    return s


def mf2_back():
    s = Svg((110, 460, 1780, 410))
    body(s, 132, 487, 1870, 830, 20, inner=(147, 500, 1855, 812))
    lock_slot(s, 288, 627, 28, 62)
    s.add(text(607, 540, "OUTPUTS", 30))
    hline(s, 455, 760, 561)
    s.add(text(516, 600, "L", 28))
    s.add(text(692, 600, "R", 28))
    trs_jack(s, "out-L", 518, 705, 45)
    trs_jack(s, "out-R", 692, 705, 45)
    s.add(text(1020, 540, "MIDI", 30))
    hline(s, 802, 1238, 561)
    s.add(text(823, 600, "IN", 28))
    s.add(text(1041, 600, "OUT", 28))
    midi_din(s, "midi-in", 910, 672, 75)
    midi_din(s, "midi-out", 1128, 672, 75)
    s.add(usb_icon(1435, 542, 50))
    hline(s, 1287, 1585, 563)
    s.add(text(1346, 602, "HUB", 28))
    usb_a(s, "usb-hub1", 1327, 642, 40, 107)
    s.add(laptop_icon(1520, 682, 70))
    usb_c(s, "usb-host", 1520, 744, 80, 26)
    return s


# ---------------------------------------------------------------- MiniFuse 4
def mf4_front():
    s = Svg((90, 480, 1760, 330))
    body(s, 115, 507, 1827, 777, 16)
    for n, (jx, kx, s1, s2, lx) in enumerate([(248, 400, (180, 569), (316, 708), 155),
                                              (614, 767, (545, 569), (682, 705), 509)], start=1):
        combo_jack(s, f"in{n}-jack", jx, 638, 78, [s1, s2])
        knob(s, f"in{n}-gain", kx, 583, 40, 200)
        s.add(text(kx, 645, "GAIN", 24))
        button(s, f"in{n}-inst", kx, 689, 20)
        s.add(guitar_icon(kx + 10, 742, 62))
        s.add(text(lx, 748, str(n), 36, 500))
    vline(s, 485, 525, 765, 2)
    vline(s, 844, 525, 765, 2)
    button(s, "phantom-48v", 887, 689, 20, "red")
    s.add(text(888, 738, "48 V", 24))
    rows = [566, 597, 628, 659, 690, 721]
    meter(s, "meter", [("1", 971), ("2", 994), ("3", 1040), ("4", 1063)], rows, 5,
          [("1L", 963), ("2R", 1003), ("3", 1035), ("4", 1068)], 544, 20)
    knob(s, "monitor", 1210, 645, 92, 265, accent=DARK2)
    s.add(text(1301, 544, "MONITOR", 22))
    s.add(headphones_icon(1505, 540, 20))
    for n, x in enumerate((1421, 1585), start=1):
        knob(s, f"hp{n}-level", x, 585, 40, 125, accent=DARK2)
        trs_jack(s, f"hp{n}-jack", x, 680, 31)
        s.add(text(x, 738, str(n), 24))
    dark_button(s, "func-button", 1711, 583, 37)
    model_name(s, 1795, 645, "MINIFUSE 4", 38)
    return s


def mf4_back():
    s = Svg((95, 480, 1770, 330))
    body(s, 118, 513, 1838, 783, 16, inner=(130, 523, 1826, 770))
    lock_slot(s, 177, 663, 22, 50)
    s.add(text(378, 556, "INPUTS", 22))
    hline(s, 278, 480, 573)
    for n, x in ((3, 317), (4, 429)):
        s.add(text(x + 2, 604, str(n), 22))
        trs_jack(s, f"in{n}-jack", x, 652, 31)
    s.add(screw(500, 625, 11))
    s.add(screw(1000, 625, 11))
    s.add(text(760, 556, "OUTPUTS", 22))
    hline(s, 547, 973, 573)
    for ch, lab, x in (("1", "1L", 593), ("2", "2R", 705), ("3", "3", 817), ("4", "4", 929)):
        s.add(text(x + 1, 604, lab, 22))
        trs_jack(s, f"out-{ch}", x, 652, 31)
    s.add(text(1208, 556, "MIDI", 22))
    hline(s, 1066, 1350, 573)
    s.add(text(1073, 604, "IN", 22))
    s.add(text(1218, 604, "OUT", 22))
    midi_din(s, "midi-in", 1137, 660, 60)
    midi_din(s, "midi-out", 1284, 660, 60)
    s.add(usb_icon(1495, 556, 40))
    hline(s, 1374, 1615, 573)
    s.add(text(1443, 604, "HUB", 22))
    usb_a(s, "usb-hub1", 1391, 636, 32, 85)
    usb_a(s, "usb-hub2", 1462, 636, 32, 85)
    s.add(laptop_icon(1573, 665, 56))
    usb_c(s, "usb-host", 1572, 717, 64, 22)
    s.add(dc_polarity_icon(1665, 618, 60))
    dc_jack(s, "dc-in", 1667, 662, 33)
    return s


RENDERS = {
    "minifuse1_front": mf1_front, "minifuse1_back": mf1_back,
    "minifuse2_front": mf2_front, "minifuse2_back": mf2_back,
    "minifuse4_front": mf4_front, "minifuse4_back": mf4_back,
}


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "assets", "render")
    os.makedirs(out, exist_ok=True)
    for name, fn in RENDERS.items():
        path = os.path.join(out, name + ".svg")
        with open(path, "w") as f:
            f.write(fn().render())
        print("wrote", os.path.normpath(path))


if __name__ == "__main__":
    main()
