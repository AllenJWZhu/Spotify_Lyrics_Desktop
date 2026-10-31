"""Draws the installer artwork, in the app icon's colours.

    python3 crates/app/installer/make_art.py

Needs Pillow, fontTools and brotli (to read the bundled woff2 font). Writes
sidebar.bmp (Welcome/Finish pages, 164x314) and header.bmp (top-right of the
other pages, 150x57) next to this script. NSIS wants 24-bit BMPs at exactly
these sizes; everything is drawn at 4x and scaled down for clean edges.
"""

import io
from pathlib import Path

from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
APP = HERE.parent
FONT_WOFF2 = APP / "ui" / "fonts" / "instrument-sans-latin-standard-normal.woff2"
ICON = APP / "icons" / "icon.png"

X = 4  # supersampling factor

# The icon's palette.
INDIGO = (49, 46, 129)
NIGHT = (15, 23, 42)
CYAN = (94, 208, 255)
WHITE = (255, 255, 255)


def font(size, weight):
    woff = TTFont(FONT_WOFF2)
    woff.flavor = None
    ttf = io.BytesIO()
    woff.save(ttf)
    ttf.seek(0)
    f = ImageFont.truetype(ttf, size * X)
    f.set_variation_by_axes([100, weight])
    return f


def gradient(w, h, top, bottom):
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    for y in range(h):
        t = y / (h - 1)
        d.line([(0, y), (w, y)], fill=tuple(round(top[i] * (1 - t) + bottom[i] * t) for i in range(3)))
    return img


def glow(size, center, radius, color, alpha):
    """A soft round light, as an RGBA layer."""
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    cx, cy = center
    ImageDraw.Draw(layer).ellipse(
        [cx - radius, cy - radius, cx + radius, cy + radius], fill=color + (alpha,)
    )
    return layer.filter(ImageFilter.GaussianBlur(radius * 0.6))


def bar(layer, x, y, w, h, fill):
    ImageDraw.Draw(layer).rounded_rectangle([x, y, x + w, y + h], radius=h // 2, fill=fill)


def sidebar():
    w, h = 164 * X, 314 * X
    img = gradient(w, h, INDIGO, NIGHT).convert("RGBA")
    img.alpha_composite(glow((w, h), (int(w * 0.25), int(h * 0.74)), 64 * X, CYAN, 38))
    img.alpha_composite(glow((w, h), (int(w * 0.95), int(h * 0.08)), 60 * X, (129, 140, 248), 55))

    # Wordmark.
    d = ImageDraw.Draw(img)
    left = 18 * X
    title = font(25, 640)
    d.text((left, 26 * X), "Lyrics", font=title, fill=WHITE)
    d.text((left, 54 * X), "Overlay", font=title, fill=WHITE)
    d.text((left, 90 * X), "Floating desktop lyrics", font=font(11, 480), fill=WHITE + (170,))

    # The icon's motif, larger: a line being sung, then the lines to come.
    lines = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    y0, bh = 214 * X, 13 * X
    bar(lines, left, y0, 124 * X, bh, WHITE + (235,))
    bar(lines, left, y0, 74 * X, bh, CYAN + (255,))
    bar(lines, left, y0 + 25 * X, 92 * X, 10 * X, WHITE + (120,))
    bar(lines, left, y0 + 46 * X, 108 * X, 10 * X, WHITE + (55,))
    # Light from the sung part.
    halo = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    bar(halo, left - 2 * X, y0 - 2 * X, 78 * X, bh + 4 * X, CYAN + (220,))
    img.alpha_composite(halo.filter(ImageFilter.GaussianBlur(4 * X)))
    img.alpha_composite(lines)

    return img.convert("RGB").resize((164, 314), Image.LANCZOS)


def header():
    w, h = 150 * X, 57 * X
    img = Image.new("RGBA", (w, h), WHITE + (255,))
    size = 42 * X
    icon = Image.open(ICON).convert("RGBA").resize((size, size), Image.LANCZOS)
    img.alpha_composite(icon, (w - size - 10 * X, (h - size) // 2))
    return img.convert("RGB").resize((150, 57), Image.LANCZOS)


if __name__ == "__main__":
    sidebar().save(HERE / "sidebar.bmp")
    header().save(HERE / "header.bmp")
    print("wrote", HERE / "sidebar.bmp", HERE / "header.bmp")
