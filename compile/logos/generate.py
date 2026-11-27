#!/usr/bin/env python3
"""Draws Openbar5000's logos: every icon to choose from in Preferences >
Appearance, each in its regular look, a look for each holiday and one for
each month's pride flag (see `crates/app/src/holiday.rs`).

    python3 compile/logos/generate.py

writes `crates/app/logos/<icon>/<version>.svg` and the table of them,
`crates/app/src/logo_files.rs`. Only the standard library is needed. The
original music box ("music-box") isn't drawn here: its files are kept as
they are.
"""

import math
import os
import random

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "app", "logos")
TABLE = os.path.join(ROOT, "crates", "app", "src", "logo_files.rs")

RAINBOW = ["#e40303", "#ff8c00", "#ffed00", "#008026", "#004dff", "#750787"]
# One per month, January first -- as `holiday::MONTHLY_FLAGS`.
FLAGS = [
    ("Rainbow", RAINBOW),
    ("Transgender", ["#5bcefa", "#f5a9b8", "#ffffff", "#f5a9b8", "#5bcefa"]),
    ("Bisexual", ["#d60270", "#d60270", "#9b4f96", "#0038a8", "#0038a8"]),
    ("Pansexual", ["#ff218c", "#ffd800", "#21b1ff"]),
    ("Lesbian", ["#d52d00", "#ff9a56", "#ffffff", "#d362a4", "#a30262"]),
    ("Progress Pride", ["#000000", "#784f17", "#5bcefa", "#f5a9b8", "#ffffff"] + RAINBOW),
    ("Non-binary", ["#fcf434", "#ffffff", "#9c59d1", "#2c2c2c"]),
    ("Asexual", ["#000000", "#a3a3a3", "#ffffff", "#800080"]),
    ("Genderfluid", ["#ff76a4", "#ffffff", "#c011d7", "#000000", "#2f3cbe"]),
    ("Intersex", ["#ffd800", "#7902aa", "#ffd800"]),
    ("Aromantic", ["#3da542", "#a7d379", "#ffffff", "#a9a9a9", "#000000"]),
    ("Agender", ["#000000", "#b9b9b9", "#ffffff", "#b8f483", "#ffffff", "#b9b9b9", "#000000"]),
]
HOLIDAYS = ["workers-day", "pride-month", "christmas", "new-year", "chinese-new-year"]
VERSIONS = ["regular"] + HOLIDAYS + [f"flag-{m:02d}" for m in range(1, 13)]


class Version:
    """Which look to draw: `kind` is regular, workers, pride, christmas,
    newyear, cny or flag; `stripes` the flag's (the rainbow for pride)."""

    def __init__(self, key):
        self.key = key
        kinds = {"regular": "regular", "workers-day": "workers", "pride-month": "pride",
                 "christmas": "christmas", "new-year": "newyear", "chinese-new-year": "cny"}
        if key.startswith("flag-"):
            self.kind = "flag"
            self.stripes = FLAGS[int(key[5:]) - 1][1]
        else:
            self.kind = kinds[key]
            self.stripes = RAINBOW if self.kind == "pride" else None

    @property
    def striped(self):
        return self.stripes is not None


def visible(stripes, dark="#3a3548"):
    """Black stripes drawn on something dark would vanish: a dark grey."""
    return [dark if c in ("#000000", "#2c2c2c") else c for c in stripes]


def svg(defs, body):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">'
            f'<defs>{defs}</defs>{body}</svg>')


# --- shared pieces ------------------------------------------------------

NOTE = ('<rect x="10" y="0" width="9" height="56" rx="4.5"/>'
        '<path d="M14 0 C 40 5, 46 22, 32 38 C 35 24, 28 16, 14 13 Z"/>'
        '<ellipse cx="5" cy="54" rx="15" ry="11" transform="rotate(-20 5 54)"/>')


def note(x, y, rot=0, scale=1, fill="#ffffff", extra=""):
    return f'<g transform="translate({x},{y}) rotate({rot}) scale({scale})" fill="{fill}"{extra}>{NOTE}</g>'


def star4(cx, cy, r, fill, k=0.28):
    pts = []
    for i in range(8):
        a = math.pi / 4 * i - math.pi / 2
        rr = r if i % 2 == 0 else r * k
        pts.append(f"{cx + rr * math.cos(a):.1f},{cy + rr * math.sin(a):.1f}")
    return f'<polygon points="{" ".join(pts)}" fill="{fill}"/>'


def snowflake(cx, cy, r, color, width=2.6):
    s = f'<g stroke="{color}" stroke-width="{width}" stroke-linecap="round">'
    for k in range(6):
        a = k * math.tau / 6
        s += f'<line x1="{cx}" y1="{cy}" x2="{cx + r * math.cos(a):.1f}" y2="{cy + r * math.sin(a):.1f}"/>'
        mx, my = cx + r * 0.6 * math.cos(a), cy + r * 0.6 * math.sin(a)
        for d in (0.7, -0.7):
            s += (f'<line x1="{mx:.1f}" y1="{my:.1f}" x2="{mx + r * 0.32 * math.cos(a + d):.1f}" '
                  f'y2="{my + r * 0.32 * math.sin(a + d):.1f}"/>')
    return s + "</g>"


def burst(cx, cy, r, color, rays=12, width=3.2):
    s = ""
    for k in range(rays):
        a = k * math.tau / rays
        s += (f'<line x1="{cx + math.cos(a) * r * .38:.1f}" y1="{cy + math.sin(a) * r * .38:.1f}" '
              f'x2="{cx + math.cos(a) * r:.1f}" y2="{cy + math.sin(a) * r:.1f}" '
              f'stroke="{color}" stroke-width="{width}" stroke-linecap="round"/>')
    return s + f'<circle cx="{cx}" cy="{cy}" r="{width}" fill="{color}"/>'


def confetti(pieces):
    return "".join(f'<rect x="{x}" y="{y}" width="9" height="4.5" rx="1.2" fill="{c}" transform="rotate({r} {x} {y})"/>'
                   for x, y, c, r in pieces)


def holly(x, y, scale=1.0):
    return (f'<g transform="translate({x} {y}) scale({scale})">'
            '<path d="M0 0 C -14 -10, -30 -4, -34 6 C -26 2, -20 8, -14 6 C -16 12, -8 14, 0 0 Z" fill="#26a269"/>'
            '<path d="M0 0 C 14 -10, 30 -4, 34 6 C 26 2, 20 8, 14 6 C 16 12, 8 14, 0 0 Z" fill="#2ec27e"/>'
            '<circle cx="-5" cy="2" r="5" fill="#c01c28"/><circle cx="5" cy="3" r="5" fill="#a51d2d"/>'
            '<circle cx="0" cy="-5" r="5" fill="#e01b24"/></g>')


def santa_hat(x, y, scale=1.0, rot=0):
    """Its brim's middle at (x, y)."""
    return (f'<g transform="translate({x} {y}) rotate({rot}) scale({scale})">'
            '<path d="M-30 0 C -24 -36, 10 -52, 40 -40 C 50 -36, 54 -24, 48 -14 C 36 -22, 24 -14, 22 0 Z" fill="#d42a2a"/>'
            '<path d="M-20 -14 C -12 -32, 8 -40, 28 -38" fill="none" stroke="#ff7b7b" stroke-width="4" '
            'stroke-linecap="round" opacity="0.7"/>'
            '<rect x="-36" y="-8" width="66" height="15" rx="7.5" fill="#ffffff"/>'
            '<circle cx="50" cy="-14" r="10" fill="#ffffff"/></g>')


def party_hat(x, y, scale=1.0, rot=0):
    """Its base's middle at (x, y)."""
    return (f'<g transform="translate({x} {y}) rotate({rot}) scale({scale})">'
            '<path d="M-22 0 L0 -58 L22 0 Z" fill="#9141ac"/>'
            '<path d="M-15 -18 L15 -18 L18 -9 L-18 -9 Z M-8 -38 L8 -38 L10 -31 L-10 -31 Z" fill="#f2c14e"/>'
            '<rect x="-24" y="-4" width="48" height="7" rx="3.5" fill="#f2c14e"/>'
            '<circle cx="0" cy="-58" r="8" fill="#f2c14e"/></g>')


def lantern(x, y, scale=1.0, gold="#f2c14e"):
    """Hanging from (x, y)."""
    return (f'<g transform="translate({x} {y}) scale({scale})">'
            f'<line x1="0" y1="0" x2="0" y2="22" stroke="{gold}" stroke-width="3"/>'
            f'<rect x="-13" y="20" width="26" height="7" rx="3" fill="{gold}"/>'
            f'<ellipse cx="0" cy="50" rx="24" ry="26" fill="#e0352b" stroke="{gold}" stroke-width="3.5"/>'
            f'<path d="M0 24 C -16 37, -16 63, 0 76 M0 24 C 16 37, 16 63, 0 76" fill="none" stroke="{gold}" stroke-width="2.4"/>'
            f'<rect x="-13" y="74" width="26" height="7" rx="3" fill="{gold}"/>'
            f'<line x1="0" y1="81" x2="0" y2="96" stroke="{gold}" stroke-width="3"/></g>')


def red_flag(x, y, scale=1.0, pole="#e8d3b0"):
    """A pennant on a pole standing at (x, y)."""
    return (f'<g transform="translate({x} {y}) scale({scale})">'
            f'<rect x="-2.5" y="-70" width="5" height="70" rx="2" fill="{pole}"/>'
            '<path d="M2.5 -68 C 16 -74, 24 -60, 40 -66 L40 -38 C 24 -32, 16 -46, 2.5 -40 Z" fill="#e01b24" '
            'stroke="#fff4de" stroke-width="2"/></g>')


def blossom(cx, cy, scale, rot, color, middle="#f2c14e"):
    s = f'<g transform="translate({cx} {cy}) rotate({rot}) scale({scale})">'
    for k in range(5):
        a = k * math.tau / 5
        px, py = 8 * math.cos(a), 8 * math.sin(a)
        s += (f'<ellipse cx="{px:.1f}" cy="{py:.1f}" rx="6.5" ry="5" fill="{color}" '
              f'transform="rotate({math.degrees(a):.0f} {px:.1f} {py:.1f})"/>')
    return s + f'<circle r="3.5" fill="{middle}"/></g>'


def striped_rects(stripes, x, y, w, h, vertical=False):
    """`stripes` filling the box -- to be clipped to a shape."""
    s = ""
    n = len(stripes)
    for i, c in enumerate(stripes):
        if vertical:
            s += f'<rect x="{x + i * w / n:.2f}" y="{y}" width="{w / n + 0.6:.2f}" height="{h}" fill="{c}"/>'
        else:
            s += f'<rect x="{x}" y="{y + i * h / n:.2f}" width="{w}" height="{h / n + 0.6:.2f}" fill="{c}"/>'
    return s


def hard_stops(stripes, grad_id, vertical=True):
    """A gradient of `stripes` with hard edges, top to bottom."""
    n = len(stripes)
    stops = ""
    for i, c in enumerate(stripes):
        stops += f'<stop offset="{i / n:.4f}" stop-color="{c}"/><stop offset="{(i + 1) / n:.4f}" stop-color="{c}"/>'
    x2, y2 = ("0", "1") if vertical else ("1", "0")
    return f'<linearGradient id="{grad_id}" x1="0" y1="0" x2="{x2}" y2="{y2}">{stops}</linearGradient>'


# --- 1. Belle Époque planet (the default) -----------------------------

GOLD, GOLD2, GOLD3 = "#c9a45c", "#f0d79a", "#8a6a2e"
INK, CREAM, CRIMSON, CRIMSON2 = "#14121a", "#efe4cc", "#a3221c", "#d4483a"


def nouveau_planet(v):
    sky = {
        "regular": ("#e0955a", "#7a3a3a", "#2a2238", INK),
        "workers": ("#f06a42", "#8e1b1b", "#3d0a0a", "#140303"),
        "christmas": ("#6f9db4", "#264d5e", "#132532", "#070d14"),
        "newyear": ("#4658a0", "#1a2550", "#0b1030", "#03050f"),
        "cny": ("#f6b04a", "#b3121b", "#5c070c", "#200204"),
    }.get(v.kind, ("#e0955a", "#7a3a3a", "#2a2238", INK))
    silver = v.kind == "christmas"
    metal = "url(#silver)" if silver else "url(#gold)"
    metal_light = "#f3f7fa" if silver else GOLD2
    planet_hi, planet_lo, bands = ("#f7fafc", "#9fb6c6", "#c6d4de") if silver else ("#fbf3e2", "#b48c5e", "#dcc6a0")
    label = {"newyear": "#e2b54a", "cny": "#d42a2a", "christmas": "#c01c28"}.get(v.kind, CRIMSON)

    defs = (
        f'<linearGradient id="gold" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{GOLD2}"/>'
        f'<stop offset="0.5" stop-color="{GOLD}"/><stop offset="1" stop-color="{GOLD3}"/></linearGradient>'
        '<linearGradient id="silver" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffffff"/>'
        '<stop offset="0.5" stop-color="#c9d6df"/><stop offset="1" stop-color="#7f95a5"/></linearGradient>'
        f'<radialGradient id="sky" cx="0.5" cy="0.88" r="0.95"><stop offset="0" stop-color="{sky[0]}"/>'
        f'<stop offset="0.35" stop-color="{sky[1]}"/><stop offset="0.72" stop-color="{sky[2]}"/>'
        f'<stop offset="1" stop-color="{sky[3]}"/></radialGradient>'
        f'<radialGradient id="planet" cx="0.32" cy="0.28" r="0.85"><stop offset="0" stop-color="{planet_hi}"/>'
        f'<stop offset="0.62" stop-color="{bands if silver else "#ead6b2"}"/><stop offset="1" stop-color="{planet_lo}"/></radialGradient>'
        f'<radialGradient id="halo"><stop offset="0.45" stop-color="{metal_light}" stop-opacity="0.35"/>'
        f'<stop offset="1" stop-color="{metal_light}" stop-opacity="0"/></radialGradient>'
        '<filter id="paint" x="-5%" y="-5%" width="110%" height="110%"><feTurbulence type="fractalNoise" '
        'baseFrequency="0.05" numOctaves="2" seed="4"/><feDisplacementMap in="SourceGraphic" scale="3" '
        'xChannelSelector="R" yChannelSelector="G"/></filter>'
        '<filter id="grain" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" '
        'baseFrequency="0.85" numOctaves="2" seed="9"/><feColorMatrix values="0 0 0 0 1  0 0 0 0 0.95  '
        '0 0 0 0 0.85  0 0 0 0.16 0"/></filter>'
        '<radialGradient id="shade"><stop offset="0.55" stop-color="#2a1a24" stop-opacity="0.32"/>'
        '<stop offset="1" stop-color="#2a1a24" stop-opacity="0"/></radialGradient>'
        '<clipPath id="sky-clip"><circle cx="128" cy="128" r="112"/></clipPath>'
        '<clipPath id="planet-clip"><circle cx="128" cy="136" r="50"/></clipPath>'
    )

    # Rings: three, metal, or the flag's stripes.
    if v.striped:
        ring_colors = visible(v.stripes, "#2a2238")
    else:
        ring_colors = [metal, metal_light, metal]
    n = len(ring_colors)
    radii = [100 - i * (22 / max(n - 1, 1)) for i in range(n)]
    width = max(2.4, min(5.0, 22 / max(n - 1, 1) * 0.8))

    def rings(sweep):
        return "".join(
            f'<path d="M{128 - r:.1f} 136 A{r:.1f} {r * 0.27:.1f} 0 0 {sweep} {128 + r:.1f} 136" fill="none" '
            f'stroke="{c}" stroke-width="{width:.1f}"/>' for r, c in zip(radii, ring_colors))

    # What drifts across the sky: petals, or the holiday's own.
    r = random.Random(3)
    stream = [(30, 74, 0.85, -22), (58, 52, 0.65, 8), (88, 38, 0.75, -30), (176, 50, 0.6, 18),
              (204, 78, 0.55, -12), (216, 182, 0.6, 28), (46, 190, 0.5, -40)]

    def petal(x, y, s, rot, c, edge):
        return (f'<g transform="translate({x} {y}) rotate({rot}) scale({s})">'
                f'<path d="M0 0 C 5 -9, 17 -12, 27 -3 C 20 5, 9 7, 0 0 Z" fill="{c}"/>'
                f'<path d="M2 -1 C 7 -8, 16 -10, 24 -4" fill="none" stroke="{edge}" stroke-width="1.6" '
                f'stroke-linecap="round" opacity="0.8"/></g>')

    def flecks(x, y, c):
        return "".join(f'<rect x="{x + 22 + k * 7:.0f}" y="{y - 3 + r.uniform(-4, 4):.0f}" width="{3.2 - k * 0.7:.1f}" '
                       f'height="{3.2 - k * 0.7:.1f}" fill="{c}" opacity="{0.85 - k * 0.25:.2f}"/>' for k in range(3))

    drift = ""
    behind = ""
    front = ""
    on_planet = ""
    if v.kind == "christmas":
        for x, y, s, _ in stream + [(150, 30, 0.5, 0), (100, 212, 0.5, 0)]:
            drift += snowflake(x + 8, y, 9 * s + 3, "#ffffff", 2.2)
        front = holly(128, 24, 1.35)
    elif v.kind == "newyear":
        behind = burst(66, 66, 30, GOLD2, 14, 2.4) + burst(194, 60, 24, "#f3f7fa", 14, 2.2) + burst(204, 186, 18, GOLD2, 12, 2)
        drift = confetti([(40, 160, GOLD2, 30), (70, 196, "#f3f7fa", -20), (180, 210, GOLD, 50),
                          (150, 30, GOLD2, -30), (110, 50, "#f3f7fa", 15), (220, 130, GOLD2, -45)])
    elif v.kind == "cny":
        for x, y, s, rot in stream[3:]:
            drift += blossom(x + 10, y, s * 0.9, rot, "#ffb3c6" if x % 2 else "#ff8fab", GOLD)
        front = lantern(58, 16, 0.85, GOLD)
    else:
        colors = [CRIMSON]
        edge = CRIMSON2
        if v.kind == "workers":
            colors = ["#d42a2a", CRIMSON]
            stream = stream + [(120, 26, 0.55, -8), (232, 120, 0.5, 30)]
            on_planet = (f'<rect x="126" y="62" width="4" height="34" fill="{GOLD}"/>'
                         f'<path d="M130 63 C 141 58, 147 70, 160 65 L160 85 C 147 90, 141 78, 130 83 Z" '
                         f'fill="#d42a2a" stroke="{GOLD2}" stroke-width="1.5"/>')
        elif v.striped:
            colors = [c for c in v.stripes if c not in ("#000000", "#2c2c2c", "#ffffff")] or v.stripes
            edge = "#ffffff90"
        for i, (x, y, s, rot) in enumerate(stream):
            c = colors[i % len(colors)]
            drift += petal(x, y, s, rot, c, edge)
            if i % 2 == 0:
                drift += flecks(x, y, c)

    # A few stars in the dark of the sky.
    rs = random.Random(5)
    stars = "".join(f'<circle cx="{rs.uniform(30, 226):.0f}" cy="{rs.uniform(24, 96):.0f}" r="{rs.choice([0.7, 1.0, 1.4])}" '
                    f'fill="{metal_light}" opacity="{rs.uniform(0.35, 0.85):.2f}"/>' for _ in range(22))

    # The frame: a rim, two lines, fleurons top and bottom.
    def fleuron(y, flip):
        sy = -1 if flip else 1
        return (f'<g transform="translate(128 {y}) scale(1 {sy})">'
                f'<path d="M0 -9 L6 0 L0 9 L-6 0 Z" fill="{metal}"/>'
                f'<path d="M-6 0 C -14 -2, -20 -8, -24 -2 C -20 0, -16 2, -10 2" fill="none" stroke="{metal}" stroke-width="2"/>'
                f'<path d="M6 0 C 14 -2, 20 -8, 24 -2 C 20 0, 16 2, 10 2" fill="none" stroke="{metal}" stroke-width="2"/></g>')

    dots = "".join(f'<circle cx="{128 + 116 * math.cos(a):.1f}" cy="{128 + 116 * math.sin(a):.1f}" r="3" fill="{metal}"/>'
                   for a in (0, math.pi, math.pi / 4, 3 * math.pi / 4, 5 * math.pi / 4, 7 * math.pi / 4))

    body = (
        '<circle cx="128" cy="128" r="124" fill="#1a1420"/>'
        '<circle cx="128" cy="128" r="112" fill="url(#sky)"/>'
        '<g clip-path="url(#sky-clip)">'
        '<rect x="16" y="16" width="224" height="224" fill="none" filter="url(#grain)"/>'
        f'{stars}{behind}'
        '<circle cx="128" cy="136" r="82" fill="url(#halo)"/>'
        '</g>'
        '<g transform="rotate(-16 128 136)">'
        f'{rings(1)}'
        '<circle cx="128" cy="136" r="50" fill="url(#planet)" filter="url(#paint)"/>'
        '<g clip-path="url(#planet-clip)">'
        f'<path d="M76 120 Q128 106 180 120 M74 146 Q128 134 182 146 M84 166 Q128 158 172 166" stroke="{bands}" '
        'stroke-width="6" fill="none" opacity="0.75"/>'
        '<circle cx="150" cy="158" r="62" fill="url(#shade)"/>'
        f'<path d="M{128 - 90} 148 A90 24 0 0 0 {128 + 90} 148" fill="none" stroke="#2a1a24" stroke-width="6" opacity="0.22"/>'
        '</g>'
        f'<path d="M92 112 A50 50 0 0 1 126 86" fill="none" stroke="{metal_light}" stroke-width="2.5" '
        'stroke-linecap="round" opacity="0.8"/>'
        f'<circle cx="128" cy="136" r="15.5" fill="none" stroke="{metal}" stroke-width="2"/>'
        f'<circle cx="128" cy="136" r="13" fill="{label}"/>'
        f'<circle cx="128" cy="136" r="8" fill="none" stroke="#00000033" stroke-width="1"/>'
        f'<circle cx="128" cy="136" r="2.5" fill="{INK}"/>'
        f'{on_planet}{rings(0)}'
        '</g>'
        f'<g clip-path="url(#sky-clip)">{drift}</g>'
        f'<circle cx="128" cy="128" r="116" fill="none" stroke="{metal}" stroke-width="5"/>'
        f'<circle cx="128" cy="128" r="109" fill="none" stroke="{metal}" stroke-width="1.2" opacity="0.7"/>'
        f'{dots}{fleuron(10, False)}{fleuron(246, True)}'
        f'{front}'
    )
    return svg(defs, body)



# --- 2. Vinyl on a blue tile -------------------------------------------

def vinyl_tile(v):
    bg = {"workers": ("#ff8a6b", "#d42a2a", "#5c0a14"), "christmas": ("#7be0a0", "#1f8a4c", "#0d3b22"),
          "newyear": ("#8f9bff", "#2a3a9a", "#0b1030"), "cny": ("#ffcf6b", "#e0352b", "#7a0a10")
          }.get(v.kind, ("#3ee6d9", "#2178d6", "#2a1f7a"))
    label = {"workers": ("#ff7b6b", "#c01c28"), "christmas": ("#ff6b6b", "#c01c28"),
             "newyear": ("#ffe08a", "#d9a520"), "cny": ("#ffe08a", "#d9a520")}.get(v.kind, ("#ffc061", "#f2683c"))
    defs = (f'<radialGradient id="bg" cx="0.3" cy="0.2" r="1.1"><stop offset="0" stop-color="{bg[0]}"/>'
            f'<stop offset="0.5" stop-color="{bg[1]}"/><stop offset="1" stop-color="{bg[2]}"/></radialGradient>'
            '<linearGradient id="disc" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2b2f3d"/>'
            '<stop offset="1" stop-color="#0d0f16"/></linearGradient>'
            f'<linearGradient id="label" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{label[0]}"/>'
            f'<stop offset="1" stop-color="{label[1]}"/></linearGradient>'
            '<linearGradient id="shine" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffffff" stop-opacity="0"/>'
            '<stop offset="0.45" stop-color="#ffffff" stop-opacity="0.22"/><stop offset="0.55" stop-color="#ffffff" stop-opacity="0"/></linearGradient>'
            '<filter id="sh" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="8" stdDeviation="8" '
            'flood-color="#0a0a30" flood-opacity="0.45"/></filter>'
            '<clipPath id="tile"><rect width="256" height="256" rx="58"/></clipPath>')
    if v.striped:
        stripes = visible(v.stripes, "#3a3f52")
        n = len(stripes)
        step = 34 / max(n - 1, 1)
        w = max(3.0, min(7.0, step * 0.8))
        grooves = "".join(f'<circle cx="118" cy="142" r="{80 - i * step:.1f}" fill="none" stroke="{c}" stroke-width="{w:.1f}"/>'
                          for i, c in enumerate(stripes))
    else:
        grooves = ('<g fill="none" stroke="#ffffff" stroke-opacity="0.08" stroke-width="2">'
                   + "".join(f'<circle cx="118" cy="142" r="{r}"/>' for r in (78, 68, 58, 48)) + '</g>')
    deco = ""
    hat = ""
    if v.kind == "workers":
        deco = red_flag(42, 100, 0.8)
    elif v.kind == "christmas":
        deco = snowflake(40, 44, 14, "#ffffff") + snowflake(222, 222, 10, "#ffffff") + snowflake(28, 96, 7, "#ffffff")
        hat = santa_hat(70, 74, 0.8, -32)
    elif v.kind == "newyear":
        deco = burst(46, 46, 26, "#ffe08a") + confetti([(30, 100, "#ff6fb5", 30), (220, 214, "#ffe08a", -20), (204, 236, "#6fd6ff", 40)])
        hat = party_hat(70, 74, 0.75, -32)
    elif v.kind == "cny":
        deco = lantern(42, 0, 0.8, "#ffe08a")
    body = ('<rect width="256" height="256" rx="58" fill="url(#bg)"/>'
            f'<g clip-path="url(#tile)">{deco}</g>'
            '<g filter="url(#sh)"><circle cx="118" cy="142" r="88" fill="url(#disc)"/></g>'
            f'{grooves}<circle cx="118" cy="142" r="88" fill="url(#shine)"/>'
            '<circle cx="118" cy="142" r="32" fill="url(#label)"/><circle cx="118" cy="142" r="5" fill="#0d0f16"/>'
            f'<g clip-path="url(#tile)">{hat}</g>'
            '<g transform="translate(160,30) rotate(-12)" fill="#ffffff" filter="url(#sh)">'
            '<rect x="12" y="-4" width="11" height="66" rx="5.5"/><path d="M17 -4 C 46 2, 54 22, 36 40 C 40 24, 32 16, 17 14 Z"/>'
            '<ellipse cx="6" cy="60" rx="18" ry="13" transform="rotate(-20 6 60)"/></g>')
    return svg(defs, body)


# --- 3. The enchanted music box -----------------------------------------

def enchanted_box(v):
    sky = {"workers": ("#8e1b1b", "#2a0505"), "christmas": ("#1f5c32", "#06140b"),
           "newyear": ("#1f2c66", "#03050f"), "cny": ("#b3121b", "#2a0306")}.get(v.kind, ("#3a2470", "#140b2e"))
    box = {"workers": ("#a51d2d", "#5c0a14"), "christmas": ("#b3121b", "#4a0a0a"),
           "newyear": ("#2c3a8a", "#141c4a"), "cny": ("#c01c28", "#5c070c")}.get(v.kind, ("#6a3d9a", "#3b1f5c"))
    gold = "#e8b84a"
    defs = ('<filter id="glow" x="-40%" y="-40%" width="180%" height="180%"><feGaussianBlur stdDeviation="5"/></filter>'
            f'<radialGradient id="night" cx="0.5" cy="0.35" r="0.7"><stop offset="0" stop-color="{sky[0]}"/>'
            f'<stop offset="1" stop-color="{sky[1]}"/></radialGradient>'
            '<linearGradient id="beam" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="#ffe27a" stop-opacity="0.7"/>'
            '<stop offset="1" stop-color="#ffe27a" stop-opacity="0"/></linearGradient>'
            '<clipPath id="disc"><circle cx="128" cy="128" r="116"/></clipPath>'
            '<clipPath id="front"><rect x="48" y="132" width="160" height="78" rx="10"/></clipPath>')
    sparkles = "".join(star4(x, y, r, "#fff4c2") for x, y, r in [(50, 62, 9), (206, 52, 7), (214, 120, 11), (40, 150, 6), (176, 34, 5)])
    hat = ""
    extra = ""
    if v.kind == "christmas":
        sparkles = "".join(snowflake(x, y, r, "#ffffff", 2.2) for x, y, r in [(50, 62, 10), (206, 52, 8), (214, 120, 11), (40, 150, 7), (180, 30, 6)])
        hat = santa_hat(127, 46, 0.55, 8)
    elif v.kind == "newyear":
        sparkles = burst(56, 60, 22, "#ffe27a") + burst(204, 58, 18, "#ff9be6") + confetti([(30, 150, "#ffe27a", 20), (220, 130, "#6fd6ff", -30), (180, 30, "#ff9be6", 40)])
        hat = party_hat(127, 46, 0.55, 10)
    elif v.kind == "cny":
        sparkles = lantern(50, 12, 0.7, gold) + blossom(210, 60, 0.8, 10, "#ffb3c6") + blossom(220, 130, 0.6, -20, "#ff8fab") + blossom(190, 30, 0.5, 30, "#ffb3c6")
    elif v.kind == "workers":
        extra = red_flag(80, 98, 0.62, gold)
    front_fill = f'<rect x="48" y="132" width="160" height="78" rx="10" fill="{box[0]}"/>'
    if v.striped:
        front_fill = f'<g clip-path="url(#front)">{striped_rects(v.stripes, 48, 132, 160, 78)}</g>'
    body = ('<circle cx="128" cy="128" r="116" fill="url(#night)"/>'
            f'<g clip-path="url(#disc)">{sparkles}</g>'
            '<path d="M70 132 L100 40 L156 40 L186 132 Z" fill="url(#beam)"/>'
            f'<g transform="translate(112,42) rotate(-8)" fill="#ffe27a" filter="url(#glow)">{NOTE}</g>'
            f'<g transform="translate(112,42) rotate(-8)" fill="#fffbe6">{NOTE}</g>{hat}'
            f'<path d="M52 134 L60 100 Q62 94 68 94 L188 94 Q194 94 196 100 L204 134 Z" fill="{box[1]}" stroke="{gold}" '
            'stroke-width="5" stroke-linejoin="round"/>'
            f'{extra}{front_fill}'
            f'<rect x="48" y="132" width="160" height="78" rx="10" fill="none" stroke="{gold}" stroke-width="5"/>'
            f'<path d="M48 156 L208 156" stroke="{gold}" stroke-width="4"/>'
            f'<circle cx="128" cy="180" r="14" fill="{gold}"/><path d="M128 172 L132 182 L124 182 Z" fill="{box[1]}"/>')
    return svg(defs, body)


# --- 4. Five bars ---------------------------------------------------------

def five_bars(v):
    face, lip = {"workers": ("#5c0a14", "#2e0509"), "christmas": ("#0d3b22", "#05200f"),
                 "newyear": ("#121a40", "#070b22"), "cny": ("#7a0a10", "#3d0508")}.get(v.kind, ("#3d3846", "#241f31"))
    grad = {"workers": ("#ffd27a", "#e01b24", "#a51d2d"), "christmas": ("#ffffff", "#ff6b6b", "#e01b24"),
            "newyear": ("#ffffff", "#ffe08a", "#d9a520"), "cny": ("#fff1b8", "#f2c14e", "#d9a520")
            }.get(v.kind, ("#c061cb", "#e01b24", "#ff7800"))
    if v.striped:
        fill_def = hard_stops(v.stripes, "bars").replace('x1="0" y1="0" x2="0" y2="1"',
                                                          'gradientUnits="userSpaceOnUse" x1="0" y1="40" x2="0" y2="200"')
    else:
        fill_def = (f'<linearGradient id="bars" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{grad[0]}"/>'
                    f'<stop offset="0.4" stop-color="{grad[1]}"/><stop offset="1" stop-color="{grad[2]}"/></linearGradient>')
    defs = fill_def + '<clipPath id="tile"><rect x="20" y="20" width="216" height="208" rx="40"/></clipPath>'
    bars = [(39, 116), (77, 64), (115, 40), (153, 88), (191, 132)]
    rects = "".join(f'<rect x="{x}" y="{y}" width="26" height="{200 - y}" rx="13" fill="url(#bars)"/>' for x, y in bars)
    deco = ""
    if v.kind == "christmas":
        deco = "".join(f'<ellipse cx="{x + 13}" cy="{y + 4}" rx="15" ry="7" fill="#ffffff"/>' for x, y in bars)
        deco += santa_hat(128, 42, 0.55, 6) + snowflake(48, 50, 10, "#ffffff") + snowflake(204, 56, 8, "#ffffff")
    elif v.kind == "newyear":
        deco = party_hat(128, 42, 0.6, 8) + burst(54, 60, 18, "#ffe08a") + burst(204, 60, 16, "#ff9be6")
    elif v.kind == "cny":
        deco = lantern(52, 20, 0.55)
    elif v.kind == "workers":
        deco = red_flag(166, 90, 0.55)
    body = ('<rect x="20" y="28" width="216" height="208" rx="40" fill="' + lip + '"/>'
            f'<rect x="20" y="20" width="216" height="208" rx="40" fill="{face}"/>'
            f'{rects}<g clip-path="url(#tile)">{deco}</g>')
    return svg(defs, body)


# --- 5. Neon ringed planet ------------------------------------------------

def neon_planet(v):
    sky = {"workers": ("#8e1b1b", "#3d0a0a", "#140303"), "christmas": ("#1f5c32", "#0d2b17", "#04100a"),
           "newyear": ("#1a2550", "#0b1030", "#03050f"), "cny": ("#b3121b", "#5c070c", "#200204")
           }.get(v.kind, ("#3b1f7a", "#121034", "#05060f"))
    planet = {"workers": ("#e0574a", "#5c0d0d"), "christmas": ("#3fae6a", "#0f4a2a"),
              "newyear": ("#4a6cff", "#14205c"), "cny": ("#e8352b", "#7a0a10")}.get(v.kind, ("#6a5cff", "#1a1250"))
    label = {"workers": "#ffd27a", "christmas": "#d42a2a", "newyear": "#f2c14e", "cny": "#f2c14e"}.get(v.kind, "#ff7800")
    if v.striped:
        rings = visible(v.stripes, "#1d1b2b")
    else:
        rings = {"workers": ["#ffd27a", "#ff9a56", "#ffd27a"], "christmas": ["#d42a2a", "#ffffff", "#d42a2a", "#ffffff"],
                 "newyear": ["#f2c14e", "#fff1b8", "#f2c14e"], "cny": ["#f2c14e", "#ffe08a", "#f2c14e"]
                 }.get(v.kind, ["#ff7ad9", "#c79bff", "#4de8ff", "#8a7dff"])
    n = len(rings)
    step = (118 - 80) / max(n - 1, 1)
    w = max(3.0, min(7.0, step * 0.75))
    ring_list = [(118 - i * step, c) for i, c in enumerate(rings)]

    def half(sweep):
        return "".join(f'<path d="M{128 - r:.1f} 132 A{r:.1f} {r * 0.28:.1f} 0 0 {sweep} {128 + r:.1f} 132" fill="none" '
                       f'stroke="{c}" stroke-width="{w:.1f}"/>' for r, c in ring_list)

    rs = random.Random(7)
    stars = ""
    for _ in range(46):
        a = rs.uniform(0, math.tau)
        d = math.sqrt(rs.uniform(0, 1)) * 112
        stars += (f'<circle cx="{128 + d * math.cos(a):.0f}" cy="{128 + d * math.sin(a):.0f}" r="{rs.choice([0.8, 1.3, 1.8])}" '
                  f'fill="#ffffff" opacity="{rs.uniform(.4, 1):.2f}"/>')
    behind = front = on_planet = ""
    if v.kind == "workers":
        on_planet = ('<rect x="126" y="44" width="5" height="34" fill="#e8d3b0"/>'
                     '<path d="M131 46 C 145 41, 152 53, 168 48 L168 70 C 152 75, 145 63, 131 68 Z" fill="#e23030" '
                     'stroke="#fff4de" stroke-width="2"/>')
    elif v.kind == "christmas":
        behind = "".join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="#ffffff" opacity="0.9"/>' for x, y, r in
                         [(40, 60, 4), (70, 30, 3), (196, 40, 4), (222, 90, 3), (30, 120, 3), (220, 170, 4), (60, 200, 3), (180, 220, 3), (110, 22, 3)])
        on_planet = santa_hat(126, 90, 0.95, -6)
    elif v.kind == "newyear":
        behind = burst(56, 64, 34, "#f2c14e") + burst(196, 54, 26, "#ff6fb5") + burst(206, 184, 20, "#6fd6ff")
    elif v.kind == "cny":
        front = lantern(56, 0, 1.0)
    defs = ('<filter id="glow" x="-40%" y="-40%" width="180%" height="180%"><feGaussianBlur stdDeviation="4"/></filter>'
            f'<radialGradient id="sky" cx="0.35" cy="0.3" r="0.9"><stop offset="0" stop-color="{sky[0]}"/>'
            f'<stop offset="0.55" stop-color="{sky[1]}"/><stop offset="1" stop-color="{sky[2]}"/></radialGradient>'
            f'<radialGradient id="pl" cx="0.35" cy="0.3" r="0.8"><stop offset="0" stop-color="{planet[0]}"/>'
            f'<stop offset="1" stop-color="{planet[1]}"/></radialGradient>'
            '<clipPath id="disc"><circle cx="128" cy="128" r="120"/></clipPath>')
    glow = "" if v.kind == "flag" else f'<g filter="url(#glow)" opacity="0.55">{half(0)}</g>'
    body = ('<circle cx="128" cy="128" r="120" fill="url(#sky)"/>'
            f'<g clip-path="url(#disc)">{stars}{behind}</g>'
            f'<g transform="rotate(-18 128 132)">{half(1)}'
            '<circle cx="128" cy="132" r="58" fill="url(#pl)"/>'
            '<path d="M78 112 Q128 100 178 112 M72 136 Q128 124 184 136 M80 158 Q128 148 176 158" stroke="#ffffff" '
            'stroke-opacity="0.12" stroke-width="6" fill="none"/>'
            f'<circle cx="128" cy="132" r="16" fill="{label}"/><circle cx="128" cy="132" r="3" fill="#05060f"/>'
            f'{on_planet}{glow}{half(0)}</g>'
            f'<g clip-path="url(#disc)">{front}</g>')
    return svg(defs, body)


# --- 6. Turntable ---------------------------------------------------------

def turntable(v):
    plinth, plinth_lip, plinth_hi = {"cny": ("#c01c28", "#7a0a10", "#ff6b6b"), "christmas": ("#f6f5f4", "#9a9996", "#ffffff")
                                     }.get(v.kind, ("#deddda", "#9a9996", "#f6f5f4"))
    label = {"workers": "#e01b24", "christmas": "#c01c28", "newyear": "#f2c14e", "cny": "#f2c14e"}.get(v.kind, "#ff7800")
    if v.striped:
        stripes = visible(v.stripes, "#4a4656")
        n = len(stripes)
        step = 40 / max(n - 1, 1)
        w = max(2.6, min(6.0, step * 0.8))
        grooves = "".join(f'<circle cx="108" cy="127" r="{70 - i * step:.1f}" fill="none" stroke="{c}" stroke-width="{w:.1f}"/>'
                          for i, c in enumerate(stripes))
    else:
        grooves = '<g fill="none" stroke="#5e5c64" stroke-width="2">' + "".join(
            f'<circle cx="108" cy="127" r="{r}"/>' for r in (66, 56, 46)) + '</g>'
    on_pivot = deco = ""
    light = '<circle cx="214" cy="200" r="9" fill="#ed333b"/>'
    if v.kind == "workers":
        deco = red_flag(214, 206, 0.6)
        light = ""
    elif v.kind == "christmas":
        on_pivot = santa_hat(204, 52, 0.5, 10)
        deco = holly(46, 206, 0.7) + '<path d="M30 33 Q60 26 90 33 Q120 28 150 33 Q180 26 226 33 L226 40 L30 40 Z" fill="#ffffff"/>'
    elif v.kind == "newyear":
        on_pivot = party_hat(206, 50, 0.5, 10)
        deco = confetti([(36, 190, "#ff6fb5", 30), (60, 210, "#f2c14e", -20), (180, 214, "#6fd6ff", 40), (150, 200, "#f2c14e", 10)])
    elif v.kind == "cny":
        deco = lantern(40, 0, 0.55) + blossom(200, 210, 0.7, 10, "#ffb3c6")
        light = ""
    body = (f'<rect x="18" y="40" width="220" height="190" rx="22" fill="{plinth_lip}"/>'
            f'<rect x="18" y="32" width="220" height="190" rx="22" fill="{plinth}"/>'
            f'<rect x="36" y="32" width="184" height="4" rx="2" fill="{plinth_hi}"/>'
            '<circle cx="108" cy="127" r="82" fill="#c0bfbc"/><circle cx="108" cy="127" r="76" fill="#241f31"/>'
            f'{grooves}'
            '<path d="M108 127 L126 53 A76 76 0 0 1 162 73 Z" fill="#fff" opacity="0.10"/>'
            f'<circle cx="108" cy="127" r="25" fill="{label}"/><circle cx="108" cy="127" r="4" fill="#241f31"/>'
            '<path d="M206 62 L206 150 L170 182" fill="none" stroke="#77767b" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/>'
            '<rect x="156" y="172" width="22" height="16" rx="4" fill="#3d3846" transform="rotate(-42 167 180)"/>'
            '<circle cx="206" cy="62" r="16" fill="#9a9996"/><circle cx="206" cy="62" r="8" fill="#5e5c64"/>'
            f'{light}{deco}{on_pivot}')
    return svg("", body)


# --- 7. Idol pop -----------------------------------------------------------

OUTLINE = "#1d1b2b"


def idol_pop(v):
    base, light = {"workers": ("#e01b24", "#ff4d4d"), "christmas": ("#26a269", "#33d17a"),
                   "newyear": ("#1c3a8a", "#2a50b8"), "cny": ("#c01c28", "#e0352b")}.get(v.kind, ("#2ec4b6", "#3fd9cb"))
    dots = "".join(f'<circle cx="{x}" cy="{y}" r="{2 + (x + y) % 3}" fill="#ffffff" opacity="0.25"/>'
                   for x in range(150, 246, 14) for y in range(150, 246, 14) if (x - 128) ** 2 + (y - 128) ** 2 < 108 ** 2)
    if v.striped:
        face = f'<g clip-path="url(#disc)">{striped_rects(v.stripes, 16, 16, 224, 224)}</g>'
    else:
        face = (f'<circle cx="128" cy="128" r="112" fill="{base}"/>'
                f'<path d="M128 16 A112 112 0 0 1 240 128 L128 128 Z" fill="{light}"/>{dots}')
    stick = ('<path d="M54 214 L86 150" stroke="#ff4f9a" stroke-width="14" stroke-linecap="round" stroke-opacity="0.35"/>'
             '<path d="M58 206 L84 154" stroke="#ff4f9a" stroke-width="9" stroke-linecap="round"/>'
             f'<path d="M50 222 L56 210" stroke="{OUTLINE}" stroke-width="11" stroke-linecap="round"/>')
    sparkles = star4(196, 70, 26, "#ff4f9a") + star4(64, 82, 16, "#ffffff") + star4(206, 184, 12, "#ffffff")
    hat = ""
    if v.kind == "christmas":
        stick = ('<path d="M60 214 L78 150 C 82 134, 100 134, 102 150" fill="none" stroke="#ffffff" stroke-width="12" stroke-linecap="round"/>'
                 '<path d="M60 214 L78 150 C 82 134, 100 134, 102 150" fill="none" stroke="#e01b24" stroke-width="12" '
                 'stroke-linecap="round" stroke-dasharray="8 8"/>')
        sparkles = snowflake(196, 70, 22, "#ffffff", 3) + snowflake(64, 82, 13, "#ffffff") + snowflake(206, 184, 11, "#ffffff")
        hat = santa_hat(116, 58, 0.72, 10)
    elif v.kind == "newyear":
        sparkles = burst(196, 70, 26, "#ffe08a") + burst(64, 82, 16, "#ff9be6") + burst(206, 184, 13, "#6fd6ff")
        hat = party_hat(116, 58, 0.72, 14)
        stick = stick.replace("#ff4f9a", "#ffe08a")
    elif v.kind == "cny":
        stick = ""
        sparkles = lantern(66, 16, 0.75) + blossom(196, 70, 1.1, 10, "#ffb3c6") + blossom(206, 184, 0.8, -20, "#ff8fab")
    elif v.kind == "workers":
        stick = red_flag(60, 214, 0.95, "#fff4de")
        sparkles = star4(196, 70, 26, "#ffd27a") + star4(206, 184, 12, "#ffffff")
    defs = '<clipPath id="disc"><circle cx="128" cy="128" r="112"/></clipPath>'
    body = (f'<circle cx="128" cy="128" r="112" fill="{base}"/>{face}'
            f'<g clip-path="url(#disc)">{sparkles}</g>'
            f'<g transform="translate(84,52) scale(1.6)" fill="#ffffff" stroke="{OUTLINE}" stroke-width="5" paint-order="stroke">{NOTE}</g>'
            f'{hat}{stick}'
            f'<circle cx="128" cy="128" r="112" fill="none" stroke="{OUTLINE}" stroke-width="8"/>')
    return svg(defs, body)


ICONS = [
    # (id, name in Preferences, drawing -- None: kept as it is)
    ("nouveau-planet", "Belle Époque", nouveau_planet),
    ("vinyl", "Vinyl", vinyl_tile),
    ("enchanted-box", "Enchanted box", enchanted_box),
    ("music-box", "Original", None),
    ("five-bars", "Five bars", five_bars),
    ("neon-planet", "Neon planet", neon_planet),
    ("turntable", "Turntable", turntable),
    ("idol-pop", "Idol pop", idol_pop),
]


def write_table():
    lines = [
        "//! Generated by `compile/logos/generate.py` -- don't edit by hand.",
        "//! Every icon to choose from (Preferences > Appearance), each with",
        "//! its versions: `regular`, the holidays and `flag-01`..`flag-12`.",
        "",
        "/// An icon: its id (saved in the settings), its name, and its",
        "/// versions as SVG, in `VERSIONS` order.",
        "pub struct LogoIcon {",
        "    pub id: &'static str,",
        "    pub name: &'static str,",
        "    pub versions: [&'static str; 18],",
        "}",
        "",
        "/// The versions, in the order every icon lists them.",
        "pub const VERSIONS: [&str; 18] = [",
    ]
    lines += [f'    "{key}",' for key in VERSIONS]
    lines += ["];", "", "pub const ICONS: &[LogoIcon] = &["]
    for icon_id, name, _ in ICONS:
        lines += ["    LogoIcon {", f'        id: "{icon_id}",', f'        name: "{name}",', "        versions: ["]
        for key in VERSIONS:
            path = os.path.join(OUT, icon_id, f"{key}.svg")
            if not os.path.exists(path):
                raise SystemExit(f"missing {path}")
            lines.append(f'            include_str!("../logos/{icon_id}/{key}.svg"),')
        lines += ["        ],", "    },"]
    lines += ["];", ""]
    with open(TABLE, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


def main():
    only = os.environ.get("LOGO_ONLY")
    for icon_id, _, draw in ICONS:
        if draw is None or (only and only != icon_id):
            continue
        os.makedirs(os.path.join(OUT, icon_id), exist_ok=True)
        for key in VERSIONS:
            with open(os.path.join(OUT, icon_id, f"{key}.svg"), "w", encoding="utf-8") as f:
                f.write(draw(Version(key)))
    write_table()


if __name__ == "__main__":
    main()
