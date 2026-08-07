"""Render the Mica mark into windows/runner/resources/app_icon.ico.

The Windows app icon was still Flutter's default (the light-blue swirl) while
the app drew its own mark everywhere else. This makes the third place agree with
the other two instead of inventing a fourth design.

`lib/widgets/mica_logo.dart` is AUTHORITATIVE for the geometry and the palette;
`web/favicon.svg` already mirrors it, and so does this. Three mirrors of one
formula is two too many, but none of them can import the others: the tab icon
must exist before Flutter boots, and an .ico is a binary the build cannot draw.
So the rule is the same one the Markdown dialect uses — one source, and every
copy says out loud what it copies.

    python tool/gen_app_icon.py

Committed next to the .ico it produces: a binary asset with no source beside it
is the thing nobody dares touch five months later.
"""

from __future__ import annotations

import pathlib

from PIL import Image, ImageDraw

# --- mirrored from _MicaLogoPainter (lib/widgets/mica_logo.dart) -------------
# Bottom -> top: light to deep blue, so the stack reads as layered sheets.
LAYERS = ["#93C5FD", "#3B82F6", "#1D4ED8"]
HALF_W = 0.44   # half width of a sheet, as a fraction of the box
HALF_H = 0.20   # half height
GAP = 0.165     # vertical offset between layers
# ----------------------------------------------------------------------------

# Windows picks a frame by size; 16/32/48/256 are what the shell actually asks
# for (and what the default icon shipped). 24/64/128 are cheap and stop the
# shell from downscaling 256 for mid-DPI taskbars.
SIZES = [16, 24, 32, 48, 64, 128, 256]

# Draw big, then downsample: the sheets are thin diagonals, and rasterising
# those directly at 16px gives stair-stepped edges no amount of care fixes.
SUPERSAMPLE = 8


def render(box: int) -> Image.Image:
    w = h = box * SUPERSAMPLE
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    cx = w / 2
    hw = w * HALF_W
    hh = h * HALF_H
    gap = h * GAP
    base_cy = h * 0.5 + gap

    # Bottom-most first, so the upper sheets overlap it (the painter's order).
    for i, colour in enumerate(LAYERS):
        cy = base_cy - i * gap
        draw.polygon(
            [(cx, cy - hh), (cx + hw, cy), (cx, cy + hh), (cx - hw, cy)],
            fill=colour,
        )

    return img.resize((box, box), Image.LANCZOS)


def main() -> None:
    root = pathlib.Path(__file__).resolve().parent.parent
    out = root / "windows" / "runner" / "resources" / "app_icon.ico"
    frames = [render(s) for s in SIZES]
    # Every frame is drawn at its own scale and handed over explicitly; letting
    # the encoder derive them from the 256 would undo the point of supersampling
    # each size separately.
    frames[-1].save(
        out,
        format="ICO",
        sizes=[(s, s) for s in SIZES],
        append_images=frames[:-1],
    )
    print(f"wrote {out} ({out.stat().st_size} bytes) sizes={SIZES}")


if __name__ == "__main__":
    main()
