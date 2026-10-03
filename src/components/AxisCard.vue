<script setup lang="ts">
// One physical axis (a joystick axis, a pad stick): its deadzone and
// saturation, a live bar with both laid over the input when input reaches
// the app, and the actions bound to it. Fixed height: the chip line holds at
// most two chips plus "+N", so the card never resizes.
import { computed } from "vue";
import { axisOutput } from "../axisStream";

const props = defineProps<{
  title: string;
  deadzone: number;
  // null: the axis has no saturation (a pad stick).
  saturation: number | null;
  deadzoneLabel: string;
  saturationLabel: string;
  deadzoneChanged: boolean;
  saturationChanged: boolean;
  // A live bar and readout at all (input reaches the app from the device).
  live: boolean;
  // The input: -1..1 for a joystick axis, 0..1 for a pad stick; null until
  // the axis moved.
  raw: number | null;
  // A pad stick: the bar runs from the left edge, no saturation.
  pad: boolean;
  // The bound actions; null hides the line.
  bound: string[] | null;
  disabled?: boolean;
}>();
const emit = defineEmits<{ deadzone: [value: number]; saturation: [value: number] }>();

const raw = computed(() => props.raw ?? 0);
const sat = computed(() => props.saturation ?? 1);

// The output after deadzone and saturation, the Monitor's rule: 0 inside
// the deadzone, full travel at or beyond saturation, the raw value in
// between (the game's exact formula is unknown).
const out = computed(() => axisOutput(props.pad ? "thumb" : "joystick", raw.value, props.deadzone, props.saturation));

const readout = computed(() => `${out.value >= 0 ? "+" : ""}${out.value.toFixed(2)}`);

const dzStyle = computed(() =>
  props.pad ? { left: "0", width: `${props.deadzone * 100}%` } : { left: `${50 - props.deadzone * 50}%`, width: `${props.deadzone * 100}%` },
);
const satWidth = computed(() => `${(1 - sat.value) * 50}%`);
const rawLeft = computed(() => (props.pad ? `${raw.value * 100}%` : `${50 + raw.value * 50}%`));
const outStyle = computed(() =>
  props.pad
    ? { left: "0", width: `${Math.abs(out.value) * 100}%` }
    : { left: out.value >= 0 ? "50%" : `${50 + out.value * 50}%`, width: `${Math.abs(out.value) * 50}%` },
);

const chips = computed(() => props.bound?.slice(0, 2) ?? []);
const more = computed(() => Math.max(0, (props.bound?.length ?? 0) - 2));

function num(e: Event): number {
  return Math.round(Number((e.target as HTMLInputElement).value) * 100) / 100;
}
</script>

<template>
  <div class="axis" :class="{ changed: deadzoneChanged || saturationChanged }">
    <div class="ah">
      <span class="title">{{ title }}</span>
      <span v-if="live" class="readout mono">{{ readout }}</span>
    </div>
    <div v-if="live" class="livebar" :class="{ pad }">
      <template v-if="!pad">
        <span class="sat" :style="{ left: '0', width: satWidth }" />
        <span class="sat" :style="{ right: '0', width: satWidth }" />
      </template>
      <span class="dz" :style="dzStyle" />
      <span class="ctr" />
      <span class="out" :style="outStyle" />
      <span class="raw" :style="{ left: rawLeft }" />
    </div>
    <div class="kv" :class="{ changed: deadzoneChanged }" :title="deadzoneLabel">
      <span>Deadzone</span>
      <input
        class="range"
        type="range"
        min="0"
        max="1"
        step="0.01"
        :value="deadzone"
        :style="{ '--p': `${deadzone * 100}%` }"
        :aria-label="deadzoneLabel"
        :disabled="disabled"
        @input="emit('deadzone', num($event))"
      />
      <span class="v mono">{{ deadzone.toFixed(2) }}</span>
    </div>
    <div v-if="saturation !== null" class="kv" :class="{ changed: saturationChanged }" :title="saturationLabel">
      <span>Saturation</span>
      <input
        class="range"
        type="range"
        min="0"
        max="1"
        step="0.01"
        :value="saturation"
        :style="{ '--p': `${saturation * 100}%` }"
        :aria-label="saturationLabel"
        :disabled="disabled"
        @input="emit('saturation', num($event))"
      />
      <span class="v mono">{{ saturation.toFixed(2) }}</span>
    </div>
    <div v-if="bound" class="bound-wrap" :tabindex="bound.length ? 0 : -1">
      <div class="bound">
        <template v-if="bound.length">
          <span v-for="b in chips" :key="b" class="chip">{{ b }}</span>
          <span v-if="more" class="chip more">+{{ more }}</span>
        </template>
        <span v-else class="none">not bound</span>
      </div>
      <div v-if="bound.length" class="pop" role="tooltip">
        <span v-for="b in bound" :key="b" class="chip full">{{ b }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.axis {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
  padding: 10px 12px 12px;
  border: 1px solid var(--border-dim);
  border-radius: 6px;
  background: var(--bg-base);
}

.axis.changed {
  border-color: color-mix(in srgb, var(--warn) 60%, transparent);
}

.ah {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.title {
  font-weight: 600;
  font-size: 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.readout {
  margin-left: auto;
  flex-shrink: 0;
  font-size: 12px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

.livebar {
  position: relative;
  height: 26px;
  border-radius: 3px;
  background: var(--bg-surface-2);
  overflow: hidden;
}

.livebar > span {
  position: absolute;
}

.dz {
  top: 0;
  bottom: 0;
  background: repeating-linear-gradient(135deg, color-mix(in srgb, var(--warn) 35%, transparent) 0 4px, transparent 4px 8px);
  border-left: 1px solid var(--warn);
  border-right: 1px solid var(--warn);
  box-sizing: border-box;
}

.sat {
  top: 0;
  bottom: 0;
  background: color-mix(in srgb, var(--err) 25%, transparent);
}

.ctr {
  left: 50%;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--text-3);
}

.livebar.pad .ctr {
  left: 0;
}

.raw {
  top: 3px;
  bottom: 3px;
  width: 3px;
  margin-left: -1px;
  border-radius: 1px;
  background: var(--text-3);
}

.out {
  top: 7px;
  bottom: 7px;
  border-radius: 2px;
  background: var(--live);
}

.kv {
  display: grid;
  grid-template-columns: 70px minmax(0, 1fr) 44px;
  gap: 10px;
  align-items: center;
  font-size: 12px;
  color: var(--text-2);
}

.kv .range {
  height: 22px;
  --track: 7px;
  --thumb: 17px;
}

.v {
  font-size: 15px;
  color: var(--text);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.kv.changed .v {
  color: var(--warn);
}

/* The overlay opens upwards from the chip line, inside the card. */
.bound-wrap {
  position: relative;
  outline: none;
}

.pop {
  position: absolute;
  left: 0;
  bottom: calc(100% - 4px);
  z-index: 20;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  max-width: 100%;
  max-height: 132px;
  overflow-y: auto;
  padding: 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface);
  box-shadow: 0 8px 28px rgb(0 0 0 / 45%);
  opacity: 0;
  visibility: hidden;
  transition: opacity 0.12s, visibility 0.12s;
}

.bound-wrap:hover .pop,
.bound-wrap:focus-visible .pop {
  opacity: 1;
  visibility: visible;
}

.chip.full {
  flex: none;
  max-width: 100%;
}

/* One line of fixed height under a light separator. */
.bound {
  display: flex;
  flex-wrap: nowrap;
  gap: 4px;
  height: 29px;
  box-sizing: border-box;
  padding-top: 8px;
  border-top: 1px solid var(--border-dim);
  overflow: hidden;
  font-size: 12px;
}

.chip {
  flex: 0 1 auto;
  min-width: 0;
  padding: 1px 6px;
  border-radius: 3px;
  background: var(--bg-surface-2);
  color: var(--text-2);
  line-height: 18px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip.more {
  flex: none;
  color: var(--text);
}

.none {
  color: var(--text-3);
  line-height: 20px;
}
</style>
