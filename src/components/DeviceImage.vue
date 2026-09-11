<script setup lang="ts">
import { computed, ref } from "vue";
import { SYMBOL_PATHS, polygonPx, symbolPx, type Area, type ImageMap } from "../imagemap";

const props = defineProps<{
  map: ImageMap;
  src: string;
  // Input keys currently lit (held, pulsing, or pinned in the list).
  active: Set<string>;
}>();

// Natural size of the loaded image; the SVG viewBox uses it so all area
// coordinates are plain pixels of the original image.
const W = ref(0);
const H = ref(0);

function onLoad(e: Event) {
  const img = e.target as HTMLImageElement;
  W.value = img.naturalWidth;
  H.value = img.naturalHeight;
}

// Only lit shapes are drawn at all: the image itself is the resting look.
const lit = computed<Area[]>(() => props.map.areas.filter((a) => props.active.has(a.input)));

function polyPoints(a: Area): string {
  if (a.shape.kind !== "polygon") return "";
  const px = polygonPx(a.shape, W.value, H.value);
  const out: string[] = [];
  for (let i = 0; i < px.length; i += 2) out.push(`${px[i]},${px[i + 1]}`);
  return out.join(" ");
}

function symbolTransform(a: Area): string {
  if (a.shape.kind !== "symbol") return "";
  const s = symbolPx(a.shape, W.value, H.value);
  return `translate(${s.x} ${s.y}) rotate(${s.rotation}) scale(${s.scaleX} ${s.scaleY}) translate(-50 -50)`;
}
</script>

<template>
  <div class="device-image">
    <img :src="src" :alt="map.image.label" @load="onLoad" />
    <svg v-if="W && H" :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none">
      <template v-for="a in lit" :key="a.id">
        <rect
          v-if="a.shape.kind === 'rect'"
          class="area"
          :x="a.shape.x * W"
          :y="a.shape.y * H"
          :width="a.shape.w * W"
          :height="a.shape.h * H"
          :transform="`rotate(${a.shape.rotation} ${(a.shape.x + a.shape.w / 2) * W} ${(a.shape.y + a.shape.h / 2) * H})`"
        />
        <ellipse
          v-else-if="a.shape.kind === 'ellipse'"
          class="area"
          :cx="a.shape.cx * W"
          :cy="a.shape.cy * H"
          :rx="a.shape.rx * W"
          :ry="a.shape.ry * H"
          :transform="`rotate(${a.shape.rotation} ${a.shape.cx * W} ${a.shape.cy * H})`"
        />
        <polygon v-else-if="a.shape.kind === 'polygon'" class="area" :points="polyPoints(a)" />
        <path
          v-else-if="a.shape.kind === 'symbol'"
          class="area"
          :d="SYMBOL_PATHS[a.shape.symbol]"
          :transform="symbolTransform(a)"
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

.area {
  fill: var(--shape-fill);
  stroke: var(--shape-stroke);
  stroke-width: 2;
  vector-effect: non-scaling-stroke;
}
</style>
