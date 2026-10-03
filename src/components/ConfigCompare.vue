<script setup lang="ts">
// Compare of the settings: every setting that differs between Current and
// the picked profile / backup, seen from Current. The chips filter by kind
// (none toggled = all), like the Bindings Compare.
import { computed, ref } from "vue";
import Icon from "./Icon.vue";
import ColumnHead from "./ColumnHead.vue";
import CurveThumb from "./CurveThumb.vue";
import { collator, sortRows, useTableColumns, type ColumnSpec } from "../tableColumns";
import type { CompareCell, CompareRow, CompareStatus, Curve } from "../configModel";

const props = defineProps<{ rows: CompareRow[]; sourceName: string }>();

const KINDS: { kind: CompareStatus; label: string }[] = [
  { kind: "added", label: "Added" },
  { kind: "removed", label: "Removed" },
  { kind: "changed", label: "Changed" },
];

// Not remembered across restarts (see persist.ts).
const filter = ref<CompareStatus[]>([]);

function toggle(kind: CompareStatus) {
  filter.value = filter.value.includes(kind) ? filter.value.filter((k) => k !== kind) : [...filter.value, kind];
}

const counts = computed(() => {
  const c: Record<CompareStatus, number> = { added: 0, removed: 0, changed: 0 };
  for (const r of props.rows) c[r.status] += 1;
  return c;
});

// A toggle on a kind this compare has no rows for must not filter.
const active = computed(() => filter.value.filter((k) => counts.value[k] > 0));

const COLUMNS: ColumnSpec[] = [
  { key: "device", label: "DEVICE", width: 80, icon: "devices" },
  { key: "setting", label: "SETTING", width: 330, icon: "list" },
  { key: "type", label: "TYPE", width: 110 },
  { key: "a", label: "CURRENT", width: 160, icon: "file" },
  { key: "b", label: "", width: null, icon: "file" },
];
const cols = useTableColumns("bindsight.columns.configcompare", COLUMNS, { key: "device", dir: "asc" });
const columns = computed<ColumnSpec[]>(() => COLUMNS.map((c) => (c.key === "b" ? { ...c, label: props.sourceName.toUpperCase() } : c)));

function cellText(c: CompareCell): string {
  if (!c) return "";
  return "text" in c ? c.text : "exp" in c.curve ? c.curve.exp.toFixed(2) : "";
}

function curveOf(c: CompareCell): Curve | null {
  return c && "curve" in c ? c.curve : null;
}

function valueOf(r: CompareRow, key: string): string | number {
  switch (key) {
    case "device":
      return r.rank;
    case "setting":
      return r.label;
    case "type":
      return r.type;
    case "a":
      return cellText(r.a);
    default:
      return cellText(r.b);
  }
}

const shown = computed(() =>
  sortRows(
    props.rows.filter((r) => !active.value.length || active.value.includes(r.status)),
    cols.sort.value,
    valueOf,
    (a, b) => a.rank - b.rank || collator.compare(a.label, b.label),
  ),
);
</script>

<template>
  <section class="panel" :style="{ '--cols': cols.template.value, '--cols-min': `${cols.minWidth.value}px` }">
    <div class="head">
      <Icon name="compare" :size="16" />
      <span class="head-title">Compare</span>
      <div class="spacer" />
      <div class="chips">
        <button
          v-for="c in KINDS"
          :key="c.kind"
          type="button"
          class="chip"
          :class="[c.kind, { active: active.includes(c.kind) }]"
          :disabled="!counts[c.kind]"
          @click="toggle(c.kind)"
        >
          {{ c.label }} <span class="count">{{ counts[c.kind] }}</span>
        </button>
      </div>
    </div>
    <div class="table">
      <ColumnHead
        :columns="columns"
        :sort="cols.sort.value"
        @sort="cols.toggleSort"
        @resize="cols.startResize"
        @reset="cols.resetWidth"
      />
      <div v-for="r in shown" :key="r.key" class="row diff-row" :class="r.status">
        <span><span class="slot mono">{{ r.device }}</span></span>
        <span class="setting">
          <span class="l">{{ r.label }}</span>
          <span v-if="r.path" class="p">{{ r.path }}</span>
        </span>
        <span class="type">{{ r.type }}</span>
        <span v-for="side in (['a', 'b'] as const)" :key="side" class="cv mono">
          <template v-if="r[side]">
            <CurveThumb v-if="curveOf(r[side])" :curve="curveOf(r[side])!" :size="22" />
            {{ cellText(r[side]) }}
          </template>
          <span v-else class="none">—</span>
        </span>
      </div>
      <div v-if="!shown.length" class="empty-line">{{ rows.length ? "No matches" : "No differences" }}</div>
    </div>
  </section>
</template>

<style scoped>
.panel {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  flex-shrink: 0;
}

.head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border-dim);
}

.head-title {
  font-weight: 600;
  font-size: 14px;
}

.spacer {
  flex: 1;
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
  border: 1px solid var(--border);
  background: transparent;
  font-family: inherit;
  font-weight: 400;
  font-size: 13px;
  cursor: pointer;
}

.chip .count {
  font-weight: 600;
}

.chip:disabled {
  opacity: 0.4;
  cursor: default;
}

/* The kind chips stay quiet until toggled on, then take their kind's colour
   (as in the Bindings Compare). */
.chip.added {
  color: color-mix(in srgb, var(--ok) 70%, var(--text-2));
  border-color: color-mix(in srgb, var(--ok) 30%, transparent);
}

.chip.removed {
  color: color-mix(in srgb, var(--err) 70%, var(--text-2));
  border-color: color-mix(in srgb, var(--err) 30%, transparent);
}

.chip.changed {
  color: color-mix(in srgb, var(--warn) 70%, var(--text-2));
  border-color: color-mix(in srgb, var(--warn) 30%, transparent);
}

.chip.added.active {
  color: color-mix(in srgb, var(--ok) 85%, var(--text-2));
  border-color: color-mix(in srgb, var(--ok) 60%, transparent);
  background: color-mix(in srgb, var(--ok) 12%, transparent);
}

.chip.removed.active {
  color: color-mix(in srgb, var(--err) 85%, var(--text-2));
  border-color: color-mix(in srgb, var(--err) 60%, transparent);
  background: color-mix(in srgb, var(--err) 12%, transparent);
}

.chip.changed.active {
  color: color-mix(in srgb, var(--warn) 85%, var(--text-2));
  border-color: color-mix(in srgb, var(--warn) 60%, transparent);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
}

.table {
  overflow-x: auto;
  font-size: 13px;
}

.row {
  display: grid;
  grid-template-columns: var(--cols);
  gap: 12px;
  padding: 6px 16px;
  align-items: center;
  min-width: var(--cols-min);
}

.cols-head {
  padding: 8px 16px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-2);
  border-bottom: 1px solid var(--border-dim);
}

.diff-row {
  position: relative;
  min-height: 32px;
}

.diff-row > span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.diff-row::before {
  content: "";
  position: absolute;
  left: 4px;
  top: 8px;
  bottom: 8px;
  width: 3px;
  border-radius: 2px;
}

.diff-row.added::before {
  background: var(--ok);
}

.diff-row.removed::before {
  background: var(--err);
}

.diff-row.changed::before {
  background: var(--warn);
}

.slot {
  font-size: 12px;
  padding: 1px 6px;
  border-radius: 3px;
  background: var(--bg-surface-2);
  color: var(--text-2);
}

.setting {
  display: flex;
  flex-direction: column;
}

.setting .l,
.setting .p {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.setting .p {
  font-size: 11px;
  color: var(--text-3);
}

.type {
  font-size: 12px;
  color: var(--text-2);
}

.cv {
  display: flex;
  align-items: center;
  gap: 8px;
  font-variant-numeric: tabular-nums;
}

.none {
  color: var(--text-3);
  font-family: var(--font-ui);
}

.empty-line {
  padding: 24px 16px;
  text-align: center;
  color: var(--text-3);
}
</style>
