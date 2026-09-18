<script setup lang="ts">
// A floating, top-z preview of one image-map with a fixed input highlight.
// Reusable: it renders whatever map + `active` key set it is handed, and
// positions itself either pinned to a screen corner or offset from a mouse
// anchor (with an edge flip so it never leaves the viewport). It captures no
// pointer events, so it never gets in the way of what is underneath.
import { computed } from "vue";
import DeviceImage from "./DeviceImage.vue";
import type { ImageMap } from "../imagemap";
import type { OverlayPosition } from "../types";

const props = defineProps<{
  map: ImageMap;
  src: string;
  imageUrl: (file: string) => string;
  active: Set<string>;
  // The box's longer side in px.
  size: number;
  position: OverlayPosition;
  // Cursor position for the mouse-offset placement; ignored for a corner.
  anchor: { x: number; y: number } | null;
}>();

// Distance from a screen edge (corner modes) and from the cursor (offset).
const MARGIN = 16;
const OFFSET = 44;

const style = computed<Record<string, string>>(() => {
  const s = props.size;
  const out: Record<string, string> = {};
  if (props.position === "mouse-offset") {
    const a = props.anchor ?? { x: 0, y: 0 };
    // Flip to the other side of the cursor when the box's max footprint (`s`,
    // the longer side) would overflow. When flipped, anchor the box's far edge
    // to the cursor (`right`/`bottom`) so it hugs the cursor regardless of the
    // image's actual, usually smaller, size — no gap above/left of it.
    if (a.x + OFFSET + s + MARGIN > window.innerWidth) out.right = `${window.innerWidth - a.x + OFFSET}px`;
    else out.left = `${a.x + OFFSET}px`;
    if (a.y + OFFSET + s + MARGIN > window.innerHeight) out.bottom = `${window.innerHeight - a.y + OFFSET}px`;
    else out.top = `${a.y + OFFSET}px`;
    return out;
  }
  if (props.position.startsWith("top")) out.top = `${MARGIN}px`;
  else out.bottom = `${MARGIN}px`;
  if (props.position.endsWith("left")) out.left = `${MARGIN}px`;
  else out.right = `${MARGIN}px`;
  return out;
});
</script>

<template>
  <Teleport to="body">
    <div class="input-overlay" :style="[style, { width: `${size}px`, '--image-max-h': `${size}px` }]">
      <DeviceImage :map="map" :src="src" :active="active" :imageUrl="imageUrl" />
    </div>
  </Teleport>
</template>

<style scoped>
.input-overlay {
  position: fixed;
  z-index: 100;
  padding: 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel, 8px);
  background: var(--bg-surface);
  box-shadow: 0 8px 28px rgb(0 0 0 / 45%);
  pointer-events: none;
  line-height: 0;
}
</style>
