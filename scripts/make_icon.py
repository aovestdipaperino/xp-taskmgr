#!/usr/bin/env python3
"""Draws the Task Manager icon: an XP style CRT and desktop case with a heartbeat.

Usage: make_icon.py <out.iconset dir>

Renders at 4x and downsamples for clean anti-aliasing, then writes every
size macOS expects in an .iconset; `iconutil -c icns` turns it into .icns.
"""

import os
import sys

from PIL import Image, ImageDraw, ImageFilter

SS = 4  # supersampling factor
BASE = 1024


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def vertical_gradient(size, top, bottom):
    w, h = size
    grad = Image.new("RGBA", (1, h))
    for y in range(h):
        grad.putpixel((0, y), lerp(top, bottom, y / max(1, h - 1)))
    return grad.resize((w, h))


def rounded_mask(size, box, radius):
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).rounded_rectangle(box, radius=radius, fill=255)
    return mask


def paste_gradient(img, box, radius, top, bottom):
    x0, y0, x1, y1 = box
    grad = vertical_gradient((x1 - x0, y1 - y0), top, bottom)
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    layer.paste(grad, (x0, y0))
    img.alpha_composite(Image.composite(layer, Image.new("RGBA", img.size), rounded_mask(img.size, box, radius)))


def poly(d, pts, fill, outline, width):
    d.polygon(pts, fill=fill)
    d.line(pts + [pts[0]], fill=outline, width=width, joint="curve")


def draw_icon():
    """The XP Task Manager icon: a CRT on a desktop case, heartbeat on screen."""
    s = BASE * SS
    u = s / 1024
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    def p(x, y):
        return (int(x * u), int(y * u))

    outline = (64, 64, 60, 255)
    ow = int(10 * u)

    # Soft shadow under the case.
    shadow = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).ellipse([*p(110, 760), *p(940, 900)], fill=(0, 0, 0, 120))
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(30 * u)))

    d = ImageDraw.Draw(img)
    # Desktop case: top, right side, then the front face.
    poly(d, [p(120, 650), p(210, 580), p(910, 580), p(840, 650)], (238, 236, 228, 255), outline, ow)
    poly(d, [p(840, 650), p(910, 580), p(910, 750), p(840, 830)], (150, 148, 140, 255), outline, ow)
    paste_gradient(img, [*p(120, 650), *p(840, 830)], 0, (222, 220, 212, 255), (178, 176, 168, 255))
    d = ImageDraw.Draw(img)
    d.rectangle([*p(120, 650), *p(840, 830)], outline=outline, width=ow)
    # Power LED and the floppy slot.
    d.ellipse([*p(170, 715), *p(214, 759)], fill=(40, 230, 40, 255), outline=(20, 90, 20, 255), width=int(5 * u))
    d.rounded_rectangle([*p(560, 722), *p(790, 752)], radius=int(8 * u), fill=(40, 40, 38, 255))
    d.rectangle([*p(740, 704), *p(790, 716)], fill=(90, 90, 86, 255))

    # Monitor: top, right side, then the bezel.
    poly(d, [p(220, 170), p(290, 110), p(840, 110), p(770, 170)], (240, 238, 230, 255), outline, ow)
    poly(d, [p(770, 170), p(840, 110), p(840, 520), p(770, 590)], (150, 148, 140, 255), outline, ow)
    paste_gradient(img, [*p(220, 170), *p(770, 590)], 0, (230, 228, 220, 255), (186, 184, 176, 255))
    d = ImageDraw.Draw(img)
    d.rectangle([*p(220, 170), *p(770, 590)], outline=outline, width=ow)

    # Screen: sunken edge (dark top/left, light bottom/right) around black glass.
    d.rectangle([*p(272, 218), *p(718, 540)], fill=(250, 250, 246, 255))
    d.rectangle([*p(272, 218), *p(712, 534)], fill=(110, 110, 104, 255))
    screen = [*p(286, 232), *p(706, 528)]
    d.rectangle(screen, fill=(0, 0, 0, 255))

    # Faint grid, as on the Performance tab.
    x0, y0, x1, y1 = screen
    step = int(60 * u)
    for x in range(x0 + step, x1, step):
        d.line([(x, y0), (x, y1)], fill=(0, 70, 35, 255), width=int(4 * u))
    for y in range(y0 + step, y1, step):
        d.line([(x0, y), (x1, y)], fill=(0, 70, 35, 255), width=int(4 * u))

    # The heartbeat: flat, a small bump, a tall spike, a dip, then flat again.
    mid = 0.55
    trace = [(0.00, mid), (0.30, mid), (0.36, mid - 0.10), (0.41, mid), (0.46, mid),
             (0.52, 0.10), (0.59, 0.92), (0.65, mid - 0.08), (0.70, mid), (1.00, mid)]
    pad = int(14 * u)
    line = [(x0 + pad + tx * (x1 - x0 - 2 * pad), y0 + pad + ty * (y1 - y0 - 2 * pad)) for tx, ty in trace]
    glow = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(glow).line(line, fill=(0, 255, 0, 150), width=int(40 * u), joint="curve")
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(14 * u)))
    ImageDraw.Draw(img).line(line, fill=(0, 255, 0, 255), width=int(20 * u), joint="curve")

    return img.resize((BASE, BASE), Image.LANCZOS)


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    icon = draw_icon()
    for size in (16, 32, 128, 256, 512):
        icon.resize((size, size), Image.LANCZOS).save(os.path.join(out, f"icon_{size}x{size}.png"))
        icon.resize((size * 2, size * 2), Image.LANCZOS).save(os.path.join(out, f"icon_{size}x{size}@2x.png"))


if __name__ == "__main__":
    main()
