<script setup lang="ts">
import { computed, ref, watch } from "vue";
import DeviceImage from "./DeviceImage.vue";
import Icon from "./Icon.vue";
import Splitter from "./Splitter.vue";
import type { DeviceInfo, ImageMapView } from "../types";
import type { HighlightClass } from "../imagemap";

const props = defineProps<{
  views: ImageMapView[];
  // Connected devices without an image-map: shown as placeholder tiles.
  placeholders: DeviceInfo[];
  // Stage height in px (the parent owns the vertical splitter).
  height: number;
  imgSrc: (id: string, file: string) => string;
  activeFor: (sdlGuid: string) => Map<string, HighlightClass>;
}>();
const emit = defineEmits<{ choose: [hardwareId: string | null, id: string] }>();

type Tile = { device: DeviceInfo; view: ImageMapView | null };

// Tile order: user-swapped device indices first (remembered), the rest in
// SDL order.
const ORDER_KEY = "bindsight.stage.order";
const order = ref<number[]>([]);
try {
  const parsed = JSON.parse(localStorage.getItem(ORDER_KEY) ?? "[]") as unknown;
  if (Array.isArray(parsed) && parsed.every((x) => typeof x === "number")) order.value = parsed;
} catch {
  /* SDL order */
}

const tiles = computed<Tile[]>(() => {
  const all: Tile[] = [
    ...props.views.map((v) => ({ device: v.device, view: v })),
    ...props.placeholders.map((d) => ({ device: d, view: null })),
  ];
  const rank = (t: Tile) => {
    const i = order.value.indexOf(t.device.index);
    return i === -1 ? order.value.length + t.device.index : i;
  };
  return all.sort((a, b) => rank(a) - rank(b));
});

function swap(i: number) {
  const ids = tiles.value.map((t) => t.device.index);
  [ids[i], ids[i + 1]] = [ids[i + 1], ids[i]];
  order.value = ids;
  try {
    localStorage.setItem(ORDER_KEY, JSON.stringify(ids));
  } catch {
    /* ignore */
  }
}

// Width share per tile; draggable gutters between neighbours. Remembered per
// tile count so a HOTAS/HOSAS split survives a restart.
const MIN_SHARE = 0.15;
const shares = ref<number[]>([]);
const storeKey = (n: number) => `bindsight.stage.shares.${n}`;

function equalShares(n: number): number[] {
  return Array.from({ length: n }, () => 1 / n);
}

function loadShares(n: number): number[] {
  try {
    const raw = localStorage.getItem(storeKey(n));
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    if (Array.isArray(parsed) && parsed.length === n && parsed.every((x) => typeof x === "number" && x > 0)) {
      return parsed;
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
let dragStart: { left: number; pair: number } | null = null;

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
  if (!dragStart) dragStart = { left: shares.value[i], pair: shares.value[i] + shares.value[i + 1] };
  const { left: left0, pair } = dragStart;
  const left = Math.min(Math.max(left0 + deltaPx / width, MIN_SHARE), pair - MIN_SHARE);
  shares.value[i] = left;
  shares.value[i + 1] = pair - left;
}

function onEnd() {
  dragStart = null;
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
      <div class="tile" :class="{ placeholder: !t.view }" :style="{ flexGrow: shares[i] ?? 1 }">
        <div class="caption">
          <Icon name="image" :size="13" />
          <template v-if="t.view">
            <select
              v-if="t.view.options.length > 1"
              class="picker"
              :value="t.view.map.id"
              @change="emit('choose', t.device.hardware_id, ($event.target as HTMLSelectElement).value)"
            >
              <option v-for="s in t.view.options" :key="s.id" :value="s.id">{{ s.name }}</option>
            </select>
            <span v-else class="name">{{ t.view.map.name }}</span>
          </template>
          <span v-else class="name">{{ t.device.sc_name ?? t.device.sdl_name }}</span>
          <button v-if="i > 0" type="button" class="move" title="Move left" @click="swap(i - 1)">
            <Icon name="arrow-left" :size="14" />
          </button>
          <button v-if="i < tiles.length - 1" type="button" class="move" title="Move right" @click="swap(i)">
            <Icon name="arrow-right" :size="14" />
          </button>
        </div>
        <DeviceImage
          v-if="t.view && imgSrc(t.view.map.id, t.view.map.image.file)"
          :map="t.view.map"
          :src="imgSrc(t.view.map.id, t.view.map.image.file)"
          :active="activeFor(t.device.sdl_guid)"
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

.move:hover {
  color: var(--accent);
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
  color: var(--text);
}

.placeholder .name {
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

.picker {
  height: var(--h-chip-sm);
  padding: 0 8px;
  border-radius: var(--radius-control);
  border: none;
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 12px;
  font-family: inherit;
}
</style>
