#!/usr/bin/env python3
"""Generate the bundled default image-maps (keyboard US/DE, Xbox, PlayStation).

Usage: scripts/gen-imagemaps.py [--out DIR] [--overlays DIR]

Each device's geometry is defined exactly once, in pixel space, and drives both
the rendered `image.png` (via an SVG built here and rasterized with cairosvg)
and the `imagemap.json` (format 3) whose area coordinates are that same
geometry normalized to 0..1 of the PNG size.

  --out       image-map root to write `<id>/imagemap.json` + `<id>/image.png`
              into (default: ../src-tauri/resources/imagemaps)
  --overlays  optional directory for review renders: every area drawn
              semi-transparent over the image with its input name. Never part
              of the repo.

Only stdlib plus cairosvg is used.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import cairosvg

# --------------------------------------------------------------------------
# Palette — mirrors src/styles/tokens.css (the only colour source in the app).
# --------------------------------------------------------------------------

BG_BASE = "#0a1d29"
BG_SURFACE = "#0f2c3e"
BG_SURFACE_2 = "#143a52"
BG_SURFACE_3 = "#194967"
BORDER = "#1e4a66"
ACCENT = "#54adf7"
TEXT = "#ffffff"
TEXT_2 = "#add3eb"
TEXT_3 = "#5d7f96"  # opaque stand-in for --text-3 (rgba over the panel colour)
OK = "#4fd28a"
WARN = "#f2b34c"
ERR = "#ff5c6c"

FONT = "DejaVu Sans"
# Rough advance width of DejaVu Sans at font-size 1, used to fit legends.
CHAR_W = 0.60

# The `arrow` symbol path from src/imagemap.ts, in its 100x100 box, points right.
ARROW_PATH = "M5 40 H60 V20 L95 50 L60 80 V60 H5 Z"

BUNDLED = [
    ("4b7a2c1e-0001-4000-8000-000000000001", "Keyboard US", "keyboard", "Keyboard/Mouse"),
    ("4b7a2c1e-0001-4000-8000-000000000002", "Keyboard DE", "keyboard", "Keyboard/Mouse"),
    ("4b7a2c1e-0001-4000-8000-000000000003", "Xbox controller", "gamepad", "Gamepad"),
    ("4b7a2c1e-0001-4000-8000-000000000004", "PlayStation controller", "gamepad", "Gamepad"),
]


# --------------------------------------------------------------------------
# Tiny SVG builder
# --------------------------------------------------------------------------


def esc(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def n(v: float) -> str:
    return f"{v:.2f}".rstrip("0").rstrip(".")


class Doc:
    """An SVG document collecting raw element strings."""

    def __init__(self, width: float, height: float, background: str) -> None:
        self.width = width
        self.height = height
        self.parts: list[str] = [
            f'<rect x="0" y="0" width="{n(width)}" height="{n(height)}" fill="{background}"/>'
        ]

    def add(self, part: str) -> None:
        self.parts.append(part)

    def svg(self) -> str:
        head = (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{n(self.width)}" '
            f'height="{n(self.height)}" viewBox="0 0 {n(self.width)} {n(self.height)}">'
        )
        return head + "".join(self.parts) + "</svg>"

    def write_png(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        cairosvg.svg2png(
            bytestring=self.svg().encode("utf-8"),
            write_to=str(path),
            output_width=int(round(self.width)),
            output_height=int(round(self.height)),
        )


def style(fill: str, stroke: str | None, sw: float) -> str:
    out = f' fill="{fill}"'
    if stroke and sw > 0:
        out += f' stroke="{stroke}" stroke-width="{n(sw)}"'
    return out


def svg_rrect(
    x: float, y: float, w: float, h: float, r: float,
    fill: str, stroke: str | None = None, sw: float = 0, rotation: float = 0,
) -> str:
    tr = ""
    if rotation:
        tr = f' transform="rotate({n(rotation)} {n(x + w / 2)} {n(y + h / 2)})"'
    return (
        f'<rect x="{n(x)}" y="{n(y)}" width="{n(w)}" height="{n(h)}" '
        f'rx="{n(r)}" ry="{n(r)}"{style(fill, stroke, sw)}{tr}/>'
    )


def svg_ellipse(
    cx: float, cy: float, rx: float, ry: float,
    fill: str, stroke: str | None = None, sw: float = 0,
) -> str:
    return (
        f'<ellipse cx="{n(cx)}" cy="{n(cy)}" rx="{n(rx)}" ry="{n(ry)}"'
        f"{style(fill, stroke, sw)}/>"
    )


def svg_poly(
    points: list[tuple[float, float]],
    fill: str, stroke: str | None = None, sw: float = 0,
) -> str:
    pts = " ".join(f"{n(px)},{n(py)}" for px, py in points)
    return f'<polygon points="{pts}"{style(fill, stroke, sw)}/>'


def svg_arrow(
    cx: float, cy: float, w: float, h: float, rotation: float, fill: str
) -> str:
    """The `arrow` symbol path stretched into a w x h box centred on cx/cy."""
    tr = f"rotate({n(rotation)} {n(cx)} {n(cy)}) translate({n(cx - w / 2)} {n(cy - h / 2)}) scale({n(w / 100)} {n(h / 100)})"
    return f'<path d="{ARROW_PATH}" fill="{fill}" transform="{tr}"/>'


def svg_text(
    cx: float, cy: float, text: str, size: float, fill: str,
    weight: str = "500", anchor: str = "middle",
) -> str:
    if not text:
        return ""
    return (
        f'<text x="{n(cx)}" y="{n(cy + size * 0.35)}" font-family="{FONT}" '
        f'font-size="{n(size)}" font-weight="{weight}" fill="{fill}" '
        f'text-anchor="{anchor}">{esc(text)}</text>'
    )


def fit_size(label: str, box_w: float, base: float, floor: float = 15.0) -> float:
    if not label:
        return base
    avail = box_w - 12
    return max(floor, min(base, avail / (CHAR_W * len(label))))


# --------------------------------------------------------------------------
# Areas — geometry in pixels, normalized on write
# --------------------------------------------------------------------------


class Areas:
    """Collects areas in pixel space; `to_json` normalizes them to 0..1."""

    def __init__(self, width: float, height: float) -> None:
        self.width = width
        self.height = height
        self.items: list[tuple[str, dict]] = []

    def rect(self, input_key: str, x: float, y: float, w: float, h: float) -> None:
        self.items.append((input_key, {"kind": "rect", "x": x, "y": y, "w": w, "h": h, "rotation": 0.0}))

    def ellipse(self, input_key: str, cx: float, cy: float, rx: float, ry: float) -> None:
        self.items.append((input_key, {"kind": "ellipse", "cx": cx, "cy": cy, "rx": rx, "ry": ry, "rotation": 0.0}))

    def polygon(self, input_key: str, points: list[tuple[float, float]]) -> None:
        self.items.append((input_key, {"kind": "polygon", "points": [list(p) for p in points]}))

    def arrow(self, input_key: str, cx: float, cy: float, w: float, h: float, rotation: float) -> None:
        self.items.append(
            (input_key, {"kind": "symbol", "symbol": "arrow", "x": cx, "y": cy, "w": w, "h": h, "rotation": rotation})
        )

    def to_json(self) -> list[dict]:
        out = []
        for i, (input_key, shape) in enumerate(self.items, start=1):
            out.append({"id": f"a{i}", "input": input_key, "shape": self._norm(shape)})
        return out

    def _norm(self, shape: dict) -> dict:
        w, h = self.width, self.height
        r = lambda v: round(v, 6)  # noqa: E731
        k = shape["kind"]
        if k == "rect":
            return {"kind": "rect", "x": r(shape["x"] / w), "y": r(shape["y"] / h),
                    "w": r(shape["w"] / w), "h": r(shape["h"] / h), "rotation": 0.0}
        if k == "ellipse":
            return {"kind": "ellipse", "cx": r(shape["cx"] / w), "cy": r(shape["cy"] / h),
                    "rx": r(shape["rx"] / w), "ry": r(shape["ry"] / h), "rotation": 0.0}
        if k == "polygon":
            return {"kind": "polygon", "points": [[r(px / w), r(py / h)] for px, py in shape["points"]]}
        if k == "symbol":
            return {"kind": "symbol", "symbol": shape["symbol"], "x": r(shape["x"] / w), "y": r(shape["y"] / h),
                    "w": r(shape["w"] / w), "h": r(shape["h"] / h), "rotation": float(shape["rotation"])}
        raise ValueError(k)


# --------------------------------------------------------------------------
# Keyboards
# --------------------------------------------------------------------------

U = 104.0        # key pitch in px (1u)
INSET = 5.0      # half the gap between two caps
MARGIN = 44.0
COLS = 26.0      # full-size width in units, mouse block included
ROWS = 6.5       # F-row + 0.5u gap + 5 rows

KB_W = MARGIN * 2 + COLS * U
KB_H = MARGIN * 2 + ROWS * U

R_F = 0.0        # F-row
R1, R2, R3, R4, R5 = 1.5, 2.5, 3.5, 4.5, 5.5

NAV_X = 15.25
NUM_X = 18.5
MOUSE_X = 23.0   # the mouse sits right of the numpad: SC binds it as kb1_


def kx(col: float) -> float:
    return MARGIN + col * U


def ky(row: float) -> float:
    return MARGIN + row * U


def cap_box(col: float, row: float, w: float = 1.0, h: float = 1.0) -> tuple[float, float, float, float]:
    """Pixel rect of a keycap, gaps already taken off."""
    return (kx(col) + INSET, ky(row) + INSET, w * U - 2 * INSET, h * U - 2 * INSET)


def row_of(x0: float, row: float, pairs: list[tuple[str, str | None]], w: float = 1.0) -> list:
    """`pairs` laid out left to right, each `w` units wide."""
    return [(x0 + i * w, row, w, 1.0, label, name) for i, (label, name) in enumerate(pairs)]


def kb_common() -> list:
    """F-row, nav cluster, arrows and numpad — identical on US and DE."""
    keys: list = []
    keys.append((0, R_F, 1.0, 1.0, "Esc", "escape"))
    for i in range(4):
        keys.append((2 + i, R_F, 1.0, 1.0, f"F{i + 1}", f"f{i + 1}"))
    for i in range(4):
        keys.append((6.5 + i, R_F, 1.0, 1.0, f"F{i + 5}", f"f{i + 5}"))
    for i in range(4):
        keys.append((11 + i, R_F, 1.0, 1.0, f"F{i + 9}", f"f{i + 9}"))

    keys += row_of(NAV_X, R_F, [("PrtSc", "print"), ("Scroll", "scrolllock"), ("Pause", "pause")])
    keys += row_of(NAV_X, R1, [("Insert", "insert"), ("Home", "home"), ("PgUp", "pgup")])
    keys += row_of(NAV_X, R2, [("Delete", "delete"), ("End", "end"), ("PgDn", "pgdn")])
    keys.append((NAV_X + 1, R4, 1.0, 1.0, "↑", "up"))
    keys += row_of(NAV_X, R5, [("←", "left"), ("↓", "down"), ("→", "right")])

    keys += row_of(NUM_X, R1, [("Num", "numlock"), ("/", "np_divide"), ("*", "np_multiply"), ("−", "np_subtract")])
    keys += row_of(NUM_X, R2, [("7", "np_7"), ("8", "np_8"), ("9", "np_9")])
    keys += row_of(NUM_X, R3, [("4", "np_4"), ("5", "np_5"), ("6", "np_6")])
    keys += row_of(NUM_X, R4, [("1", "np_1"), ("2", "np_2"), ("3", "np_3")])
    keys.append((NUM_X + 3, R2, 1.0, 2.0, "+", "np_add"))
    keys.append((NUM_X + 3, R4, 1.0, 2.0, "Enter", "np_enter"))
    keys.append((NUM_X, R5, 2.0, 1.0, "0", "np_0"))
    keys.append((NUM_X + 2, R5, 1.0, 1.0, ".", "np_period"))
    return keys


BOTTOM_ROW = [
    (0.0, 1.25, "Ctrl", "lctrl"),
    (1.25, 1.25, "Win", None),
    (2.5, 1.25, "Alt", "lalt"),
    (3.75, 6.25, "", "space"),
    (10.0, 1.25, "Alt", "ralt"),
    (11.25, 1.25, "Win", None),
    (12.5, 1.25, "Menu", None),
    (13.75, 1.25, "Ctrl", "rctrl"),
]


def kb_us() -> tuple[list, list]:
    """(rect keys, extra shapes) for the ANSI/US main block plus the shared blocks."""
    keys = kb_common()
    digits = [("1", "1"), ("2", "2"), ("3", "3"), ("4", "4"), ("5", "5"),
              ("6", "6"), ("7", "7"), ("8", "8"), ("9", "9"), ("0", "0")]

    keys.append((0, R1, 1.0, 1.0, "`", None))            # Backquote: no SC name
    keys += row_of(1, R1, digits)
    keys.append((11, R1, 1.0, 1.0, "-", "minus"))
    keys.append((12, R1, 1.0, 1.0, "=", "equals"))
    keys.append((13, R1, 2.0, 1.0, "Backspace", "backspace"))

    keys.append((0, R2, 1.5, 1.0, "Tab", "tab"))
    keys += row_of(1.5, R2, [(c.upper(), c) for c in "qwertyuiop"])
    keys.append((11.5, R2, 1.0, 1.0, "[", "lbracket"))
    keys.append((12.5, R2, 1.0, 1.0, "]", "rbracket"))
    keys.append((13.5, R2, 1.5, 1.0, "\\", "backslash"))

    keys.append((0, R3, 1.75, 1.0, "Caps", "capslock"))
    keys += row_of(1.75, R3, [(c.upper(), c) for c in "asdfghjkl"])
    keys.append((10.75, R3, 1.0, 1.0, ";", "semicolon"))
    keys.append((11.75, R3, 1.0, 1.0, "'", "apostrophe"))
    keys.append((12.75, R3, 2.25, 1.0, "Enter", "enter"))

    keys.append((0, R4, 2.25, 1.0, "Shift", "lshift"))
    keys += row_of(2.25, R4, [(c.upper(), c) for c in "zxcvbnm"])
    keys.append((9.25, R4, 1.0, 1.0, ",", "comma"))
    keys.append((10.25, R4, 1.0, 1.0, ".", "period"))
    keys.append((11.25, R4, 1.0, 1.0, "/", "slash"))
    keys.append((12.25, R4, 2.75, 1.0, "Shift", "rshift"))

    for col, w, label, name in BOTTOM_ROW:
        keys.append((col, R5, w, 1.0, label, name))
    return keys, []


def iso_enter_points() -> list[tuple[float, float]]:
    """The reverse-L ISO Enter, gaps taken off (concave corner included)."""
    left_hi, left_lo, right = kx(13.5) + INSET, kx(13.75) + INSET, kx(15) - INSET
    top, mid, bot = ky(R2) + INSET, ky(R3), ky(R3 + 1) - INSET
    return [
        (left_hi, top), (right, top), (right, bot),
        (left_lo, bot), (left_lo, mid - INSET), (left_hi, mid - INSET),
    ]


def kb_de() -> tuple[list, list]:
    """(rect keys, extra shapes) for the ISO/DE main block plus the shared blocks.

    Areas are keyed by the SC scancode name of the PHYSICAL key, so the DE cap
    "Y" is `key:z`, "ß" is `key:minus` and so on.
    """
    keys = kb_common()
    digits = [("1", "1"), ("2", "2"), ("3", "3"), ("4", "4"), ("5", "5"),
              ("6", "6"), ("7", "7"), ("8", "8"), ("9", "9"), ("0", "0")]

    keys.append((0, R1, 1.0, 1.0, "^", None))            # Backquote position: no SC name
    keys += row_of(1, R1, digits)
    keys.append((11, R1, 1.0, 1.0, "ß", "minus"))
    keys.append((12, R1, 1.0, 1.0, "´", "equals"))
    keys.append((13, R1, 2.0, 1.0, "Backspace", "backspace"))

    keys.append((0, R2, 1.5, 1.0, "Tab", "tab"))
    # Cap "Z" sits on the US "y" position and vice versa.
    keys += row_of(1.5, R2, [("Q", "q"), ("W", "w"), ("E", "e"), ("R", "r"), ("T", "t"),
                             ("Z", "y"), ("U", "u"), ("I", "i"), ("O", "o"), ("P", "p")])
    keys.append((11.5, R2, 1.0, 1.0, "Ü", "lbracket"))
    keys.append((12.5, R2, 1.0, 1.0, "+", "rbracket"))

    keys.append((0, R3, 1.75, 1.0, "Caps", "capslock"))
    keys += row_of(1.75, R3, [(c.upper(), c) for c in "asdfghjkl"])
    keys.append((10.75, R3, 1.0, 1.0, "Ö", "semicolon"))
    keys.append((11.75, R3, 1.0, 1.0, "Ä", "apostrophe"))
    keys.append((12.75, R3, 1.0, 1.0, "#", "backslash"))

    keys.append((0, R4, 1.25, 1.0, "Shift", "lshift"))
    keys.append((1.25, R4, 1.0, 1.0, "<", "oem_102"))
    keys += row_of(2.25, R4, [("Y", "z"), ("X", "x"), ("C", "c"), ("V", "v"),
                              ("B", "b"), ("N", "n"), ("M", "m")])
    keys.append((9.25, R4, 1.0, 1.0, ",", "comma"))
    keys.append((10.25, R4, 1.0, 1.0, ".", "period"))
    keys.append((11.25, R4, 1.0, 1.0, "-", "slash"))
    keys.append((12.25, R4, 2.75, 1.0, "Shift", "rshift"))

    for col, w, label, name in BOTTOM_ROW:
        keys.append((col, R5, w, 1.0, label if label != "Alt" or col < 10 else "AltGr", name))

    extras = [("iso_enter", iso_enter_points(), "Enter", "enter")]
    return keys, extras


def mouse(doc: Doc, areas: Areas) -> None:
    """A mouse in the block right of the numpad: mouse1 / mouse2 (left /
    right), the wheel (mouse3 = click, arrows = mwheel_up / mwheel_down)
    and the two thumb buttons (mouse5 front, mouse4 rear)."""
    bw, bh = 2.4 * U, 4.6 * U
    bx = kx(MOUSE_X) + (3.0 * U - bw) / 2 + 12   # nudged right: the thumb buttons stick out left
    by = ky(R_F) + (ROWS * U - bh) / 2
    cx = bx + bw / 2
    doc.add(svg_rrect(bx, by, bw, bh, bw / 2 - 8, BG_SURFACE_2, BORDER, 2))

    # Main buttons: the top 42 % of the body, a channel for the wheel between.
    top_h = 0.42 * bh
    gap = 34.0
    for name, x0, x1, label in (("mouse1", bx + 10, cx - gap, "1"), ("mouse2", cx + gap, bx + bw - 10, "2")):
        doc.add(svg_rrect(x0, by + 10, x1 - x0, top_h - 10, 40, BG_SURFACE, BORDER, 2))
        doc.add(svg_text((x0 + x1) / 2, by + top_h * 0.58, label, 34, TEXT_2))
        areas.rect(f"key:{name}", x0, by + 10, x1 - x0, top_h - 10)

    # Wheel with an arrow above and below.
    ww, wh = 40.0, 110.0
    wy = by + 64
    doc.add(svg_rrect(cx - ww / 2, wy, ww, wh, 12, BG_SURFACE, BORDER, 2))
    areas.rect("key:mouse3", cx - ww / 2, wy, ww, wh)
    arrow = 34.0
    for name, cy, rot in (("mwheel_up", wy - 24, -90), ("mwheel_down", wy + wh + 24, 90)):
        doc.add(svg_arrow(cx, cy, arrow, arrow, rot, TEXT_2))
        areas.arrow(f"key:{name}", cx, cy, arrow, arrow, rot)

    # Thumb buttons on the left flank.
    tw, th = 44.0, 62.0
    tx = bx - tw + 14
    for name, ty, label in (("mouse5", by + top_h + 30, "5"), ("mouse4", by + top_h + 30 + th + 14, "4")):
        doc.add(svg_rrect(tx, ty, tw, th, 12, BG_SURFACE, BORDER, 2))
        doc.add(svg_text(tx + tw / 2 - 4, ty + th / 2, label, 24, TEXT_2))
        areas.rect(f"key:{name}", tx, ty, tw, th)


def build_keyboard(keys: list, extras: list) -> tuple[Doc, Areas]:
    doc = Doc(KB_W, KB_H, BG_BASE)
    areas = Areas(KB_W, KB_H)

    # Block backdrops, so the four clusters read as blocks.
    for x0, w in ((0.0, 15.0), (NAV_X, 3.0), (NUM_X, 4.0), (MOUSE_X, 3.0)):
        doc.add(svg_rrect(kx(x0) - 10, ky(R_F) - 10, w * U + 20, ROWS * U + 20, 14, BG_SURFACE))

    for col, row, w, h, label, name in keys:
        x, y, bw, bh = cap_box(col, row, w, h)
        bound = name is not None
        doc.add(svg_rrect(x, y, bw, bh, 9, BG_SURFACE_2 if bound else BG_SURFACE,
                          BORDER if bound else "#12324a", 2))
        doc.add(svg_text(x + bw / 2, y + bh / 2, label,
                         fit_size(label, bw, 34), TEXT_2 if bound else TEXT_3))
        if bound:
            areas.rect(f"key:{name}", x, y, bw, bh)

    for kind, points, label, name in extras:
        assert kind == "iso_enter"
        doc.add(svg_poly(points, BG_SURFACE_2, BORDER, 2))
        doc.add(svg_text(kx(14.375), ky(R3) + 14, label, 26, TEXT_2))
        areas.polygon(f"key:{name}", points)

    mouse(doc, areas)
    return doc, areas


# --------------------------------------------------------------------------
# Gamepads
# --------------------------------------------------------------------------

PAD_W, PAD_H = 1200.0, 850.0

STICK_WELL = 88.0    # outer ring radius
STICK_CAP = 62.0     # cap radius
STICK_TICK = 112.0   # distance of the small direction arrows from the centre

# Shoulder / trigger boxes (mirrored), shared by both pads.
TRIG_L = (210.0, 16.0, 175.0, 82.0)
TRIG_R = (815.0, 16.0, 175.0, 82.0)
SHLD_L = (195.0, 104.0, 205.0, 66.0)
SHLD_R = (800.0, 104.0, 205.0, 66.0)


def body_shapes(parts: list, doc: Doc) -> None:
    """Draw a union silhouette: every part once thick in the border colour,
    then again filled, so only the outer edge keeps a border."""
    for fill, stroke, sw in ((BORDER, BORDER, 10), (BG_SURFACE, None, 0)):
        for p in parts:
            if p[0] == "rrect":
                _, x, y, w, h, r, rot = p
                doc.add(svg_rrect(x, y, w, h, r, fill, stroke, sw, rot))
            else:
                _, cx, cy, rx, ry = p
                doc.add(svg_ellipse(cx, cy, rx, ry, fill, stroke, sw))


def stick(doc: Doc, areas: Areas, cx: float, cy: float, side: str) -> None:
    """One analogue stick: well, cap, four direction ticks, and every area
    SC knows for it (click, both axes, four derived directions)."""
    doc.add(svg_ellipse(cx, cy, STICK_WELL, STICK_WELL, BG_SURFACE_2, BORDER, 3))
    doc.add(svg_ellipse(cx, cy, STICK_CAP, STICK_CAP, BG_SURFACE_3, BORDER, 3))
    doc.add(svg_ellipse(cx, cy, STICK_CAP * 0.55, STICK_CAP * 0.55, BG_SURFACE_2))

    for name, rot, dx, dy in (
        ("up", -90, 0, -1), ("down", 90, 0, 1), ("left", 180, -1, 0), ("right", 0, 1, 0)
    ):
        ax, ay = cx + dx * STICK_TICK, cy + dy * STICK_TICK
        doc.add(svg_arrow(ax, ay, 44, 34, rot, TEXT_3))
        areas.arrow(f"pad:thumb{side}_{name}", ax, ay, 48, 40, rot)

    areas.ellipse(f"pad:thumb{side}", cx, cy, STICK_CAP, STICK_CAP)
    areas.arrow(f"pad:thumb{side}x", cx, cy, 168, 52, 0)
    areas.arrow(f"pad:thumb{side}y", cx, cy, 168, 52, 90)


def dpad(doc: Doc, areas: Areas, cx: float, cy: float) -> None:
    arm, half = 108.0, 40.0
    doc.add(svg_poly([
        (cx - half, cy - arm), (cx + half, cy - arm), (cx + half, cy - half),
        (cx + arm, cy - half), (cx + arm, cy + half), (cx + half, cy + half),
        (cx + half, cy + arm), (cx - half, cy + arm), (cx - half, cy + half),
        (cx - arm, cy + half), (cx - arm, cy - half), (cx - half, cy - half),
    ], BG_SURFACE_3, BORDER, 3))
    for name, rot, dx, dy in (
        ("up", -90, 0, -1), ("down", 90, 0, 1), ("left", 180, -1, 0), ("right", 0, 1, 0)
    ):
        doc.add(svg_arrow(cx + dx * (arm - 32), cy + dy * (arm - 32), 34, 26, rot, TEXT_2))
    areas.rect("pad:dpad_up", cx - half, cy - arm, half * 2, arm - half)
    areas.rect("pad:dpad_down", cx - half, cy + half, half * 2, arm - half)
    areas.rect("pad:dpad_left", cx - arm, cy - half, arm - half, half * 2)
    areas.rect("pad:dpad_right", cx + half, cy - half, arm - half, half * 2)


def shoulder(doc: Doc, areas: Areas, name: str, label: str,
             x: float, y: float, w: float, h: float) -> None:
    doc.add(svg_rrect(x, y, w, h, h / 2.4, BG_SURFACE_3, BORDER, 3))
    doc.add(svg_text(x + w / 2, y + h / 2, label, 30, TEXT_2))
    areas.rect(f"pad:{name}", x, y, w, h)


def trigger(doc: Doc, areas: Areas, side: str, label: str,
            x: float, y: float, w: float, h: float) -> None:
    """The trigger axis over the whole pull, plus the derived button on top."""
    doc.add(svg_rrect(x, y, w, h, h / 2.6, BG_SURFACE_3, BORDER, 3))
    doc.add(svg_text(x + w / 2, y + h / 2, label, 30, TEXT_2))
    areas.rect(f"pad:trigger{side}", x, y, w, h)
    pad = 14.0
    areas.rect(f"pad:trigger{side}_btn", x + pad, y + pad, w - 2 * pad, h - 2 * pad)


def small_button(doc: Doc, areas: Areas, name: str, label: str,
                 cx: float, cy: float, w: float, h: float) -> None:
    doc.add(svg_rrect(cx - w / 2, cy - h / 2, w, h, min(w, h) / 2.2, BG_SURFACE_3, BORDER, 3))
    if label:
        doc.add(svg_text(cx, cy + h / 2 + 24, label, 22, TEXT_3))
    areas.ellipse(f"pad:{name}", cx, cy, w / 2 + 6, h / 2 + 6)


def build_xbox() -> tuple[Doc, Areas]:
    doc = Doc(PAD_W, PAD_H, BG_BASE)
    areas = Areas(PAD_W, PAD_H)

    body_shapes([
        ("rrect", 340, 150, 520, 530, 130, 0),
        ("ellipse", 310, 315, 195, 175),
        ("ellipse", 890, 315, 195, 175),
        ("rrect", 239, 430, 172, 370, 86, 16),
        ("rrect", 789, 430, 172, 370, 86, -16),
        ("rrect", *TRIG_L, 32, 0),
        ("rrect", *TRIG_R, 32, 0),
        ("rrect", *SHLD_L, 30, 0),
        ("rrect", *SHLD_R, 30, 0),
    ], doc)

    trigger(doc, areas, "l", "LT", *TRIG_L)
    trigger(doc, areas, "r", "RT", *TRIG_R)
    shoulder(doc, areas, "shoulderl", "LB", *SHLD_L)
    shoulder(doc, areas, "shoulderr", "RB", *SHLD_R)

    stick(doc, areas, 310, 315, "l")
    stick(doc, areas, 745, 515, "r")
    dpad(doc, areas, 455, 515)

    # Face diamond, Xbox colour hints on the legends.
    fx, fy, off, r = 890.0, 305.0, 74.0, 40.0
    for name, label, colour, dx, dy in (
        ("a", "A", OK, 0, 1), ("b", "B", ERR, 1, 0), ("x", "X", ACCENT, -1, 0), ("y", "Y", WARN, 0, -1)
    ):
        bx, by = fx + dx * off, fy + dy * off
        doc.add(svg_ellipse(bx, by, r, r, BG_SURFACE_3, BORDER, 3))
        doc.add(svg_text(bx, by, label, 34, colour, weight="700"))
        areas.ellipse(f"pad:{name}", bx, by, r, r)

    small_button(doc, areas, "back", "View", 545, 320, 44, 34)
    small_button(doc, areas, "start", "Menu", 655, 320, 44, 34)

    # Guide: drawn, no area (SC has no token for it).
    doc.add(svg_ellipse(600, 210, 33, 33, BG_SURFACE_3, BORDER, 3))
    doc.add(svg_ellipse(600, 210, 15, 15, BG_SURFACE_2, TEXT_3, 3))
    doc.add(svg_text(600, 264, "Guide", 22, TEXT_3))

    doc.add(svg_text(PAD_W / 2, PAD_H - 26, "Xbox", 26, TEXT_3))
    return doc, areas


def ps_glyph(doc: Doc, cx: float, cy: float, kind: str, colour: str) -> None:
    """The four PlayStation face marks as plain shapes (no font dependency)."""
    s, sw = 20.0, 5.0
    if kind == "triangle":
        h = s * 1.05
        doc.add(svg_poly([(cx, cy - h), (cx + h * 0.92, cy + h * 0.72), (cx - h * 0.92, cy + h * 0.72)],
                         "none", colour, sw))
    elif kind == "circle":
        doc.add(svg_ellipse(cx, cy, s, s, "none", colour, sw))
    elif kind == "cross":
        d = s * 0.85
        doc.add(f'<path d="M{n(cx - d)} {n(cy - d)} L{n(cx + d)} {n(cy + d)} '
                f'M{n(cx + d)} {n(cy - d)} L{n(cx - d)} {n(cy + d)}" '
                f'stroke="{colour}" stroke-width="{n(sw)}" stroke-linecap="round" fill="none"/>')
    else:  # square
        d = s * 0.82
        doc.add(svg_rrect(cx - d, cy - d, d * 2, d * 2, 3, "none", colour, sw))


def build_playstation() -> tuple[Doc, Areas]:
    doc = Doc(PAD_W, PAD_H, BG_BASE)
    areas = Areas(PAD_W, PAD_H)

    body_shapes([
        ("rrect", 340, 150, 520, 460, 120, 0),
        ("ellipse", 300, 300, 190, 165),
        ("ellipse", 900, 300, 190, 165),
        ("rrect", 350, 430, 500, 270, 130, 0),
        ("rrect", 246, 460, 168, 320, 84, 12),
        ("rrect", 786, 460, 168, 320, 84, -12),
        ("rrect", *TRIG_L, 32, 0),
        ("rrect", *TRIG_R, 32, 0),
        ("rrect", *SHLD_L, 30, 0),
        ("rrect", *SHLD_R, 30, 0),
    ], doc)

    trigger(doc, areas, "l", "L2", *TRIG_L)
    trigger(doc, areas, "r", "R2", *TRIG_R)
    shoulder(doc, areas, "shoulderl", "L1", *SHLD_L)
    shoulder(doc, areas, "shoulderr", "R1", *SHLD_R)

    dpad(doc, areas, 300, 305)

    # Symmetric sticks, both bottom-centre.
    stick(doc, areas, 455, 530, "l")
    stick(doc, areas, 745, 530, "r")

    # Touchpad: drawn, no area (no SC token).
    doc.add(svg_rrect(480, 180, 240, 150, 20, BG_SURFACE_2, BORDER, 3))
    doc.add(svg_text(600, 255, "Touchpad", 22, TEXT_3))

    fx, fy, off, r = 900.0, 305.0, 74.0, 40.0
    for name, glyph, colour, dx, dy in (
        ("a", "cross", TEXT_2, 0, 1), ("b", "circle", TEXT_2, 1, 0),
        ("x", "square", TEXT_2, -1, 0), ("y", "triangle", TEXT_2, 0, -1)
    ):
        bx, by = fx + dx * off, fy + dy * off
        doc.add(svg_ellipse(bx, by, r, r, BG_SURFACE_3, BORDER, 3))
        ps_glyph(doc, bx, by, glyph, colour)
        areas.ellipse(f"pad:{name}", bx, by, r, r)

    small_button(doc, areas, "back", "Share", 432, 205, 30, 52)
    small_button(doc, areas, "start", "Options", 768, 205, 30, 52)

    # PS button: drawn, no area.
    doc.add(svg_ellipse(600, 395, 30, 30, BG_SURFACE_3, BORDER, 3))
    doc.add(svg_text(600, 395, "PS", 20, TEXT_3, weight="700"))

    doc.add(svg_text(PAD_W / 2, PAD_H - 26, "PlayStation", 26, TEXT_3))
    return doc, areas


# --------------------------------------------------------------------------
# Output
# --------------------------------------------------------------------------


def write_map(out_root: Path, meta: tuple[str, str, str, str], doc: Doc, areas: Areas) -> dict:
    map_id, name, hardware_id, hardware_name = meta
    folder = out_root / map_id
    folder.mkdir(parents=True, exist_ok=True)
    doc.write_png(folder / "image.png")
    data = {
        "format": 3,
        "id": map_id,
        "name": name,
        "hardware_id": hardware_id,
        "hardware_name": hardware_name,
        "image": {"file": "image.png", "label": "Top"},
        "areas": areas.to_json(),
    }
    (folder / "imagemap.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    return data


def check(data: dict) -> list[str]:
    """Structural checks matching imagemap.rs plus the 0..1 bound."""
    problems = []
    for area in data["areas"]:
        s = area["shape"]
        vals: list[tuple[str, float]] = []
        if s["kind"] == "rect":
            vals = [("x", s["x"]), ("y", s["y"]), ("x+w", s["x"] + s["w"]), ("y+h", s["y"] + s["h"])]
        elif s["kind"] == "ellipse":
            vals = [("cx-rx", s["cx"] - s["rx"]), ("cx+rx", s["cx"] + s["rx"]),
                    ("cy-ry", s["cy"] - s["ry"]), ("cy+ry", s["cy"] + s["ry"])]
        elif s["kind"] == "polygon":
            for i, (px, py) in enumerate(s["points"]):
                vals += [(f"p{i}.x", px), (f"p{i}.y", py)]
        elif s["kind"] == "symbol":
            half_w, half_h = max(s["w"], s["h"]) / 2, max(s["w"], s["h"]) / 2
            vals = [("x-", s["x"] - half_w), ("x+", s["x"] + half_w),
                    ("y-", s["y"] - half_h), ("y+", s["y"] + half_h)]
        for what, v in vals:
            if not (-1e-9 <= v <= 1 + 1e-9):
                problems.append(f"{data['name']}: {area['id']} {area['input']} {what}={v:.4f} out of 0..1")
    return problems


OVERLAY_COLOURS = ["#54adf7", "#4fd28a", "#f2b34c", "#ff5c6c", "#4ee0ff", "#c58bff"]


def write_overlay(doc: Doc, areas: Areas, path: Path) -> None:
    """Base render plus every area drawn translucent with its input name."""
    over = Doc(doc.width, doc.height, BG_BASE)
    over.parts = list(doc.parts)
    for i, (input_key, shape) in enumerate(areas.items):
        colour = OVERLAY_COLOURS[i % len(OVERLAY_COLOURS)]
        k = shape["kind"]
        if k == "rect":
            over.add(f'<g opacity="0.45">{svg_rrect(shape["x"], shape["y"], shape["w"], shape["h"], 3, colour, colour, 2)}</g>')
            lx, ly = shape["x"] + shape["w"] / 2, shape["y"] + shape["h"] / 2
        elif k == "ellipse":
            over.add(f'<g opacity="0.45">{svg_ellipse(shape["cx"], shape["cy"], shape["rx"], shape["ry"], colour, colour, 2)}</g>')
            lx, ly = shape["cx"], shape["cy"]
        elif k == "polygon":
            pts = [(p[0], p[1]) for p in shape["points"]]
            over.add(f'<g opacity="0.45">{svg_poly(pts, colour, colour, 2)}</g>')
            lx = sum(p[0] for p in pts) / len(pts)
            ly = sum(p[1] for p in pts) / len(pts)
        else:
            over.add(f'<g opacity="0.55">{svg_arrow(shape["x"], shape["y"], shape["w"], shape["h"], shape["rotation"], colour)}</g>')
            lx, ly = shape["x"], shape["y"]
        over.add(svg_text(lx, ly, input_key.split(":", 1)[1], 15, "#000000", weight="700"))
    over.write_png(path)


def main() -> int:
    here = Path(__file__).resolve().parent
    ap = argparse.ArgumentParser(description="Generate the bundled default image-maps.")
    ap.add_argument("--out", type=Path, default=here.parent / "src-tauri" / "resources" / "imagemaps",
                    help="image-map root to write into")
    ap.add_argument("--overlays", type=Path, default=None,
                    help="optional directory for review overlay renders")
    args = ap.parse_args()

    builders = [
        lambda: build_keyboard(*kb_us()),
        lambda: build_keyboard(*kb_de()),
        build_xbox,
        build_playstation,
    ]

    problems: list[str] = []
    for meta, build in zip(BUNDLED, builders):
        doc, areas = build()
        data = write_map(args.out, meta, doc, areas)
        problems += check(data)

        png = args.out / meta[0] / "image.png"
        inputs = [a["input"] for a in data["areas"]]
        dupes = sorted({i for i in inputs if inputs.count(i) > 1})
        print(f"{meta[1]:<24} {len(data['areas']):>4} areas  "
              f"{png.stat().st_size / 1024:>7.1f} KiB  {int(doc.width)}x{int(doc.height)}"
              + (f"  multi-area inputs: {', '.join(dupes)}" if dupes else ""))

        if args.overlays:
            args.overlays.mkdir(parents=True, exist_ok=True)
            write_overlay(doc, areas, args.overlays / f"{meta[1].replace(' ', '-').lower()}.png")

    for p in problems:
        print(f"PROBLEM: {p}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
