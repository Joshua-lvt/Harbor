"""Generate Harbor Tauri icons (32x32.png, 128x128.png, icon.ico).

A small rounded-square app tile in the ocean palette with a stylized shark
silhouette, matching the in-app SharkMascot. Run from the repo root:

    python server/scripts/make_icons.py

Writes into client/src-tauri/icons/. Requires Pillow.
"""
from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw

# Harbor palette (see client/src/style.css).
MIST = (247, 250, 252)
SKY = (190, 227, 248)
ICE = (144, 205, 244)
SEA = (99, 179, 237)
DEEP = (43, 108, 176)
INK = (26, 54, 93)


def _rounded_bg(size: int, draw: ImageDraw.ImageDraw) -> None:
    """An app-tile-style rounded square, deep-blue gradient-ish via two bands."""
    radius = int(size * 0.22)
    bb = [0, 0, size - 1, size - 1]
    draw.rounded_rectangle(bb, radius=radius, fill=DEEP)


def _shark(size: int, draw: ImageDraw.ImageDraw) -> None:
    """A compact stylized shark facing right, drawn in ocean blue on the tile.

    Coordinates are scaled from a 0..1 design so the icon stays crisp at any
    size. The silhouette is a smooth body + tail + dorsal, built from polygons
    and an ellipse for the head. Eye and belly highlight in mist/ink.
    """
    s = size
    cx, cy = s * 0.5, s * 0.54
    scale = s / 128.0  # design authored at 128

    def p(x: float, y: float) -> tuple[float, float]:
        return cx + (x - 0.5) * s * 0.74, cy + (y - 0.5) * s * 0.74

    # Body: a horizontal leaf/teardrop from tail (left) to nose (right).
    body = [
        p(0.18, 0.50),  # tail base (left, center)
        p(0.40, 0.30),  # upper back
        p(0.70, 0.28),  # toward head
        p(0.86, 0.42),  # nose tip
        p(0.82, 0.52),  # under nose
        p(0.66, 0.58),  # belly
        p(0.40, 0.62),  # lower belly
        p(0.18, 0.50),  # back to tail base
    ]
    draw.polygon([b for b in body], fill=ICE, outline=SKY)

    # Tail (caudal fin): two triangles flaring left from the tail base.
    tail = [
        p(0.20, 0.50),
        p(0.06, 0.34),
        p(0.12, 0.50),
        p(0.06, 0.66),
        p(0.20, 0.50),
    ]
    draw.polygon([t for t in tail], fill=SEA, outline=SKY)

    # Dorsal fin: a small triangle above the back.
    dorsal = [
        p(0.42, 0.30),
        p(0.52, 0.16),
        p(0.58, 0.30),
        p(0.42, 0.30),
    ]
    draw.polygon([d for d in dorsal], fill=SEA, outline=SKY)

    # Belly highlight (mist) — a soft band under the body.
    belly = [
        p(0.34, 0.55),
        p(0.66, 0.55),
        p(0.60, 0.60),
        p(0.40, 0.60),
        p(0.34, 0.55),
    ]
    draw.polygon([b for b in belly], fill=MIST)

    # Eye: a small dark dot near the nose.
    eye_r = max(1, int(3.2 * scale))
    draw.ellipse(
        [p(0.74, 0.40)[0] - eye_r, p(0.74, 0.40)[1] - eye_r,
         p(0.74, 0.40)[0] + eye_r, p(0.74, 0.40)[1] + eye_r],
        fill=INK,
    )

    # Smile: a short arc under the nose.
    smile = [
        p(0.70, 0.48),
        p(0.80, 0.50),
        p(0.70, 0.52),
    ]
    draw.line([smile[0], smile[1], smile[2]], fill=INK, width=max(1, int(2.4 * scale)))


def _make(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    _rounded_bg(size, draw)
    _shark(size, draw)
    return img


# Tray presence icons. The shark tile is illegible at 16px tray size, so the
# tray shows a plain status disc whose color matches the in-app presence dots
# (see client/src/style.css `.dot-online/away/offline`). Online/away share the
# same "present" tile with a colored dot; offline is a muted gray disc. These
# are swapped at runtime by the `set_tray_presence` Rust command.
TRAY_PRESENCE = {
    "tray-online.png": ((72, 187, 120), (40, 130, 85)),    # #48bb78 green
    "tray-away.png": ((236, 201, 75), (200, 160, 40)),    # #ecc94b amber
    "tray-offline.png": ((160, 174, 192), (110, 120, 140)),  # #a0aec0 gray
}


def _tray_disc(size: int, color: tuple, ring: tuple) -> Image.Image:
    """A filled status disc on a transparent tray canvas (reads at 16px)."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    r = int(size * 0.40)  # disc radius
    cx = cy = size // 2
    # subtle darker ring (outline) so the disc reads on light + dark taskbars
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=color, outline=ring, width=max(1, int(size * 0.06)))
    return img


def main() -> int:
    out = Path(__file__).resolve().parents[2] / "client" / "src-tauri" / "icons"
    out.mkdir(parents=True, exist_ok=True)

    big = _make(128)
    big.save(out / "128x128.png")

    small = _make(32)
    small.save(out / "32x32.png")

    # Windows .ico: save one large image and let Pillow embed the requested
    # sub-sizes automatically (it resizes on save — no `append` needed).
    _make(256).save(
        out / "icon.ico",
        format="ICO",
        sizes=[(n, n) for n in [16, 32, 48, 64, 128, 256]],
    )

    # Tray presence icons at 32px (Windows scales down to the tray's 16/20/24).
    for name, (color, ring) in TRAY_PRESENCE.items():
        _tray_disc(32, color, ring).save(out / name)

    for f in sorted(out.iterdir()):
        print("wrote", f)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
