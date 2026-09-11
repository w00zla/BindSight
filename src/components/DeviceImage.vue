<script setup lang="ts">
import { computed, ref } from "vue";
import { SYMBOL_PATHS, polygonPx, symbolPx, type HighlightClass, type Area, type ImageMap } from "../imagemap";

const props = defineProps<{
  map: ImageMap;
  src: string;
  // input key -> highlight class for the currently active inputs
  active: Map<string, HighlightClass>;
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

const areas = computed<Area[]>(() => props.map.areas);

function cls(a: Area): string {
  const c = props.active.get(a.input);
  return c ? `area ${c}` : "area";
}

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
      <template v-for="a in areas" :key="a.id">
        <rect
          v-if="a.shape.kind === 'rect'"
          :class="cls(a)"
          :x="a.shape.x * W"
          :y="a.shape.y * H"
          :width="a.shape.w * W"
          :height="a.shape.h * H"
          :transform="`rotate(${a.shape.rotation} ${(a.shape.x + a.shape.w / 2) * W} ${(a.shape.y + a.shape.h / 2) * H})`"
        />
        <ellipse
          v-else-if="a.shape.kind === 'ellipse'"
          :class="cls(a)"
          :cx="a.shape.cx * W"
          :cy="a.shape.cy * H"
          :rx="a.shape.rx * W"
          :ry="a.shape.ry * H"
          :transform="`rotate(${a.shape.rotation} ${a.shape.cx * W} ${a.shape.cy * H})`"
        />
        <polygon v-else-if="a.shape.kind === 'polygon'" :class="cls(a)" :points="polyPoints(a)" />
        <path
          v-else-if="a.shape.kind === 'symbol'"
          :class="cls(a)"
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
  fill: rgba(128, 128, 128, 0.08);
  stroke: rgba(128, 128, 128, 0.5);
  stroke-width: 1.5;
  vector-effect: non-scaling-stroke;
}

/* blue: the input has SC bindings (same as the live tile) */
.area.bound {
  fill: rgba(57, 108, 216, 0.55);
  stroke: #396cd8;
  stroke-width: 2;
}

/* grey: nothing bound (axes always) */
.area.none {
  fill: rgba(128, 128, 128, 0.55);
  stroke: rgba(128, 128, 128, 0.9);
  stroke-width: 2;
}
</style>
