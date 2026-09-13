// Image-map types (mirroring imagemap.json, format 4) plus the small helpers
// both renderers share.

import type { DeviceInfo, JoyInput } from "./types";

export type SymbolKind = "arrow" | "arrow2" | "rotate" | "curve";

// All coordinates normalized 0..1 relative to the image's natural size
// (x/w -> width, y/h -> height, radii -> width); rotation in degrees,
// clockwise, around the shape's own center — for an arc or wedge, where its
// sweep starts. A symbol is its 100x100 path stretched into a w x h box
// centered at (x, y); an image shape is its file stretched the same way, a
// path shape its own path data (the editor's text tool writes those: glyph
// outlines, so a map never needs a font).
export type RectGeometry = {
  kind: "rect";
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  // Corner radius as a fraction of the shorter side (0..0.5).
  radius: number;
};
export type EllipseGeometry = { kind: "ellipse"; cx: number; cy: number; rx: number; ry: number; rotation: number };
export type PolygonGeometry = { kind: "polygon"; points: [number, number][] };
// `angle`: the sweep of the ring symbols (`rotate`, `curve`), unset = their
// default; the straight arrows ignore it.
export type SymbolGeometry = {
  kind: "symbol";
  symbol: SymbolKind;
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  angle?: number;
};
// `r` outer radius, `inner` the inner one as a fraction of it (0..1).
export type ArcGeometry = { kind: "arc"; cx: number; cy: number; r: number; inner: number; angle: number; rotation: number };
export type WedgeGeometry = { kind: "wedge"; cx: number; cy: number; r: number; angle: number; rotation: number };
export type ImageGeometry = { kind: "image"; file: string; x: number; y: number; w: number; h: number; rotation: number };
export type PathGeometry = { kind: "path"; d: string; x: number; y: number; w: number; h: number; rotation: number };
export type Geometry =
  | RectGeometry
  | EllipseGeometry
  | PolygonGeometry
  | SymbolGeometry
  | ArcGeometry
  | WedgeGeometry
  | ImageGeometry
  | PathGeometry;
export type GeometryKind = Geometry["kind"];

export interface ImageFile {
  file: string;
  label: string;
}

// A drawn shape, lit on the image while its input is active. `stroke` /
// `fill` (`#rrggbb` or `#rrggbbaa`) override the app's default lit colours.
export interface Shape {
  id: string;
  // SDL-level input key: `button:<n>`, `hat:<n>:<dir>`, `axis:<n>` (joystick),
  // `key:<sc name>` (keyboard) or `pad:<sc name>` (gamepad).
  input: string;
  stroke?: string;
  fill?: string;
  geometry: Geometry;
}

export interface ImageMap {
  format: number;
  id: string;
  name: string;
  hardware_id: string;
  hardware_name: string;
  image: ImageFile;
  shapes: Shape[];
}

export interface ImageMapSummary {
  id: string;
  name: string;
  hardware_id: string;
  hardware_name: string;
  source: "bundled" | "user";
  shape_count: number;
}

// Files the image shapes of a map reference (the device image not included).
export function shapeImageFiles(map: ImageMap): string[] {
  const out: string[] = [];
  for (const s of map.shapes) {
    if (s.geometry.kind === "image" && !out.includes(s.geometry.file)) out.push(s.geometry.file);
  }
  return out;
}

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

// The parts of a keyboard / gamepad token name: a combo's modifiers and its
// last part ("lalt+x" -> ["lalt", "x"]); SC's both-triggers button is the
// two trigger buttons.
function tokenParts(name: string): string[] {
  if (name === "triggerl_r_btn") return ["triggerl_btn", "triggerr_btn"];
  return name.split("+").filter(Boolean);
}

// Everything after the last `+` of a combo token, e.g. "lalt+x" -> "x".
function comboTail(name: string): string {
  const parts = tokenParts(name);
  return parts[parts.length - 1] ?? "";
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

// Every image-map key a token lights up: a keyboard / gamepad combo lights
// its modifiers too (`kb1_ralt+mwheel_up` -> `key:ralt`, `key:mwheel_up`),
// anything else just its own key. The token's own key comes last.
export function inputKeysForToken(token: string, d: DeviceInfo): string[] {
  const main = inputKeyForToken(token, d);
  if (!main) return [];
  const prefix = token.startsWith("kb1_") ? "key" : token.startsWith("gp1_") ? "pad" : null;
  if (!prefix) return [main];
  const modifiers = tokenParts(token.slice(4)).slice(0, -1);
  return [...modifiers.map((m) => `${prefix}:${m}`), main];
}

// Hardware ids are SC Product GUIDs (or "keyboard" / "gamepad"); compare
// case-insensitively.
export function sameHardware(a: string | null | undefined, b: string | null | undefined): boolean {
  return !!a && !!b && a.toLowerCase() === b.toLowerCase();
}

// The path data a symbol or path shape draws (both live in the 100x100 box).
export function pathData(g: SymbolGeometry | PathGeometry): string {
  if (g.kind === "path") return g.d;
  if (g.symbol === "rotate") return arcArrowPath(symbolAngle(g), 2);
  if (g.symbol === "curve") return arcArrowPath(symbolAngle(g), 1);
  return SYMBOL_PATHS[g.symbol];
}

// The sweep a ring symbol draws: its own, else the default (`rotate` a 270°
// ring, `curve` a half turn).
export function symbolAngle(g: SymbolGeometry): number {
  return g.angle ?? (g.symbol === "rotate" ? 270 : 180);
}

export function hasAngle(g: SymbolGeometry): boolean {
  return g.symbol === "rotate" || g.symbol === "curve";
}

// Symbol outlines in a 100x100 box centered at (50,50), as SVG path data.
// `arrow` points right, `arrow2` both ways. The ring symbols are built by
// `arcArrowPath`.
export const SYMBOL_PATHS: Record<Exclude<SymbolKind, "rotate" | "curve">, string> = {
  arrow: "M5 40 H60 V20 L95 50 L60 80 V60 H5 Z",
  arrow2: "M5 50 L30 25 V40 H70 V25 L95 50 L70 75 V60 H30 V75 Z",
};

// A ring segment with an arrowhead at its clockwise end (`heads` 1) or at
// both ends (`heads` 2: the rotate symbol, SC does not tell the twist
// directions apart), in the 100x100 box: `sweep` degrees centered on the
// top, so the opening sits at the bottom. Ring 24..40 from the center,
// heads 16..48 with the tip on the ring's middle.
export function arcArrowPath(sweep: number, heads: 1 | 2): string {
  const c = 50;
  const ro = 40;
  const ri = 24;
  const ho = 48;
  const hi = 16;
  const rm = 32;
  // Degrees of arc a head takes, so its tip lands on the sweep's end.
  const hl = 20;
  const s = Math.min(Math.max(sweep, 2 * hl + 5), 359.99);
  const a0 = -90 - s / 2;
  const a1 = -90 + s / 2;
  const bs = heads === 2 ? a0 + hl : a0;
  const be = a1 - hl;
  const pt = (r: number, deg: number): string => {
    const a = (deg * Math.PI) / 180;
    return `${(c + r * Math.cos(a)).toFixed(2)} ${(c + r * Math.sin(a)).toFixed(2)}`;
  };
  const large = be - bs > 180 ? 1 : 0;
  let d = `M${pt(ro, bs)} A${ro} ${ro} 0 ${large} 1 ${pt(ro, be)}`;
  d += ` L${pt(ho, be)} L${pt(rm, a1)} L${pt(hi, be)} L${pt(ri, be)}`;
  d += ` A${ri} ${ri} 0 ${large} 0 ${pt(ri, bs)}`;
  if (heads === 2) d += ` L${pt(hi, bs)} L${pt(rm, a0)} L${pt(ho, bs)}`;
  return `${d} Z`;
}

// Pixel placement of a symbol, path or image shape in a W x H pixel space:
// center, per-axis scale (100 path units == w * W px by h * H px) and
// rotation.
export function symbolPx(
  s: SymbolGeometry | ImageGeometry | PathGeometry,
  W: number,
  H: number,
): { x: number; y: number; scaleX: number; scaleY: number; rotation: number } {
  return { x: s.x * W, y: s.y * H, scaleX: (s.w * W) / 100, scaleY: (s.h * H) / 100, rotation: s.rotation };
}

// Polygon vertices as a flat [x0, y0, x1, y1, ...] pixel list.
export function polygonPx(p: PolygonGeometry, W: number, H: number): number[] {
  return p.points.flatMap(([x, y]) => [x * W, y * H]);
}

function polar(cx: number, cy: number, r: number, deg: number): [number, number] {
  const a = (deg * Math.PI) / 180;
  return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
}

// SVG path of a ring segment (`inner` > 0) or pie slice (`inner` == 0) in
// pixels: `angle` degrees of sweep clockwise from `rotation`. A full 360° is
// drawn a hair short, so the arc commands stay well defined.
export function arcPath(cx: number, cy: number, r: number, inner: number, angle: number, rotation: number): string {
  const sweep = Math.min(Math.max(angle, 0), 359.99);
  const large = sweep > 180 ? 1 : 0;
  const [x0, y0] = polar(cx, cy, r, rotation);
  const [x1, y1] = polar(cx, cy, r, rotation + sweep);
  if (inner <= 0) return `M${cx} ${cy} L${x0} ${y0} A${r} ${r} 0 ${large} 1 ${x1} ${y1} Z`;
  const ri = r * inner;
  const [xi0, yi0] = polar(cx, cy, ri, rotation);
  const [xi1, yi1] = polar(cx, cy, ri, rotation + sweep);
  return `M${x0} ${y0} A${r} ${r} 0 ${large} 1 ${x1} ${y1} L${xi1} ${yi1} A${ri} ${ri} 0 ${large} 0 ${xi0} ${yi0} Z`;
}

// Corner radius of a rect in pixels.
export function rectRadiusPx(r: RectGeometry, W: number, H: number): number {
  return r.radius * Math.min(r.w * W, r.h * H);
}
