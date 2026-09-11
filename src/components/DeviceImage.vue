<script setup lang="ts">
import { computed, ref } from "vue";
import { SYMBOL_PATHS, arcPath, polygonPx, rectRadiusPx, symbolPx, type Shape, type ImageMap } from "../imagemap";

const props = defineProps<{
  map: ImageMap;
  src: string;
  // Input keys currently lit (held, pulsing, or pinned in the list).
  active: Set<string>;
  // Data URL of a file in the image-map folder (image shapes), "" while unloaded.
  imageUrl: (file: string) => string;
}>();

// Natural size of the loaded image; the SVG viewBox uses it so all shape
// coordinates are plain pixels of the original image.
const W = ref(0);
const H = ref(0);

function onLoad(e: Event) {
  const img = e.target as HTMLImageElement;
  W.value = img.naturalWidth;
  H.value = img.naturalHeight;
}

// Only lit shapes are drawn at all: the image itself is the resting look.
const lit = computed<Shape[]>(() => props.map.shapes.filter((s) => props.active.has(s.input)));

// A shape's own colours override the tokens (the class defaults).
function style(s: Shape): Record<string, string> {
  const out: Record<string, string> = {};
  if (s.stroke) out.stroke = s.stroke;
  if (s.fill) out.fill = s.fill;
  return out;
}

function polyPoints(s: Shape): string {
  if (s.geometry.kind !== "polygon") return "";
  const px = polygonPx(s.geometry, W.value, H.value);
  const out: string[] = [];
  for (let i = 0; i < px.length; i += 2) out.push(`${px[i]},${px[i + 1]}`);
  return out.join(" ");
}

function symbolTransform(s: Shape): string {
  if (s.geometry.kind !== "symbol") return "";
  const p = symbolPx(s.geometry, W.value, H.value);
  return `translate(${p.x} ${p.y}) rotate(${p.rotation}) scale(${p.scaleX} ${p.scaleY}) translate(-50 -50)`;
}

function arcD(s: Shape): string {
  const g = s.geometry;
  if (g.kind === "arc") return arcPath(g.cx * W.value, g.cy * H.value, g.r * W.value, g.inner, g.angle, g.rotation);
  if (g.kind === "wedge") return arcPath(g.cx * W.value, g.cy * H.value, g.r * W.value, 0, g.angle, g.rotation);
  return "";
}
</script>

<template>
  <div class="device-image">
    <img :src="src" :alt="map.image.label" @load="onLoad" />
    <svg v-if="W && H" :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none">
      <template v-for="s in lit" :key="s.id">
        <rect
          v-if="s.geometry.kind === 'rect'"
          class="shape"
          :style="style(s)"
          :x="s.geometry.x * W"
          :y="s.geometry.y * H"
          :width="s.geometry.w * W"
          :height="s.geometry.h * H"
          :rx="rectRadiusPx(s.geometry, W, H)"
          :transform="`rotate(${s.geometry.rotation} ${(s.geometry.x + s.geometry.w / 2) * W} ${(s.geometry.y + s.geometry.h / 2) * H})`"
        />
        <ellipse
          v-else-if="s.geometry.kind === 'ellipse'"
          class="shape"
          :style="style(s)"
          :cx="s.geometry.cx * W"
          :cy="s.geometry.cy * H"
          :rx="s.geometry.rx * W"
          :ry="s.geometry.ry * H"
          :transform="`rotate(${s.geometry.rotation} ${s.geometry.cx * W} ${s.geometry.cy * H})`"
        />
        <polygon v-else-if="s.geometry.kind === 'polygon'" class="shape" :style="style(s)" :points="polyPoints(s)" />
        <path
          v-else-if="s.geometry.kind === 'symbol'"
          class="shape"
          :style="style(s)"
          :d="SYMBOL_PATHS[s.geometry.symbol]"
          :transform="symbolTransform(s)"
        />
        <path v-else-if="s.geometry.kind === 'arc' || s.geometry.kind === 'wedge'" class="shape" :style="style(s)" :d="arcD(s)" />
        <image
          v-else-if="s.geometry.kind === 'image' && imageUrl(s.geometry.file)"
          :href="imageUrl(s.geometry.file)"
          :x="(s.geometry.x - s.geometry.w / 2) * W"
          :y="(s.geometry.y - s.geometry.h / 2) * H"
          :width="s.geometry.w * W"
          :height="s.geometry.h * H"
          preserveAspectRatio="none"
          :transform="`rotate(${s.geometry.rotation} ${s.geometry.x * W} ${s.geometry.y * H})`"
        />
      </template>
    </svg>
  </div>
</template>

<style scoped>
/* min-width: 0 — as a flex item the box would otherwise refuse to shrink
   below the image's natural width and get clipped instead of scaling. */
.device-image {
  position: relative;
  display: inline-block;
  max-width: 100%;
  min-width: 0;
  line-height: 0;
}

/* The image sizes the box (fits width and, via the parent's --image-max-h,
   height); the SVG overlay stretches over it. */
.device-image img {
  display: block;
  max-width: 100%;
  max-height: var(--image-max-h, none);
  width: auto;
  height: auto;
}

.device-image svg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}

.shape {
  fill: var(--shape-fill);
  stroke: var(--shape-stroke);
  stroke-width: 2;
  vector-effect: non-scaling-stroke;
}
</style>
