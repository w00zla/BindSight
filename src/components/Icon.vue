<script lang="ts">
// SVG inner markup only (paths/shapes on a 24x24 grid, stroke-based, one
// consistent style). Sourced from the design mockups where available; see
// Icon.vue's usage sites for any hand-drawn fallbacks noted at the call site.
export const ICONS = {
  monitor: '<path d="M2 12h4l3-8 4 16 3-8h6"/>',
  // Two inputs wired to one action: the binding.
  bindings: '<rect x="3" y="4" width="7" height="6" rx="1"/><rect x="3" y="14" width="7" height="6" rx="1"/><rect x="14" y="9" width="7" height="6" rx="1"/><path d="M10 7h2a2 2 0 0 1 2 2"/><path d="M10 17h2a2 2 0 0 0 2-2"/>',
  devices: '<path d="M12 13V7"/><circle cx="12" cy="5" r="2"/><path d="M5 17h14l-1.5-4h-11z"/><path d="M4 17v2h16v-2"/>',
  // Device kinds (the Devices icon above doubles as the joystick).
  keyboard: '<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>',
  // A plain question mark: the editor controls toggle.
  help: '<path d="M8 8.5a4 4 0 1 1 5.6 3.7c-1 .5-1.6 1.3-1.6 2.3v.5"/><path d="M12 19h.01"/>',
  gamepad: '<path d="M6 11h4M8 9v4"/><path d="M15 12h.01M18 10h.01"/><path d="M17.3 5H6.7a4 4 0 0 0-3.97 3.52l-.7 5.6A3 3 0 0 0 5 17.5c1 0 1.9-.5 2.4-1.3L9 14h6l1.6 2.2c.5.8 1.4 1.3 2.4 1.3a3 3 0 0 0 2.97-3.38l-.7-5.6A4 4 0 0 0 17.3 5z"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  refresh: '<path d="M21 12a9 9 0 1 1-3-6.7"/><path d="M21 3v6h-6"/>',
  bolt: '<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>',
  link: '<path d="M9 17H7a5 5 0 0 1 0-10h2"/><path d="M15 7h2a5 5 0 1 1 0 10h-2"/><path d="M8 12h8"/>',
  list: '<path d="M8 6h13M8 12h13M8 18h13"/><path d="M3 6h.01M3 12h.01M3 18h.01"/>',
  log: '<path d="M4 4h16v16H4z"/><path d="M8 9h8M8 13h6"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>',
  warning: '<path d="M12 3 2 21h20L12 3z"/><path d="M12 10v5M12 18v.5"/>',
  rotate: '<path d="M3 12a9 9 0 1 0 3-6.7"/><path d="M3 4v5h5"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a1 1 0 0 1 1-1h10"/>',
  // A console window with a prompt: the in-game console.
  terminal: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3"/><path d="M12 15h5"/>',
  image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-5-5-9 9"/>',
  // The image icon struck through: hidden from the stage.
  'image-off': '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-5-5-9 9"/><path d="M2 2l20 20"/>',
  'image-plus': '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M8 12h8M12 8v8"/>',
  // An open folder: pick a file from disk.
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v1"/><path d="M3 7v11a2 2 0 0 0 2 2h13a2 2 0 0 0 1.9-1.4L22 11H7.5a2 2 0 0 0-1.9 1.4L3 19"/>',
  lock: '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>',
  clone: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a1 1 0 0 1 1-1h10"/>',
  // Hand-drawn: a pencil over its stroke, for entering the editor.
  edit: '<path d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16z"/><path d="m13.5 6.5 4 4"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  close: '<path d="M6 6l12 12M18 6 6 18"/>',
  // Window controls.
  minimize: '<path d="M5 12h14"/>',
  maximize: '<rect x="5" y="5" width="14" height="14" rx="1"/>',
  restore: '<rect x="4" y="8" width="12" height="12" rx="1"/><path d="M8 8V5a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1h-3"/>',
  'chevron-down': '<path d="m6 9 6 6 6-6"/>',
  'chevron-up': '<path d="m6 15 6-6 6 6"/>',
  'chevron-right': '<path d="m9 6 6 6-6 6"/>',
  'chevron-left': '<path d="m15 6-6 6 6 6"/>',
  // Z-order: one step up / down, all the way to the top / bottom.
  'z-top': '<path d="M5 12h14"/><path d="m8 8 4-4 4 4"/><path d="M12 4v11"/>',
  'z-up': '<path d="m6 14 6-6 6 6"/>',
  'z-down': '<path d="m6 10 6 6 6-6"/>',
  'z-bottom': '<path d="M5 12h14"/><path d="m8 16 4 4 4-4"/><path d="M12 20V9"/>',
  unfold: '<path d="m7 15 5 5 5-5"/><path d="m7 9 5-5 5 5"/>',
  // Two collapsed buckets: chevron plus head line, twice.
  group: '<path d="m4 4 3 3-3 3"/><path d="M11 7h10"/><path d="m4 14 3 3-3 3"/><path d="M11 17h10"/>',
  fold: '<path d="m7 20 5-5 5 5"/><path d="m7 4 5 5 5-5"/>',
  target: '<circle cx="12" cy="12" r="7"/><circle cx="12" cy="12" r="2"/><path d="M12 2v4M12 18v4M2 12h4M18 12h4"/>',
  swap: '<path d="M8 3 4 7l4 4"/><path d="M4 7h16"/><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/>',
  upload: '<path d="M12 15V3M7 8l5-5 5 5"/><path d="M4 21h16"/>',
  download: '<path d="M12 3v12M7 10l5 5 5-5"/><path d="M4 21h16"/>',
  // The GitHub mark (Simple Icons, CC0): a filled shape, pass fill="currentColor".
  github: '<path stroke="none" d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/>',
  check: '<path d="m5 12 5 5L20 7"/>',
  // A floppy disk, because saving.
  save: '<path d="M5 3h11l3 3v15H5z"/><path d="M8 3v6h8V3"/><rect x="8" y="14" width="8" height="7"/>',
  'arrow-left': '<path d="M19 12H5"/><path d="m11 18-6-6 6-6"/>',
  'swap-circle': '<path d="M20 12a8 8 0 0 1-14 5.3"/><path d="M4 12a8 8 0 0 1 14-5.3"/><path d="M18 3v4h-4"/><path d="M6 21v-4h4"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l4 2"/>',
  file: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M14 3v6h6"/>',
  history: '<path d="M3 12a9 9 0 1 0 3-6.7"/><path d="M3 4v5h5"/><path d="M12 8v4l3 2"/>',
  compare: '<path d="M12 3v18"/><rect x="3" y="6" width="6" height="12" rx="1"/><rect x="15" y="6" width="6" height="12" rx="1"/>',
  'arrow-right': '<path d="M4 12h14M13 6l6 6-6 6"/>',
  eye: '<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>',
  'eye-off': '<path d="M10.6 6.2A9.9 9.9 0 0 1 12 6c6.4 0 10 7 10 7a17 17 0 0 1-3 3.8"/><path d="M6.6 7.6A17 17 0 0 0 2 13s3.6 7 10 7a9.8 9.8 0 0 0 4.5-1.1"/><path d="M9.9 10.9a3 3 0 0 0 4.2 4.2"/><path d="M3 3l18 18"/>',
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>',
  redo: '<path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0 0 12h3"/>',
  'shape-rect': '<rect x="4" y="5" width="16" height="14" rx="1"/>',
  'shape-ellipse': '<ellipse cx="12" cy="12" rx="8" ry="6"/>',
  'shape-polygon': '<path d="M12 3l8 6-3 10H7L4 9z"/>',
  // A drop: pick a colour.
  drop: '<path d="M12 3s6 7 6 11a6 6 0 0 1-12 0c0-4 6-11 6-11z"/>',
  // Any shape: a square behind a circle.
  shape: '<rect x="3" y="3" width="11" height="11" rx="1"/><circle cx="16" cy="16" r="5"/>',
  'shape-arrow': '<path d="M4 12h14M13 6l6 6-6 6"/>',
  'shape-arrow2': '<path d="M4 12h16"/><path d="m8 8-4 4 4 4"/><path d="m16 8 4 4-4 4"/>',
  // A ring open at the bottom with an arrowhead at each end: rotation either way.
  // A half-turn ring with one arrowhead: the curved arrow symbol.
  'shape-curve': '<path d="M5 14a8 8 0 0 1 14-5"/><path d="M15 4l4 5-5 3"/>',
  'shape-rotate': '<path d="M6.3 17.7a8 8 0 1 1 11.4 0"/><path d="M3 15l3.3 2.7 3-2.7"/><path d="M21 15l-3.3 2.7-3-2.7"/>',
  'shape-arc': '<path d="M4 16a8 8 0 0 1 16 0"/><path d="M8 16a4 4 0 0 1 8 0"/><path d="M4 16h4M16 16h4"/>',
  'shape-wedge': '<path d="M12 12V3a9 9 0 0 1 9 9z"/>',
  // A capital T: the text tool (its shapes are paths).
  'shape-text': '<path d="M5 5h14"/><path d="M12 5v14"/>',
  // A picture frame with a hill, no sun: an image shape, not an image-map.
  'shape-image': '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m3 17 5-5 4 4 3-3 6 6"/>',
} as const

export type IconName = keyof typeof ICONS
</script>

<script setup lang="ts">
withDefaults(defineProps<{ name: IconName; size?: number; fill?: string }>(), {
  size: 16,
  fill: 'none',
})
</script>

<template>
  <!-- v-html renders ICONS[name], a constant defined above -- never user input -->
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    :fill="fill"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    v-html="ICONS[name]"
  />
</template>
