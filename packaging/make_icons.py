#!/usr/bin/env python3
"""Generates every Pitlane icon from one drawing (needs Pillow):
  crates/pitlane-app/android/res/...   Android launcher icons (legacy + adaptive layers)
  crates/pitlane-app/assets/icon.png   window / AppImage icon (256 px)
Run from the repository root: python3 packaging/make_icons.py
"""
import math, os
from PIL import Image, ImageDraw

SS = 4  # supersampling factor for smooth edges
BG_TOP, BG_BOTTOM = (18, 35, 63), (29, 79, 145)
ARC, REDLINE, NEEDLE, HUB = (232, 238, 247), (255, 107, 53), (255, 107, 53), (232, 238, 247)

def gradient(size):
    im = Image.new("RGBA", (size, size))
    d = ImageDraw.Draw(im)
    for y in range(size):
        t = y / max(1, size - 1)
        d.line([(0, y), (size, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)) + (255,))
    return im

def gauge(size, radius_frac):
    """Tachometer: 270-degree arc, red-line zone, ticks, needle, hub. Transparent background."""
    S = size * SS
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    r = S * radius_frac
    w = r * 0.20
    cx, cy = S / 2, S / 2 + r * 0.14  # the arc is open at the bottom: sit a little low to look centred
    box = [cx - r, cy - r, cx + r, cy + r]
    # PIL angles: 0 = 3 o'clock, clockwise. Gauge runs from 135 deg (bottom-left) to 405 deg (bottom-right).
    d.arc(box, 135, 345, fill=ARC, width=int(w))
    d.arc(box, 348, 405, fill=REDLINE, width=int(w))
    # ticks inside the arc
    for i in range(10):
        a = math.radians(135 + i * 30)
        r1, r2 = r - w * 1.35, r - w * (2.0 if i % 2 == 0 else 1.75)
        col = REDLINE if i >= 8 else ARC
        d.line([(cx + r1 * math.cos(a), cy + r1 * math.sin(a)), (cx + r2 * math.cos(a), cy + r2 * math.sin(a))], fill=col, width=int(w * 0.32))
    # needle pointing high in the rev range
    a = math.radians(135 + 225)
    tip = (cx + (r - w * 1.2) * math.cos(a), cy + (r - w * 1.2) * math.sin(a))
    nw = w * 0.42
    perp = (math.cos(a + math.pi / 2) * nw, math.sin(a + math.pi / 2) * nw)
    tail = (cx - (r * 0.18) * math.cos(a), cy - (r * 0.18) * math.sin(a))
    d.polygon([(tail[0] + perp[0], tail[1] + perp[1]), tip, (tail[0] - perp[0], tail[1] - perp[1])], fill=NEEDLE)
    hr = w * 0.75
    d.ellipse([cx - hr, cy - hr, cx + hr, cy + hr], fill=HUB)
    return im.resize((size, size), Image.LANCZOS)

def legacy(size, round_=False):
    """Pre-Android-8 launcher icon: gradient tile (rounded square or circle) with the gauge."""
    S = size * SS
    tile = gradient(S)
    mask = Image.new("L", (S, S), 0)
    m = ImageDraw.Draw(mask)
    inset = S * 0.04
    if round_:
        m.ellipse([inset, inset, S - inset, S - inset], fill=255)
    else:
        m.rounded_rectangle([inset, inset, S - inset, S - inset], radius=S * 0.22, fill=255)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(tile, (0, 0), mask)
    out = out.resize((size, size), Image.LANCZOS)
    out.alpha_composite(gauge(size, 0.31))
    return out

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RES = os.path.join(ROOT, "crates/pitlane-app/android/res")
DENSITIES = {"mdpi": 1.0, "hdpi": 1.5, "xhdpi": 2.0, "xxhdpi": 3.0, "xxxhdpi": 4.0}
for name, k in DENSITIES.items():
    d = os.path.join(RES, f"mipmap-{name}")
    os.makedirs(d, exist_ok=True)
    legacy(int(48 * k)).save(os.path.join(d, "ic_launcher.png"), optimize=True)
    legacy(int(48 * k), round_=True).save(os.path.join(d, "ic_launcher_round.png"), optimize=True)
    # adaptive layers are 108dp; the launcher mask shows the central 72dp, keep content in 66dp
    a = int(108 * k)
    gradient(a).save(os.path.join(d, "ic_launcher_background.png"), optimize=True)
    gauge(a, 0.27).save(os.path.join(d, "ic_launcher_foreground.png"), optimize=True)
os.makedirs(os.path.join(RES, "mipmap-anydpi-v26"), exist_ok=True)
xml = """<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
</adaptive-icon>
"""
for n in ("ic_launcher.xml", "ic_launcher_round.xml"):
    open(os.path.join(RES, "mipmap-anydpi-v26", n), "w").write(xml)
legacy(256).save(os.path.join(ROOT, "crates/pitlane-app/assets/icon.png"), optimize=True)
print("icons written to", RES)
