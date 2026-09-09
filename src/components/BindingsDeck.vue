<script setup lang="ts">
import { computed, ref } from "vue";
import Icon from "./Icon.vue";
import type { Action, ActionMap, DeviceInfo, JoyInput, ResolvedBinding } from "../types";

const props = defineProps<{
  bindings: ResolvedBinding[];
  devices: DeviceInfo[];
  actionMaps: ActionMap[];
  events: JoyInput[];
  currentToken: string | null;
  tokenLabel: (token: string) => string;
  categoryLabel: (actionmap: string) => string;
  instanceOf: (token: string) => string;
  isClash: (token: string) => boolean;
  isConnected: (b: ResolvedBinding) => boolean;
  isMissing: (b: ResolvedBinding) => boolean;
  isPinned: (b: ResolvedBinding) => boolean;
  deviceName: (guid: string) => string;
}>();
const emit = defineEmits<{ pin: [b: ResolvedBinding] }>();

type Tab = "bindings" | "actions" | "log";
const tab = ref<Tab>("bindings");
const instanceFilter = ref<string | "all">("all");
const search = ref("");

// Distinct joystick instances present in the bindings, in order, with counts.
const instanceCounts = computed(() => {
  const counts = new Map<string, number>();
  for (const b of props.bindings) {
    const n = props.instanceOf(b.token);
    counts.set(n, (counts.get(n) ?? 0) + 1);
  }
  return [...counts.entries()].sort((a, b) => Number(a[0]) - Number(b[0]));
});

function matchesSearch(b: ResolvedBinding, q: string): boolean {
  if (!q) return true;
  // Search covers the LABEL and ACTION columns only.
  const hay = [labelOf(b.token), b.label ?? b.action].join(" ").toLowerCase();
  return hay.includes(q);
}

const filteredBindings = computed(() => {
  const q = search.value.trim().toLowerCase();
  return props.bindings.filter((b) => {
    if (instanceFilter.value !== "all" && props.instanceOf(b.token) !== instanceFilter.value) return false;
    return matchesSearch(b, q);
  });
});

// Localized input label; empty when SC has none (tokenLabel echoes the token then).
function labelOf(token: string): string {
  const l = props.tokenLabel(token);
  return l === token ? "" : l;
}

// Last 8 hex chars of an SDL GUID: enough to tell devices apart in the log.
function shortGuid(guid: string): string {
  return guid.slice(-8);
}

// Input token without its "jsN_" prefix, e.g. "js1_button5" -> "button5".
function inputPart(token: string): string {
  return token.replace(/^js\d+_/, "");
}

function actionText(a: Action): string {
  return a.label ?? a.name;
}

function rowTitle(b: ResolvedBinding): string | undefined {
  return props.isMissing(b) ? `No area for ${props.tokenLabel(b.token)} on the ${b.device ?? "device"} image` : undefined;
}

function deviceTitle(b: ResolvedBinding): string | undefined {
  if (props.isClash(b.token)) return "misassigned";
  if (!props.isConnected(b)) return "not connected";
  return undefined;
}
</script>

<template>
  <div class="deck">
    <div class="header">
      <button type="button" class="tab" :class="{ active: tab === 'bindings' }" @click="tab = 'bindings'">
        <Icon name="link" :size="14" />Bindings
      </button>
      <button type="button" class="tab" :class="{ active: tab === 'actions' }" @click="tab = 'actions'">
        <Icon name="list" :size="14" />Actions
      </button>
      <button type="button" class="tab" :class="{ active: tab === 'log' }" @click="tab = 'log'">
        <Icon name="log" :size="14" />Log
      </button>
      <template v-if="tab === 'bindings'">
        <div class="divider" />
        <div class="chips">
          <button type="button" class="chip all" :class="{ active: instanceFilter === 'all' }" @click="instanceFilter = 'all'">
            All <span class="count">{{ bindings.length }}</span>
          </button>
          <button
            v-for="[n, count] in instanceCounts"
            :key="n"
            type="button"
            class="chip mono"
            :class="{ active: instanceFilter === n }"
            @click="instanceFilter = n"
          >
            js{{ n }} <span class="count">{{ count }}</span>
          </button>
        </div>
        <div class="spacer" />
        <div class="search">
          <Icon name="search" :size="14" />
          <input v-model="search" placeholder="Find label or action…" />
        </div>
      </template>
      <div v-else class="spacer" />
    </div>

    <template v-if="tab === 'bindings'">
      <div class="row cols-head">
        <span>DEVICE</span><span>INPUT</span><span>LABEL</span><span>ACTION</span><span>CATEGORY</span>
      </div>
      <div class="body">
        <div
          v-for="(b, i) in filteredBindings"
          :key="i"
          class="row binding-row"
          :class="{ current: b.token === currentToken, pinned: isPinned(b), missing: isMissing(b) }"
          :title="rowTitle(b)"
          @click="emit('pin', b)"
        >
          <span class="mono cell-device" :class="{ clash: isClash(b.token), disconnected: !isConnected(b) }" :title="deviceTitle(b)">
            js{{ instanceOf(b.token) }}
          </span>
          <span class="mono cell-input" :title="b.token">{{ inputPart(b.token) }}</span>
          <span class="cell-label">{{ labelOf(b.token) }}</span>
          <span class="cell-action">{{ b.label ?? b.action }}</span>
          <span class="cell-category">{{ categoryLabel(b.actionmap) }}</span>
        </div>
      </div>
    </template>

    <template v-else-if="tab === 'actions'">
      <div class="body scroll">
        <div v-for="map in actionMaps" :key="map.name" class="action-group">
          <div class="group-head">{{ map.label ?? map.name }}</div>
          <div v-for="a in map.actions" :key="a.name" class="action-item">{{ actionText(a) }}</div>
        </div>
      </div>
    </template>

    <template v-else>
      <div class="body scroll mono log">
        <div class="panel-title">Devices</div>
        <div v-for="d in devices" :key="d.index" class="log-device-line">
          <span class="log-key">#{{ d.index }}</span>
          <span>{{ d.sc_name ?? "—" }}</span>
          <span class="log-dim">sdl</span><span>{{ d.sdl_name }}</span>
          <span class="log-dim">guid</span><span>{{ d.sdl_guid }}</span>
          <span class="log-dim">sc</span><span>{{ d.sc_product_guid ?? "—" }}</span>
          <span class="log-dim">io</span><span>{{ d.num_buttons }} btn {{ d.num_axes }} axes {{ d.num_hats }} hats</span>
          <span class="log-dim">axes</span><span>{{ d.axes.length ? d.axes.join(" ") : (d.axes_error ?? "—") }}</span>
        </div>
        <div class="panel-title log-events-title">Events</div>
        <div v-for="(ev, i) in events" :key="i" class="log-item">
          <span class="log-key">{{ shortGuid(ev.guid) }}</span>
          <span class="log-device">{{ deviceName(ev.guid) }}</span>
          <span class="log-desc">{{ ev.kind }} {{ ev.index }}<template v-if="ev.kind === 'button'"> {{ ev.pressed ? "down" : "up" }}</template><template v-else-if="ev.kind === 'axis'"> = {{ ev.value }}</template><template v-else> {{ ev.direction }}</template></span>
        </div>
      </div>
    </template>
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
  padding: 8px 16px 0;
  border-bottom: 1px solid var(--border-dim);
}

.tab {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 12px;
  border: none;
  background: transparent;
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  color: var(--text-2);
  cursor: pointer;
  margin-bottom: 0;
}

.tab.active {
  background: var(--bg-surface-2);
  border-radius: 4px 4px 0 0;
  color: var(--text);
}

.divider {
  width: 1px;
  height: 20px;
  background: var(--border-dim);
  margin: 0 8px 8px;
}

.chips {
  display: flex;
  gap: 4px;
  margin-bottom: 8px;
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
  margin-bottom: 8px;
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
  grid-template-columns: 70px 130px 180px minmax(0, 1fr) 220px;
  gap: 12px;
  padding: 8px 16px;
  align-items: center;
}

.cols-head {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  color: var(--text-2);
  border-bottom: 1px solid var(--border-dim);
}

.body {
  overflow-y: auto;
  min-height: 0;
  flex: 1;
}

.body.scroll {
  padding: 12px 16px;
}

.binding-row {
  font-size: 13px;
  cursor: pointer;
  border-left: 2px solid transparent;
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

.cell-device.disconnected {
  color: var(--text-3);
}

.cell-input {
  color: var(--text-2);
}

.cell-category {
  color: var(--text-2);
}

.action-group {
  margin-bottom: 12px;
}

.group-head {
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  color: var(--text-2);
  margin-bottom: 4px;
}

.action-item {
  font-size: 13px;
  padding: 2px 0;
}

.log {
  font-size: 12px;
}

.log-device-line,
.log-item {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  padding: 3px 0;
}

.log-events-title {
  margin-top: 12px;
}

.log-key {
  color: var(--live);
  min-width: 5rem;
}

.log-dim {
  color: var(--text-3);
}

.log-device {
  min-width: 14rem;
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
