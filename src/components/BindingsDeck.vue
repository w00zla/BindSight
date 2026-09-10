<script setup lang="ts">
import { computed, ref } from "vue";
import Icon from "./Icon.vue";
import ColumnHead from "./ColumnHead.vue";
import { collator, sortRows, useTableColumns, type ColumnSpec } from "../tableColumns";
import type { ResolvedBinding } from "../types";
import { KIND_RANK } from "../devices";

const props = defineProps<{
  bindings: ResolvedBinding[];
  currentToken: string | null;
  tokenLabel: (token: string) => string;
  categoryLabel: (actionmap: string) => string;
  // SC's device name for a binding: js1, js2, kb1, gp1.
  deviceLabel: (b: ResolvedBinding) => string;
  isClash: (token: string) => boolean;
  isMissing: (b: ResolvedBinding) => boolean;
  isPinned: (b: ResolvedBinding) => boolean;
}>();
const emit = defineEmits<{ pin: [b: ResolvedBinding] }>();

// Toggled device chips; none toggled = every device.
const deviceFilter = ref<string[]>([]);

function toggleDevice(label: string) {
  deviceFilter.value = deviceFilter.value.includes(label)
    ? deviceFilter.value.filter((x) => x !== label)
    : [...deviceFilter.value, label];
}
const search = ref("");

// Same order as the device tiles, joysticks by instance.
function deviceRank(b: ResolvedBinding): number {
  return KIND_RANK[b.device_kind] * 100 + b.instance;
}

// Distinct devices present in the bindings, in SC's order, with counts.
const deviceCounts = computed(() => {
  const counts = new Map<string, { rank: number; count: number }>();
  for (const b of props.bindings) {
    const label = props.deviceLabel(b);
    const hit = counts.get(label);
    if (hit) hit.count += 1;
    else counts.set(label, { rank: deviceRank(b), count: 1 });
  }
  return [...counts.entries()].sort((a, b) => a[1].rank - b[1].rank);
});

function matchesSearch(b: ResolvedBinding, q: string): boolean {
  if (!q) return true;
  // Search covers the INPUT and ACTION columns only.
  const hay = [inputText(b.token), b.label ?? b.action].join(" ").toLowerCase();
  return hay.includes(q);
}

const COLUMNS: ColumnSpec[] = [
  { key: "device", label: "DEVICE", width: 70 },
  { key: "input", label: "INPUT", width: 180 },
  { key: "action", label: "ACTION", width: 320 },
  { key: "category", label: "CATEGORY", width: null },
];
const cols = useTableColumns("bindsight.columns.bindings", COLUMNS, { key: "action", dir: "asc" });

function cellValue(b: ResolvedBinding, key: string): string | number {
  switch (key) {
    case "device":
      return deviceRank(b);
    case "input":
      return inputText(b.token);
    case "action":
      return b.label ?? b.action;
    default:
      return props.categoryLabel(b.actionmap);
  }
}

const filteredBindings = computed(() => {
  const q = search.value.trim().toLowerCase();
  const rows = props.bindings.filter((b) => {
    if (deviceFilter.value.length && !deviceFilter.value.includes(props.deviceLabel(b))) return false;
    return matchesSearch(b, q);
  });
  return sortRows(rows, cols.sort.value, cellValue, (a, b) => collator.compare(a.token, b.token));
});

// Localized input label; empty when SC has none (tokenLabel echoes the token then).
function labelOf(token: string): string {
  const l = props.tokenLabel(token);
  return l === token ? "" : l;
}

// Input token without its device prefix, e.g. "js1_button5" -> "button5".
function inputPart(token: string): string {
  return token.replace(/^(js\d+|kb1|gp1)_/, "");
}

// The INPUT cell: SC's label, else the bare token.
function inputText(token: string): string {
  return labelOf(token) || inputPart(token);
}

function rowTitle(b: ResolvedBinding): string | undefined {
  return props.isMissing(b) ? `No area for ${props.tokenLabel(b.token)} on the ${b.device ?? "device"} image` : undefined;
}

function deviceTitle(b: ResolvedBinding): string | undefined {
  return props.isClash(b.token) ? "misassigned" : undefined;
}
</script>

<template>
  <div class="deck" :style="{ '--cols': cols.template.value, '--cols-min': `${cols.minWidth.value}px` }">
    <div class="header">
      <div class="panel-title">Bindings</div>
      <span class="total">{{ bindings.length }}</span>
      <div class="divider" />
      <div class="chips">
          <button
            v-for="[label, d] in deviceCounts"
            :key="label"
            type="button"
            class="chip mono"
            :class="{ active: deviceFilter.includes(label) }"
            @click="toggleDevice(label)"
          >
            {{ label }} <span class="count">{{ d.count }}</span>
          </button>
        </div>
      <div class="spacer" />
      <div class="search">
        <Icon name="search" :size="14" />
        <input v-model="search" placeholder="Find input or action…" />
      </div>
    </div>

    <div class="table">
        <ColumnHead
          :columns="COLUMNS"
          :sort="cols.sort.value"
          @sort="cols.toggleSort"
          @resize="cols.startResize"
          @reset="cols.resetWidth"
        />
        <div
          v-for="(b, i) in filteredBindings"
          :key="i"
          class="row binding-row"
          :class="{ current: b.token === currentToken, pinned: isPinned(b), missing: isMissing(b) }"
          :title="rowTitle(b)"
          @click="emit('pin', b)"
        >
          <span class="mono cell-device" :class="{ clash: isClash(b.token) }" :title="deviceTitle(b)">
            {{ deviceLabel(b) }}
          </span>
          <span class="cell-input" :class="{ mono: !labelOf(b.token) }" :title="b.token">{{ inputText(b.token) }}</span>
          <span class="cell-action">{{ b.label ?? b.action }}</span>
          <span class="cell-category">{{ categoryLabel(b.actionmap) }}</span>
        </div>
      </div>
  </div>
</template>

<style scoped>
.deck {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  flex: 1;
  min-height: 0;
}

.header {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border-dim);
}

.divider {
  width: 1px;
  height: 20px;
  background: var(--border-dim);
  margin: 0 8px;
}

.chips {
  display: flex;
  gap: 4px;
}

.chip {
  height: var(--h-chip-sm);
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  border-radius: var(--radius-control);
  border: 1px solid rgba(173, 211, 235, 0.35);
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-size: 13px;
  cursor: pointer;
}

.chip .count {
  color: var(--text-2);
}

.chip.active {
  background: var(--accent);
  color: var(--accent-text);
  border-color: transparent;
}

.chip.active .count {
  color: inherit;
  opacity: 0.7;
}

.spacer {
  flex: 1;
}

.search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-chip);
  width: 260px;
  padding: 0 12px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text-2);
}

.search input {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-size: 13px;
  outline: none;
}

.search input::placeholder {
  color: rgba(173, 211, 235, 0.6);
}

.row {
  display: grid;
  grid-template-columns: var(--cols);
  gap: 12px;
  padding: 8px 16px;
  align-items: center;
  /* Content-box: padding comes on top. Rows widen, the table scrolls. */
  min-width: var(--cols-min);
}

.table {
  overflow: auto;
  min-height: 0;
  flex: 1;
}

.cols-head {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--bg-surface);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  color: var(--text-2);
  border-bottom: 1px solid var(--border-dim);
}

.binding-row {
  font-size: 13px;
  cursor: pointer;
  border-left: 2px solid transparent;
}

.binding-row > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.binding-row.current {
  background: rgba(78, 224, 255, 0.08);
}

.binding-row.current .cell-device,
.binding-row.current .cell-input {
  color: var(--live);
}

.binding-row.pinned {
  border-left-color: var(--live);
}

.binding-row.missing {
  opacity: 0.4;
}

.cell-device {
  color: var(--text-2);
}

.cell-device.clash {
  color: var(--warn);
}

.cell-category {
  color: var(--text-2);
}

.total {
  font-size: 12px;
  color: var(--text-2);
}
</style>
