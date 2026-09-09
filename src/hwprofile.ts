// Hardware profile types (mirroring profile.json, format 1) plus the small
// helpers both renderers (Konva editor, SVG viewer) share.

import type { JoyInput } from "./types";

export type SymbolKind = "arrow" | "cw" | "ccw";

// All coordinates normalized 0..1 relative to the image's natural size
// (x -> width, y -> height); `size` relative to the width; rotation in
// degrees, clockwise, around the shape's own center.
export type RectShape = { kind: "rect"; x: number; y: number; w: number; h: number; rotation: number };
export type EllipseShape = { kind: "ellipse"; cx: number; cy: number; rx: number; ry: number; rotation: number };
export type PolygonShape = { kind: "polygon"; points: [number, number][] };
export type SymbolShape = { kind: "symbol"; symbol: SymbolKind; x: number; y: number; size: number; rotation: number };
export type Shape = RectShape | EllipseShape | PolygonShape | SymbolShape;

export interface HwImage {
  id: string;
  file: string;
  label: string;
}

export interface HwArea {
  id: string;
  // SDL-level input key: `button:<n>`, `hat:<n>:<dir>`, `axis:<n>`.
  input: string;
  // References HwImage.id.
  image: string;
  shape: Shape;
}

export interface HwProfile {
  format: number;
  id: string;
  name: string;
  hardware_id: string;
  hardware_name: string;
  variant: string;
  images: HwImage[];
  areas: HwArea[];
}

export interface HwProfileSummary {
  id: string;
  name: string;
  hardware_id: string;
  hardware_name: string;
  variant: string;
  source: "bundled" | "user";
  image_count: number;
  area_count: number;
}

export type HighlightClass = "bound" | "none";

// Input key for a live event: buttons only while pressed, hats only when not
// centered, axes always. `null` when the event does not name an input.
export function inputKey(ev: JoyInput): string | null {
  switch (ev.kind) {
    case "button":
      return ev.pressed ? `button:${ev.index}` : null;
    case "hat":
      return ev.direction === "centered" ? null : `hat:${ev.index}:${ev.direction}`;
    case "axis":
      return `axis:${ev.index}`;
  }
}

// Short display label for an input key, e.g. "button:5" -> "Button 5".
export function inputLabel(key: string): string {
  const [kind, index, dir] = key.split(":");
  switch (kind) {
    case "button":
      return `Button ${index}`;
    case "axis":
      return `Axis ${index}`;
    case "hat":
      return `Hat ${index} ${dir}`;
    default:
      return key;
  }
}

// Hardware ids are SC Product GUIDs; compare case-insensitively.
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

// Pixel placement of a symbol in a W x H pixel space: center, uniform scale
// (100 path units == size * W px) and rotation.
export function symbolPx(s: SymbolShape, W: number, H: number): { x: number; y: number; scale: number; rotation: number } {
  return { x: s.x * W, y: s.y * H, scale: (s.size * W) / 100, rotation: s.rotation };
}

// Polygon vertices as a flat [x0, y0, x1, y1, ...] pixel list.
export function polygonPx(p: PolygonShape, W: number, H: number): number[] {
  return p.points.flatMap(([x, y]) => [x * W, y * H]);
}
