<script setup lang="ts">
import { computed, ref } from "vue";
import Icon from "./Icon.vue";
import ColumnHead from "./ColumnHead.vue";
import { collator, sortRows, useTableColumns, type ColumnSpec } from "../tableColumns";
import type { ResolvedBinding } from "../types";
import { KIND_RANK } from "../devices";
import { persistedRef } from "../persist";

const props = defineProps<{
  bindings: ResolvedBinding[];
  currentToken: string | null;
  // The current input is still held (or pulsing): its rows stay lit,
  // afterwards they fade.
  liveOn: boolean;
  tokenLabel: (token: string) => string;
  categoryLabel: (actionmap: string) => string;
  // SC's device name for a binding: js1, js2, kb1, gp1.
  deviceLabel: (b: ResolvedBinding) => string;
  isClash: (token: string) => boolean;
  isMissing: (b: ResolvedBinding) => boolean;
  // The binding just clicked, lit like a short press for a moment.
  isFlashed: (b: ResolvedBinding) => boolean;
}>();
const emit = defineEmits<{ flash: [b: ResolvedBinding] }>();

// Toggled device chips; none toggled = every device. Remembered, like the
// grouping switch.
const deviceFilter = persistedRef<string[]>("bindsight.monitor.devices", []);

function toggleDevice(label: string) {
  deviceFilter.value = deviceFilter.value.includes(label)
    ? deviceFilter.value.filter((x) => x !== label)
    : [...deviceFilter.value, label];
}
const search = ref("");

// Flat rows, or one collapsible bucket per input (device + input, the
// actions inside) like the Bindings List's categories.
const grouped = persistedRef<boolean>("bindsight.monitor.grouped", false);

interface Bucket {
  token: string;
  device: string;
  input: string;
  rows: ResolvedBinding[];
}

// Buckets in the order their first row sorts.
const buckets = computed<Bucket[]>(() => {
  const m = new Map<string, Bucket>();
  for (const b of filteredBindings.value) {
    const hit = m.get(b.token);
    if (hit) hit.rows.push(b);
    else m.set(b.token, { token: b.token, device: props.deviceLabel(b), input: inputText(b.token), rows: [b] });
  }
  return [...m.values()];
});

// Expanded buckets; everything starts collapsed. A search forces them open.
const expanded = ref(new Set<string>());

function toggleBucket(token: string) {
  const next = new Set(expanded.value);
  if (next.has(token)) next.delete(token);
  else next.add(token);
  expanded.value = next;
}

function isOpen(g: Bucket): boolean {
  return search.value.trim() !== "" || expanded.value.has(g.token);
}

const allExpanded = computed(() => buckets.value.length > 0 && buckets.value.every((g) => expanded.value.has(g.token)));

function expandAll() {
  expanded.value = new Set(buckets.value.map((g) => g.token));
}

function collapseAll() {
  expanded.value = new Set();
}

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

// The INPUT column carries the device too (`js1 · Button 5`).
const COLUMNS: ColumnSpec[] = [
  { key: "input", label: "INPUT", width: 250, icon: "bolt" },
  { key: "action", label: "ACTION", width: 320, icon: "target" },
  { key: "category", label: "CATEGORY", width: null, icon: "list" },
];
const cols = useTableColumns("bindsight.columns.monitor", COLUMNS, { key: "action", dir: "asc" });

function cellValue(b: ResolvedBinding, key: string): string | number {
  switch (key) {
    // Device order first (like the tiles), then the input.
    case "input":
      return `${String(deviceRank(b)).padStart(4, "0")} ${inputText(b.token)}`;
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
      <div class="divider" />
      <button
        type="button"
        class="btn small square"
        :class="grouped ? 'primary' : 'outline'"
        title="Group by input"
        @click="grouped = !grouped"
      >
        <Icon name="group" :size="14" />
      </button>
      <button type="button" class="btn outline small square" title="Expand all" :disabled="!grouped || allExpanded" @click="expandAll">
        <Icon name="unfold" :size="14" />
      </button>
      <button type="button" class="btn outline small square" title="Collapse all" :disabled="!grouped || !expanded.size" @click="collapseAll">
        <Icon name="fold" :size="14" />
      </button>
      <span class="head-hint">
        <Icon name="bolt" :size="13" />
        Press an input to highlight its bindings
      </span>
      <div class="spacer" />
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
      <div class="divider" />
      <div class="search">
        <Icon name="search" :size="14" />
        <input v-model="search" placeholder="Find…" />
        <button v-if="search" type="button" class="clear" title="Clear" @click="search = ''">
          <Icon name="close" :size="12" />
        </button>
      </div>
    </div>

    <div class="table" :class="{ grouped }">
        <ColumnHead
          :columns="COLUMNS"
          :sort="cols.sort.value"
          @sort="cols.toggleSort"
          @resize="cols.startResize"
          @reset="cols.resetWidth"
        />
        <template v-if="grouped">
          <template v-for="g in buckets" :key="g.token">
            <div class="row group-row" :class="{ live: liveOn && g.token === currentToken }" @click="toggleBucket(g.token)">
              <span class="input-cell">
                <Icon :name="isOpen(g) ? 'chevron-down' : 'chevron-right'" :size="14" class="chevron" />
                <span class="mono cell-device" :class="{ clash: isClash(g.token) }" :title="isClash(g.token) ? 'misassigned' : undefined">
                  {{ g.device }}
                </span>
                <span class="cell-input" :class="{ mono: !labelOf(g.token) }" :title="g.token"
                  ><Icon name="bolt" :size="12" class="live-mark" />{{ g.input }}</span
                >
              </span>
              <span />
              <span />
            </div>
            <template v-if="isOpen(g)">
              <div
                v-for="(b, i) in g.rows"
                :key="i"
                class="row binding-row"
                :class="{ live: (liveOn && b.token === currentToken) || isFlashed(b), missing: isMissing(b) }"
                :title="rowTitle(b)"
                @click="emit('flash', b)"
              >
                <span />
                <span class="cell-action">{{ b.label ?? b.action }}</span>
                <span class="cell-category">{{ categoryLabel(b.actionmap) }}</span>
              </div>
            </template>
          </template>
        </template>
        <template v-else>
          <div
            v-for="(b, i) in filteredBindings"
            :key="i"
            class="row binding-row"
            :class="{ live: (liveOn && b.token === currentToken) || isFlashed(b), missing: isMissing(b) }"
            :title="rowTitle(b)"
            @click="emit('flash', b)"
          >
            <span class="input-cell">
              <span class="mono cell-device" :class="{ clash: isClash(b.token) }" :title="deviceTitle(b)">
                {{ deviceLabel(b) }}
              </span>
              <span class="cell-input" :class="{ mono: !labelOf(b.token) }" :title="b.token"
                ><Icon name="bolt" :size="12" class="live-mark" />{{ inputText(b.token) }}</span
              >
            </span>
            <span class="cell-action">{{ b.label ?? b.action }}</span>
            <span class="cell-category">{{ categoryLabel(b.actionmap) }}</span>
          </div>
        </template>
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

.btn {
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-control);
  font-family: inherit;
  cursor: pointer;
}

.btn.small {
  height: var(--h-chip-sm);
}

/* Icon-only: as wide as it is high. */
.btn.square {
  width: var(--h-chip-sm);
  padding: 0;
}

.btn.outline {
  background: transparent;
  color: var(--text);
  border: 1px solid rgba(255, 255, 255, 0.5);
}

.btn.primary {
  background: var(--accent);
  color: var(--accent-text);
  border: none;
}

.btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.head-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: 8px;
  font-size: 12px;
  color: var(--text-3);
  white-space: nowrap;
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

/* Clears the box; only there while it has text. */
.search .clear {
  width: 20px;
  height: 20px;
  margin-right: -4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  flex-shrink: 0;
}

.search .clear:hover {
  color: var(--text);
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
  border-left: 3px solid transparent;
  transition:
    background-color 600ms ease-out,
    border-left-color 600ms ease-out;
}

.binding-row > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* The current input while held: a live-coloured left edge and a flash on
   the row, device and input in live colour, a bolt in front of the input
   (always in the layout, invisible until lit). Lights up at once, fades
   out afterwards. */
.binding-row.live {
  border-left-color: var(--live);
  background: color-mix(in srgb, var(--live) 8%, transparent);
  transition: none;
}

.cell-device,
.cell-input {
  transition: color 600ms ease-out;
}

.binding-row.live .cell-device,
.binding-row.live .cell-input {
  color: var(--live);
  transition: none;
}

.live-mark {
  vertical-align: -2px;
  margin-right: 6px;
  opacity: 0;
  transition: opacity 600ms ease-out;
}

.binding-row.live .live-mark {
  opacity: 1;
  transition: none;
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

/* --- buckets (grouped view) --- */

/* A grid row like the others, so the INPUT header's grip sizes it too. A
   bucket is set off from the one before by a line above its head; its
   children hang below it without a line. */
.group-row {
  font-size: 13px;
  cursor: pointer;
  user-select: none;
  border-left: 3px solid transparent;
  border-top: 1px solid var(--border-dim);
  transition:
    background-color 600ms ease-out,
    border-left-color 600ms ease-out;
}

/* Children: the empty INPUT cell already indents them; a thin tree line
   under the chevron ties them to their head. */
.grouped .binding-row {
  position: relative;
}

.grouped .binding-row::before {
  content: "";
  position: absolute;
  left: 22px;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--border-dim);
}

/* Device and input side by side in the one INPUT cell. */
.input-cell {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.input-cell .chevron,
.input-cell .cell-device {
  flex-shrink: 0;
}

.input-cell .cell-input {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

.group-row:hover,
.binding-row:hover {
  background: var(--bg-surface-2);
}

.group-row.live {
  border-left-color: var(--live);
  background: color-mix(in srgb, var(--live) 8%, transparent);
  transition: none;
}

.group-row.live .cell-device,
.group-row.live .cell-input {
  color: var(--live);
  transition: none;
}

.group-row.live .live-mark {
  opacity: 1;
  transition: none;
}

</style>
