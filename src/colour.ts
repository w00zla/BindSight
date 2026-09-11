// Colour strings the pickers work with: `#rrggbb` or `#rrggbbaa`.

// A CSS custom property's value, trimmed.
export function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function colourHex(c: string): string {
  return c.slice(0, 7).toLowerCase();
}

// Alpha of a colour in percent (100 when it has none).
export function colourAlpha(c: string): number {
  return c.length === 9 ? Math.round((parseInt(c.slice(7, 9), 16) / 255) * 100) : 100;
}

export function composeColour(hex: string, alpha: number): string {
  const a = Math.round((Math.min(100, Math.max(0, alpha)) / 100) * 255);
  return a >= 255 ? hex : `${hex}${a.toString(16).padStart(2, "0")}`;
}

// `#rrggbb` from user input (with or without the `#`), null when invalid.
export function parseHex(input: string): string | null {
  const h = input.trim().toLowerCase();
  const full = h.startsWith("#") ? h : `#${h}`;
  return /^#[0-9a-f]{6}$/.test(full) ? full : null;
}

// An `rgba(r, g, b, a)` token value as `#rrggbb[aa]`; other values pass through.
export function rgbaToHex(c: string): string {
  const m = /rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\)/.exec(c);
  if (!m) return c;
  const hex = `#${[m[1], m[2], m[3]].map((n) => Number(n).toString(16).padStart(2, "0")).join("")}`;
  return composeColour(hex, Math.round(Number(m[4] ?? 1) * 100));
}

// The `--shape-palette` token as a list of `#rrggbb` swatches.
export function readPalette(): string[] {
  return cssVar("--shape-palette")
    .split(",")
    .map((c) => c.trim().toLowerCase())
    .filter((c) => /^#[0-9a-f]{6}$/.test(c));
}
