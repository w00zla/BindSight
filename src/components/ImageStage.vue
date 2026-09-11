<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import DeviceImage from "./DeviceImage.vue";
import { deviceKey, deviceName } from "../devices";
import Icon from "./Icon.vue";
import Splitter from "./Splitter.vue";
import { persistedRef } from "../persist";
import { colourAlpha, colourHex, composeColour, cssVar, parseHex, readPalette } from "../colour";
import type { DeviceInfo, ImageMapView } from "../types";

const props = defineProps<{
  views: ImageMapView[];
  // Connected devices without an image-map: shown as placeholder tiles.
  placeholders: DeviceInfo[];
  // Device keys in the app's display order — the tile order until the user
  // swaps tiles by hand.
  sequence: string[];
  // Stage height in px (the parent owns the vertical splitter).
  height: number;
  imgSrc: (id: string, file: string) => string;
  activeFor: (sdlGuid: string) => Set<string>;
}>();

type Tile = { device: DeviceInfo; view: ImageMapView | null };

// Tile order: user-swapped device keys first (remembered), the rest in the
// app's display order.
const ORDER_KEY = "bindsight.stage.order";
const order = ref<string[]>([]);
try {
  const parsed = JSON.parse(localStorage.getItem(ORDER_KEY) ?? "[]") as unknown;
  if (Array.isArray(parsed) && parsed.every((x) => typeof x === "string")) order.value = parsed;
} catch {
  /* SDL order */
}

const tiles = computed<Tile[]>(() => {
  const all: Tile[] = [
    ...props.views.map((v) => ({ device: v.device, view: v })),
    ...props.placeholders.map((d) => ({ device: d, view: null })),
  ];
  const rank = (t: Tile) => {
    const key = deviceKey(t.device);
    const i = order.value.indexOf(key);
    return i === -1 ? order.value.length + props.sequence.indexOf(key) : i;
  };
  return all.sort((a, b) => rank(a) - rank(b));
});

function swap(i: number) {
  const ids = tiles.value.map((t) => deviceKey(t.device));
  [ids[i], ids[i + 1]] = [ids[i + 1], ids[i]];
  order.value = ids;
  try {
    localStorage.setItem(ORDER_KEY, JSON.stringify(ids));
  } catch {
    /* ignore */
  }
}

// --- tile background colour ---------------------------------------------

// Per device key, `#rrggbb[aa]`; unset = the panel colour. Picked in a
// small popover on the tile (the fill button in its caption).
const colours = persistedRef<Record<string, string>>("bindsight.stage.colours", {});
const colourOpen = ref<string | null>(null);
// The default tile colour first (clicking it drops the tile's own), then
// the palette.
const swatches = ref<string[]>([]);
let defaultColour = "";

function tileColour(key: string): string {
  return colours.value[key] ?? defaultColour;
}

function tileStyle(key: string, share: number): Record<string, string | number> {
  const own = colours.value[key];
  return own ? { flexGrow: share, background: own } : { flexGrow: share };
}

function toggleColour(key: string) {
  colourOpen.value = colourOpen.value === key ? null : key;
}

function pickSwatch(key: string, i: number, c: string) {
  if (i === 0) {
    const next = { ...colours.value };
    delete next[key];
    colours.value = next;
  } else {
    colours.value = { ...colours.value, [key]: composeColour(c, colourAlpha(tileColour(key))) };
  }
}

function setHex(key: string, e: Event) {
  const hex = parseHex((e.target as HTMLInputElement).value);
  if (hex) colours.value = { ...colours.value, [key]: composeColour(hex, colourAlpha(tileColour(key))) };
}

function setAlpha(key: string, e: Event) {
  colours.value = { ...colours.value, [key]: composeColour(colourHex(tileColour(key)), Number((e.target as HTMLInputElement).value)) };
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") colourOpen.value = null;
}

onMounted(() => {
  defaultColour = cssVar("--bg-surface").toLowerCase();
  swatches.value = [defaultColour, ...readPalette().filter((c) => c !== defaultColour)];
  window.addEventListener("keydown", onKey);
});
onUnmounted(() => window.removeEventListener("keydown", onKey));

// Width share per tile; draggable gutters between neighbours. Remembered per
// tile count so a HOTAS/HOSAS split survives a restart.
const MIN_SHARE = 0.06;
const shares = ref<number[]>([]);
const storeKey = (n: number) => `bindsight.stage.shares.${n}`;

function equalShares(n: number): number[] {
  return Array.from({ length: n }, () => 1 / n);
}

// Shares must sum to 1: they are flex-grow weights over the whole width, and
// a sum below 1 leaves that fraction of the stage empty.
function normalized(list: number[]): number[] {
  const sum = list.reduce((a, b) => a + b, 0);
  return sum > 0 ? list.map((x) => x / sum) : equalShares(list.length);
}

function loadShares(n: number): number[] {
  try {
    const raw = localStorage.getItem(storeKey(n));
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    if (Array.isArray(parsed) && parsed.length === n && parsed.every((x) => typeof x === "number" && x > 0)) {
      return normalized(parsed);
    }
  } catch {
    /* no storage — equal split */
  }
  return equalShares(n);
}

watch(
  () => tiles.value.length,
  (n) => {
    shares.value = n ? loadShares(n) : [];
  },
  { immediate: true },
);

const stageEl = ref<HTMLElement | null>(null);
// The gutter being dragged and the pair's shares when the drag began; keyed
// by gutter so a drag whose end never arrived cannot leak into the next one.
let dragStart: { gutter: number; left: number; pair: number } | null = null;

function persist() {
  try {
    localStorage.setItem(storeKey(shares.value.length), JSON.stringify(shares.value));
  } catch {
    /* ignore */
  }
}

// Shift width between tile i and i+1 by the gutter's drag offset.
function onDrag(i: number, deltaPx: number) {
  const width = stageEl.value?.getBoundingClientRect().width ?? 0;
  if (!width) return;
  if (dragStart?.gutter !== i) dragStart = { gutter: i, left: shares.value[i], pair: shares.value[i] + shares.value[i + 1] };
  const { left: left0, pair } = dragStart;
  const left = Math.min(Math.max(left0 + deltaPx / width, MIN_SHARE), pair - MIN_SHARE);
  shares.value[i] = left;
  shares.value[i + 1] = pair - left;
}

function onEnd() {
  dragStart = null;
  shares.value = normalized(shares.value);
  persist();
}

function onReset(i: number) {
  const pair = shares.value[i] + shares.value[i + 1];
  shares.value[i] = pair / 2;
  shares.value[i + 1] = pair / 2;
  persist();
}
</script>

<template>
  <div v-if="tiles.length" ref="stageEl" class="stage" :style="{ height: `${height}px`, '--image-max-h': `${height - 56}px` }">
    <template v-for="(t, i) in tiles" :key="t.device.index">
      <Splitter v-if="i > 0" direction="col" @drag="onDrag(i - 1, $event)" @end="onEnd" @reset="onReset(i - 1)" />
      <div class="tile" :class="{ placeholder: !t.view }" :style="tileStyle(deviceKey(t.device), shares[i] ?? 1)">
        <div class="caption">
          <span class="name">{{ deviceName(t.device) }}</span>
          <button v-if="i > 0" type="button" class="move" title="Move left" @click="swap(i - 1)">
            <Icon name="arrow-left" :size="14" />
          </button>
          <button v-if="i < tiles.length - 1" type="button" class="move" title="Move right" @click="swap(i)">
            <Icon name="arrow-right" :size="14" />
          </button>
          <button
            type="button"
            class="move"
            :class="{ on: colourOpen === deviceKey(t.device) }"
            title="Background colour"
            @click="toggleColour(deviceKey(t.device))"
          >
            <Icon name="drop" :size="14" />
          </button>
        </div>
        <!-- background colour picker, floating under the caption -->
        <div v-if="colourOpen === deviceKey(t.device)" class="colour-pop">
          <div class="swatches">
            <button
              v-for="(c, j) in swatches"
              :key="c"
              type="button"
              class="swatch"
              :class="{ on: colourHex(tileColour(deviceKey(t.device))) === c }"
              :style="{ background: c }"
              :title="j === 0 ? `Default · ${c}` : c"
              @click="pickSwatch(deviceKey(t.device), j, c)"
            />
          </div>
          <div class="line">
            <span class="swatch big" :style="{ background: tileColour(deviceKey(t.device)) }" />
            <input
              class="hex mono"
              :value="colourHex(tileColour(deviceKey(t.device)))"
              maxlength="7"
              spellcheck="false"
              @change="setHex(deviceKey(t.device), $event)"
            />
            <input
              type="range"
              class="range"
              min="0"
              max="100"
              :value="colourAlpha(tileColour(deviceKey(t.device)))"
              title="Opacity"
              @input="setAlpha(deviceKey(t.device), $event)"
            />
            <span class="mono val">{{ colourAlpha(tileColour(deviceKey(t.device))) }}%</span>
            <button type="button" class="move" title="Close" @click="colourOpen = null">
              <Icon name="close" :size="14" />
            </button>
          </div>
        </div>
        <DeviceImage
          v-if="t.view && imgSrc(t.view.map.id, t.view.map.image.file)"
          :map="t.view.map"
          :src="imgSrc(t.view.map.id, t.view.map.image.file)"
          :active="activeFor(t.device.sdl_guid)"
          :imageUrl="(f: string) => imgSrc(t.view!.map.id, f)"
        />
        <div v-else-if="!t.view" class="empty">
          <Icon name="image" :size="48" />
          <span>No image-map</span>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.stage {
  display: flex;
  align-items: stretch;
  flex-shrink: 0;
}

/* Shares are flex-grow weights over the width left after the gutters. */
.tile {
  flex: 1 1 0;
  min-width: 0;
  padding: 40px 16px 16px;
  box-sizing: border-box;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  overflow: hidden;
}

/* Sized to the caption line so the buttons do not push the text down. */
.move {
  width: 16px;
  height: 16px;
  line-height: 0;
  padding: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}

.move:hover,
.move.on {
  color: var(--accent);
}

/* --- background colour popover (same controls as the editor's shape panel) --- */

.colour-pop {
  position: absolute;
  top: 32px;
  left: 12px;
  z-index: 2;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid var(--border-dim);
  border-radius: var(--radius-panel);
  background: color-mix(in srgb, var(--bg-surface) 92%, transparent);
}

.swatches {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.swatch {
  width: 18px;
  height: 18px;
  border-radius: 3px;
  border: 1px solid var(--border);
  padding: 0;
  cursor: pointer;
}

.swatch.on {
  outline: 2px solid var(--text);
  outline-offset: 1px;
}

.swatch.big {
  width: 24px;
  height: 24px;
  flex-shrink: 0;
  cursor: default;
}

.line {
  display: flex;
  align-items: center;
  gap: 8px;
}

.hex {
  width: 74px;
  height: 24px;
  padding: 0 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 12px;
  outline: none;
}

.val {
  width: 40px;
  text-align: right;
  color: var(--text-2);
  font-size: 12px;
}

.range {
  flex: 0 0 90px;
  height: 4px;
  appearance: none;
  -webkit-appearance: none;
  background: var(--bg-surface-3);
  border-radius: 2px;
  outline: none;
  cursor: pointer;
}

.range::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: var(--accent);
  border: none;
}

.caption {
  position: absolute;
  top: 10px;
  left: 16px;
  height: 16px;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-2);
}

.name {
  font-weight: 600;
  color: var(--text-2);
}

.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: var(--text-3);
}

</style>
