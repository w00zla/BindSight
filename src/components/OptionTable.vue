<script setup lang="ts">
// The option tree of one device as a table: Setting | Curve | Invert. The
// game shows this one tree twice ("Inversion Settings", "… Sensitivity
// Curves"); here they are two columns, empty where the game has no control.
// A group's curve is inherited by the rows below it (dimmed); a group change
// replaces the curves below it — the amber chip counts them beforehand.
import { computed, ref } from "vue";
import Icon from "./Icon.vue";
import ColumnHead from "./ColumnHead.vue";
import CurveThumb from "./CurveThumb.vue";
import YesNo from "./YesNo.vue";
import { useTableColumns, type ColumnSpec } from "../tableColumns";
import {
  EXP_MAX,
  EXP_MIN,
  EXP_STEP,
  countBelow,
  curveForChildren,
  effectiveCurve,
  effectiveInvert,
  isGroup,
  nodeChanged,
  nodeLabel,
  ownCurve,
  roundExp,
  type Curve,
} from "../configModel";
import type { NodeValue, OptionNode, OptionTree } from "../types";

const props = defineProps<{
  tree: OptionTree;
  // The device's own values with the pending edits, and as saved.
  values: Record<string, NodeValue>;
  saved: Record<string, NodeValue>;
  // Read-only: the device's slot has no settings block in the file, so an
  // edit could never be saved.
  disabled?: boolean;
}>();
const emit = defineEmits<{
  exponent: [node: OptionNode, value: number];
  invert: [node: OptionNode, value: boolean | null];
  resetCurve: [node: OptionNode];
  editCurve: [node: OptionNode];
}>();

const COLUMNS: ColumnSpec[] = [
  { key: "setting", label: "SETTING", width: 340, icon: "list", sortable: false },
  { key: "curve", label: "CURVE", width: 360, icon: "curve", sortable: false },
  { key: "invert", label: "INVERT", width: null, icon: "invert", sortable: false },
];
const cols = useTableColumns("bindsight.columns.config", COLUMNS, { key: "setting", dir: "asc" });

// Collapsed groups (by node name); everything starts open. A search opens
// every group it shows.
const closed = ref(new Set<string>());
const query = ref("");
const q = computed(() => query.value.trim().toLowerCase());

function toggle(n: OptionNode) {
  const s = new Set(closed.value);
  if (!s.delete(n.name)) s.add(n.name);
  closed.value = s;
}

function groupNames(nodes: OptionNode[], out: string[] = []): string[] {
  for (const n of nodes) {
    if (isGroup(n)) {
      out.push(n.name);
      groupNames(n.children, out);
    }
  }
  return out;
}
const groups = computed(() => groupNames(props.tree.nodes));
const allOpen = computed(() => !closed.value.size);
const noneOpen = computed(() => groups.value.every((g) => closed.value.has(g)));

const hit = (n: OptionNode) => nodeLabel(n).toLowerCase().includes(q.value);
const subtreeHit = (n: OptionNode): boolean => hit(n) || n.children.some(subtreeHit);

// The slider being dragged: its value shows before it is let go of.
const drag = ref<{ node: string; value: number } | null>(null);

interface Row {
  node: OptionNode;
  level: number;
  group: boolean;
  open: boolean;
  // The curve column: the effective curve, the own one, whether it is the
  // group's (dimmed), and what a change here would replace below.
  curve: Curve | null;
  own: boolean;
  inherited: boolean;
  wipeCurves: number;
  invert: boolean;
  ownInvert: boolean;
  wipeInverts: number;
  changed: boolean;
  label: { pre: string; mark: string; post: string };
}

function split(label: string): Row["label"] {
  const i = q.value ? label.toLowerCase().indexOf(q.value) : -1;
  if (i < 0) return { pre: label, mark: "", post: "" };
  return { pre: label.slice(0, i), mark: label.slice(i, i + q.value.length), post: label.slice(i + q.value.length) };
}

// The visible rows in tree order. `showAll`: an ancestor matched the search,
// so its whole subtree shows.
const rows = computed<Row[]>(() => {
  const out: Row[] = [];
  const v = props.values;
  const walk = (nodes: OptionNode[], level: number, parent: Curve | null, showAll: boolean) => {
    for (const n of nodes) {
      if (q.value && !showAll && !subtreeHit(n)) continue;
      const group = isGroup(n);
      const open = !!q.value || !closed.value.has(n.name);
      const hasCurve = n.show_curve === 1;
      const own = hasCurve && !!ownCurve(v[n.name]);
      out.push({
        node: n,
        level,
        group,
        open,
        curve: hasCurve ? effectiveCurve(n, v, parent) : null,
        own,
        inherited: hasCurve && !own,
        wipeCurves: group && hasCurve ? countBelow(n, v, "curve") : 0,
        invert: effectiveInvert(n, v),
        ownInvert: v[n.name]?.invert !== null && v[n.name]?.invert !== undefined,
        wipeInverts: group && n.show_invert === 1 ? countBelow(n, v, "invert") : 0,
        changed: nodeChanged(v[n.name], props.saved[n.name]),
        label: split(nodeLabel(n)),
      });
      if (group && open) walk(n.children, level + 1, curveForChildren(n, v, parent), showAll || (!!q.value && hit(n)));
    }
  };
  walk(props.tree.nodes, 0, null, false);
  return out;
});

function sliderValue(r: Row): number {
  if (drag.value?.node === r.node.name) return drag.value.value;
  return r.curve && "exp" in r.curve ? r.curve.exp : 1;
}

function fill(v: number): string {
  return `${((v - EXP_MIN) / (EXP_MAX - EXP_MIN)) * 100}%`;
}

// The number next to the slider: the exponent, nothing for a custom curve.
function expText(r: Row): string {
  if (drag.value?.node === r.node.name) return drag.value.value.toFixed(2);
  return r.curve && "exp" in r.curve ? r.curve.exp.toFixed(2) : "";
}

function onSlide(r: Row, e: Event) {
  drag.value = { node: r.node.name, value: Number((e.target as HTMLInputElement).value) };
}

function onSlideEnd(r: Row, e: Event) {
  drag.value = null;
  emit("exponent", r.node, roundExp(Number((e.target as HTMLInputElement).value)));
}

function expandAll() {
  closed.value = new Set();
}

function collapseAll() {
  closed.value = new Set(groups.value);
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
</script>

<template>
  <section class="panel" :style="{ '--cols': cols.template.value, '--cols-min': `${cols.minWidth.value}px` }">
    <div class="head">
      <Icon name="invert" :size="16" />
      <span class="head-title">Inversion &amp; Sensitivity Curves</span>
      <div class="divider" />
      <button type="button" class="btn outline small square" title="Expand all" :disabled="!!q || allOpen" @click="expandAll">
        <Icon name="unfold" :size="14" />
      </button>
      <button type="button" class="btn outline small square" title="Collapse all" :disabled="!!q || noneOpen" @click="collapseAll">
        <Icon name="fold" :size="14" />
      </button>
      <div class="spacer" />
      <div class="search">
        <Icon name="search" :size="14" />
        <input v-model="query" placeholder="Find…" />
        <button v-if="query" type="button" class="clear" title="Clear" @click="query = ''">
          <Icon name="close" :size="12" />
        </button>
      </div>
    </div>
    <div class="table">
      <ColumnHead
        :columns="COLUMNS"
        :sort="cols.sort.value"
        @sort="() => {}"
        @resize="cols.startResize"
        @reset="cols.resetWidth"
      />
      <div v-for="r in rows" :key="r.node.name" class="row opt-row" :class="{ group: r.group, changed: r.changed }">
        <span class="setting" :style="{ paddingLeft: `${r.level * 18}px` }" @click="r.group && toggle(r.node)">
          <span class="tw">
            <Icon v-if="r.group" :name="r.open ? 'chevron-down' : 'chevron-right'" :size="14" />
          </span>
          <span class="label">{{ r.label.pre }}<mark v-if="r.label.mark">{{ r.label.mark }}</mark>{{ r.label.post }}</span>
        </span>
        <span v-if="r.curve" class="curve-cell">
          <input
            class="range"
            :class="{ custom: 'pts' in r.curve && drag?.node !== r.node.name, inh: r.inherited && drag?.node !== r.node.name }"
            type="range"
            :min="EXP_MIN"
            :max="EXP_MAX"
            :step="EXP_STEP"
            :value="sliderValue(r)"
            :style="{ '--p': fill(sliderValue(r)) }"
            :aria-label="nodeLabel(r.node)"
            :disabled="disabled"
            @input="onSlide(r, $event)"
            @change="onSlideEnd(r, $event)"
          />
          <span class="val mono" :class="{ inh: r.inherited && drag?.node !== r.node.name }">{{ expText(r) }}</span>
          <button type="button" class="curve-btn" title="Edit Curve" :disabled="disabled" @click="emit('editCurve', r.node)">
            <CurveThumb :curve="r.curve" :dim="r.inherited" />
          </button>
          <button type="button" class="reset" :class="{ off: !r.own }" title="Set Default" :disabled="disabled || !r.own" :tabindex="r.own ? 0 : -1" :aria-hidden="!r.own" @click="emit('resetCurve', r.node)">
            <Icon name="rotate" :size="13" />
          </button>
          <span v-if="r.wipeCurves" class="wipe mono" :title="`${plural(r.wipeCurves, 'curve')} below get replaced`">
            <Icon name="warning" :size="12" />{{ r.wipeCurves }}
          </span>
        </span>
        <span v-else />
        <span v-if="r.node.show_invert === 1" class="invert-cell">
          <YesNo :value="r.invert" :disabled="disabled" @set="emit('invert', r.node, $event)" />
          <button type="button" class="reset" :class="{ off: !r.ownInvert }" title="Set Default" :disabled="disabled || !r.ownInvert" :tabindex="r.ownInvert ? 0 : -1" :aria-hidden="!r.ownInvert" @click="emit('invert', r.node, null)">
            <Icon name="rotate" :size="13" />
          </button>
          <span v-if="r.wipeInverts" class="wipe mono" :title="`${plural(r.wipeInverts, 'invert setting')} below get replaced`">
            <Icon name="warning" :size="12" />{{ r.wipeInverts }}
          </span>
        </span>
        <span v-else />
      </div>
      <div v-if="!rows.length" class="empty-line">No matches</div>
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

.divider {
  width: 1px;
  height: 20px;
  background: var(--border-dim);
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

.btn.square {
  width: var(--h-chip-sm);
  padding: 0;
}

.btn.outline {
  background: transparent;
  color: var(--text);
  border: 1px solid rgba(255, 255, 255, 0.5);
}

.btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-chip);
  width: 200px;
  padding: 0 12px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text-2);
}

.search input {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-size: 13px;
  outline: none;
}

.search input::placeholder {
  color: var(--text-3);
}

.search .clear {
  width: 20px;
  height: 20px;
  margin-right: -4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  flex-shrink: 0;
}

.search .clear:hover {
  color: var(--text);
}

.table {
  overflow-x: auto;
  font-size: 13px;
}

.row {
  display: grid;
  grid-template-columns: var(--cols);
  gap: 12px;
  padding: 4px 16px;
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

.opt-row {
  position: relative;
  min-height: 30px;
}

.opt-row:hover {
  background: var(--bg-surface-2);
}

/* A pending change: the warn bar at the row's left edge. */
.opt-row.changed::before {
  content: "";
  position: absolute;
  left: 4px;
  top: 7px;
  bottom: 7px;
  width: 3px;
  border-radius: 2px;
  background: var(--warn);
}

.opt-row.changed .val {
  color: var(--warn);
}

.setting {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.opt-row.group .setting {
  color: var(--text-2);
  font-weight: 600;
  cursor: pointer;
  user-select: none;
}

.tw {
  width: 14px;
  display: inline-flex;
  justify-content: center;
  color: var(--text-3);
  flex-shrink: 0;
}

.label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

mark {
  background: color-mix(in srgb, var(--accent) 35%, transparent);
  color: inherit;
  border-radius: 2px;
}

.curve-cell,
.invert-cell {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.curve-cell .range {
  flex: 1;
  min-width: 60px;
}

/* A custom curve has no exponent; an inherited one is the group's. */
.range.custom {
  opacity: 0.35;
}

.range.inh {
  opacity: 0.55;
}

.val {
  width: 42px;
  flex-shrink: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.val.inh {
  color: var(--text-3);
}

.curve-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 26px;
  flex-shrink: 0;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-base);
  cursor: pointer;
}

.curve-btn:hover:not(:disabled) {
  border-color: var(--accent);
}

.curve-btn:disabled,
.reset:disabled {
  opacity: 0.4;
  cursor: default;
}

.reset {
  width: 22px;
  height: 22px;
  display: inline-grid;
  place-items: center;
  flex-shrink: 0;
  padding: 0;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}

/* Without an own value the button keeps its place, invisible: the cell never shifts. */
.reset.off {
  visibility: hidden;
}

.reset:hover:not(:disabled) {
  color: var(--text);
  background: var(--bg-surface-2);
}

.wipe {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  height: 18px;
  padding: 0 5px;
  flex-shrink: 0;
  border-radius: 3px;
  background: color-mix(in srgb, var(--warn) 16%, transparent);
  color: var(--warn);
  font-size: 11px;
}

.empty-line {
  padding: 24px 16px;
  text-align: center;
  color: var(--text-3);
}
</style>
