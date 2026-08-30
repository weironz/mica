#!/usr/bin/env python3
"""Pack the per-size PNGs exported by the Flutter painter into the two .ico files.

The PNGs come from `clients/mica_flutter/test/tmp_icon_export_test.dart`, which
rasterises the SHIPPING `MicaLogoPainter` — so the taskbar icon, the tray icon
and the in-app mark are one geometry by construction rather than by discipline.
That matters: the tray spent 2026-07-20 .. 2026-08-28 showing the stock Flutter
logo because it was a second copy nobody remembered to replace.

Each size is its own artwork, not a downscale of the 256: the painter switches
to a simplified variant below 22px (no corner nodes, relatively heavier stroke),
which is the only reason the mark survives a 16px tray slot. Downscaling the big
one instead would put the blob back.

Usage (see scripts/gen-icons.sh for the full flow):
    python scripts/gen-icons.py
"""
import pathlib
import sys

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "clients/mica_flutter/test/icon_src"
TARGETS = [
    ROOT / "clients/mica_flutter/windows/runner/resources/app_icon.ico",
    ROOT / "clients/mica_flutter/assets/tray_icon.ico",
]
SIZES = [16, 24, 32, 48, 64, 128, 256]


def main() -> int:
    frames = []
    for size in SIZES:
        path = SRC / f"{size}.png"
        if not path.exists():
            print(f"missing {path} — run the export test first", file=sys.stderr)
            return 1
        image = Image.open(path).convert("RGBA")
        if image.size != (size, size):
            print(f"{path} is {image.size}, expected {(size, size)}", file=sys.stderr)
            return 1
        # A fully transparent frame is the failure this cannot afford to pass
        # through. On 2026-08-30 the 128px golden captured an Image.asset before
        # it had decoded and came out BLANK; every check upstream was green, the
        # blank went into app_icon.ico, and it was caught only because somebody
        # opened the file to look at it. Cheap to assert, invisible otherwise.
        if not image.getchannel("A").getbbox():
            print(f"{path} is fully transparent — nothing to pack", file=sys.stderr)
            return 1
        frames.append(image)

    # Pillow writes one .ico containing every frame; `append_images` keeps the
    # per-size artwork instead of resampling the first one.
    #
    # The BIGGEST frame has to be the base: Pillow only emits sizes it can reach
    # from it, so passing the 16px first produced a one-frame .ico (16 only) and
    # every larger slot fell back to Windows upscaling that 16px art.
    frames.sort(key=lambda im: im.size[0], reverse=True)
    base, rest = frames[0], frames[1:]
    for target in TARGETS:
        target.parent.mkdir(parents=True, exist_ok=True)
        base.save(
            target,
            format="ICO",
            sizes=[(s, s) for s in SIZES],
            append_images=rest,
        )
        print(f"wrote {target.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
