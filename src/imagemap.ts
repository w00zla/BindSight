// Image-map types (mirroring imagemap.json, format 3) plus the small helpers
// both renderers share.

import type { DeviceInfo, JoyInput } from "./types";

export type SymbolKind = "arrow" | "cw" | "ccw";

// All coordinates normalized 0..1 relative to the image's natural size
// (x/w -> width, y/h -> height); rotation in degrees, clockwise, around the
// shape's own center. A symbol is its 100x100 path stretched into a w x h box
// centered at (x, y).
export type RectShape = { kind: "rect"; x: number; y: number; w: number; h: number; rotation: number };
export type EllipseShape = { kind: "ellipse"; cx: number; cy: number; rx: number; ry: number; rotation: number };
export type PolygonShape = { kind: "polygon"; points: [number, number][] };
export type SymbolShape = { kind: "symbol"; symbol: SymbolKind; x: number; y: number; w: number; h: number; rotation: number };
export type Shape = RectShape | EllipseShape | PolygonShape | SymbolShape;

export interface ImageFile {
  file: string;
  label: string;
}

export interface Area {
  id: string;
  // SDL-level input key: `button:<n>`, `hat:<n>:<dir>`, `axis:<n>` (joystick),
  // `key:<sc name>` (keyboard) or `pad:<sc name>` (gamepad).
  input: string;
  shape: Shape;
}

export interface ImageMap {
  format: number;
  id: string;
  name: string;
  hardware_id: string;
  hardware_name: string;
  image: ImageFile;
  areas: Area[];
}

export interface ImageMapSummary {
  id: string;
  name: string;
  hardware_id: string;
  hardware_name: string;
  source: "bundled" | "user";
  area_count: number;
}

export type HighlightClass = "bound" | "none";

// Input key for a live event: buttons and keys only while pressed, hats only
// when not centered, axes always. `null` when the event does not name an input.
export function inputKey(ev: JoyInput): string | null {
  switch (ev.kind) {
    case "button":
      return ev.pressed ? `button:${ev.index}` : null;
    case "hat":
      return ev.direction === "centered" ? null : `hat:${ev.index}:${ev.direction}`;
    case "axis":
      return `axis:${ev.index}`;
    case "padbutton":
      return ev.pressed ? `pad:${ev.name}` : null;
    case "padaxis":
      return `pad:${ev.name}`;
    case "key":
      return ev.pressed ? `key:${ev.name}` : null;
  }
}

// Everything after the last `+` of a combo token, e.g. "lalt+x" -> "x".
function comboTail(name: string): string {
  return name.slice(name.lastIndexOf("+") + 1);
}

// Image-map input key for an SC token on a device. Joystick tokens undo the +1
// offset of button/hat numbering and go through the device's HID-derived axis
// names (`js2_rotz` -> the SDL index whose name is `rotz`); keyboard and
// gamepad tokens map 1:1 to the event name, a combo to its last part.
export function inputKeyForToken(token: string, d: DeviceInfo): string | null {
  if (token.startsWith("kb1_")) {
    const name = comboTail(token.slice(4));
    return name ? `key:${name}` : null;
  }
  if (token.startsWith("gp1_")) {
    const name = comboTail(token.slice(4));
    return name ? `pad:${name}` : null;
  }
  const b = token.match(/^js\d+_button(\d+)$/);
  if (b) return `button:${Number(b[1]) - 1}`;
  const h = token.match(/^js\d+_hat(\d+)_(up|down|left|right)$/);
  if (h) return `hat:${Number(h[1]) - 1}:${h[2]}`;
  const a = token.match(/^js\d+_([a-z0-9]+)$/);
  if (a) {
    const i = d.axes.indexOf(a[1]);
    if (i >= 0) return `axis:${i}`;
  }
  return null;
}

// Hardware ids are SC Product GUIDs (or "keyboard" / "gamepad"); compare
// case-insensitively.
export function sameHardware(a: string | null | undefined, b: string | null | undefined): boolean {
  return !!a && !!b && a.toLowerCase() === b.toLowerCase();
}

// Symbol outlines in a 100x100 box centered at (50,50), as SVG path data.
// `arrow` points right; `cw`/`ccw` are a 270° ring with an arrowhead at the
// bottom, running clockwise / counter-clockwise (screen coordinates, y down).
export const SYMBOL_PATHS: Record<SymbolKind, string> = {
  arrow: "M5 40 H60 V20 L95 50 L60 80 V60 H5 Z",
  cw: "M10 50 A40 40 0 1 1 50 90 L50 100 L30 82 L50 64 L50 74 A24 24 0 1 0 26 50 Z",
  ccw: "M90 50 A40 40 0 1 0 50 90 L50 100 L70 82 L50 64 L50 74 A24 24 0 1 1 74 50 Z",
};

// Pixel placement of a symbol in a W x H pixel space: center, per-axis scale
// (100 path units == w * W px by h * H px) and rotation.
export function symbolPx(s: SymbolShape, W: number, H: number): { x: number; y: number; scaleX: number; scaleY: number; rotation: number } {
  return { x: s.x * W, y: s.y * H, scaleX: (s.w * W) / 100, scaleY: (s.h * H) / 100, rotation: s.rotation };
}

// Polygon vertices as a flat [x0, y0, x1, y1, ...] pixel list.
export function polygonPx(p: PolygonShape, W: number, H: number): number[] {
  return p.points.flatMap(([x, y]) => [x * W, y * H]);
}
