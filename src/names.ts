// User-given names (image-maps now, device names later): letters, digits,
// space, `_`, `-` and brackets of any kind only, so nothing ever needs
// escaping in a file name or a URL. Mirrors `imagemap::is_name_char` /
// `sanitize_name`.

export const NAME_MAX = 64;

const NOT_NAME = /[^A-Za-z0-9 _()[\]{}-]/g;

// Drop what a name must not contain; spaces are kept as typed, so this can
// run on every keystroke.
export function stripNameChars(s: string): string {
  return s.replace(NOT_NAME, "").slice(0, NAME_MAX);
}

// The final form: other characters become spaces, runs of spaces collapse,
// the ends are trimmed; empty falls back to `fallback`.
export function sanitizeName(s: string, fallback: string): string {
  const out = s.replace(NOT_NAME, " ").replace(/\s+/g, " ").trim().slice(0, NAME_MAX).trim();
  return out || fallback;
}
