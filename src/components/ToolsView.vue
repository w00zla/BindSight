<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import ColumnHead from "./ColumnHead.vue";
import { collator, sortRows, useTableColumns, type ColumnSpec } from "../tableColumns";
import ConfirmDialog, { type ConfirmButton } from "./ConfirmDialog.vue";
import type {
  ActionMap,
  ActionRef,
  BackupSummary,
  BindingProfileSummary,
  DiffKind,
  DiffReport,
  DiffRow,
  DiffSource,
  LoadStatus,
  ResolvedBinding,
} from "../types";

const props = defineProps<{ bindings: ResolvedBinding[]; actionMaps: ActionMap[] }>();
const emit = defineEmits<{
  notify: [message: string, type: "ok" | "error"];
  restored: [status: LoadStatus];
}>();

const actionCount = computed(() => props.actionMaps.reduce((n, m) => n + m.actions.length, 0));

const profiles = ref<BindingProfileSummary[]>([]);
const backups = ref<BackupSummary[]>([]);
const report = ref<DiffReport | null>(null);
// A command is running; the action buttons stay out of the way until it is done.
const busy = ref(false);

// --- confirm dialog --------------------------------------------------------

const confirm = ref<{ title: string; buttons: ConfirmButton[] } | null>(null);
let confirmResolve: ((value: string) => void) | null = null;

function ask(title: string, buttons: ConfirmButton[]): Promise<string> {
  confirm.value = { title, buttons };
  return new Promise((resolve) => {
    confirmResolve = resolve;
  });
}

function onConfirm(value: string) {
  confirm.value = null;
  const resolve = confirmResolve;
  confirmResolve = null;
  resolve?.(value);
}

// --- time formatting -------------------------------------------------------

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

// "Sep 06" within the current year, "2025-08-21" before it. Local time.
function shortDate(unixSecs: number): string {
  const d = new Date(unixSecs * 1000);
  if (d.getFullYear() === new Date().getFullYear()) {
    return d.toLocaleDateString("en-US", { month: "short", day: "2-digit" });
  }
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

// "2026-09-09 13:40", local time.
function stamp(unixSecs: number): string {
  const d = new Date(unixSecs * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

// --- compare sources -------------------------------------------------------

// A source is addressed by a flat key so it can live in a <select> value:
// "current", "profile:<file>" or "backup:<id>".
const CURRENT = "current";
const PROFILE_PREFIX = "profile:";

const aKey = ref(CURRENT);
const bKey = ref(CURRENT);

interface SourceOption {
  key: string;
  name: string;
}

const sourceOptions = computed<SourceOption[]>(() => [
  { key: CURRENT, name: "Current" },
  ...profiles.value.map((m) => ({ key: `${PROFILE_PREFIX}${m.file}`, name: m.name })),
  ...backups.value.map((b) => ({ key: `backup:${b.id}`, name: `${stamp(b.created)} · ${b.reason}` })),
]);

function sourceFor(key: string): DiffSource {
  if (key.startsWith(PROFILE_PREFIX)) return { kind: "profile", file: key.slice(PROFILE_PREFIX.length) };
  if (key.startsWith("backup:")) return { kind: "backup", id: key.slice(7) };
  return { kind: "current" };
}

function nameFor(key: string): string {
  return sourceOptions.value.find((o) => o.key === key)?.name ?? "—";
}

// The B side, when it is an exported binding profile (Export works on that only).
const bProfile = computed<BindingProfileSummary | null>(() => {
  const s = sourceFor(bKey.value);
  return s.kind === "profile" ? profiles.value.find((m) => m.file === s.file) ?? null : null;
});

// A source that vanished (deleted backup, reloaded list) falls back to Current.
function ensureKeys() {
  const keys = sourceOptions.value.map((o) => o.key);
  if (!keys.includes(aKey.value)) aKey.value = CURRENT;
  if (!keys.includes(bKey.value)) bKey.value = CURRENT;
}

// --- loading ---------------------------------------------------------------

async function loadProfiles() {
  try {
    profiles.value = await invoke<BindingProfileSummary[]>("list_binding_profiles");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function loadBackups() {
  try {
    backups.value = await invoke<BackupSummary[]>("list_backups");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function runCompare() {
  try {
    report.value = await invoke<DiffReport>("compare_bindings", {
      a: sourceFor(aKey.value),
      b: sourceFor(bKey.value),
    });
  } catch (e) {
    report.value = null;
    emit("notify", String(e), "error");
  }
}

// --- binding profiles -------------------------------------------------------

async function importProfile() {
  busy.value = true;
  try {
    const src = await open({ multiple: false, filters: [{ name: "Binding profile", extensions: ["xml"] }] });
    if (!src) return;
    const s = await invoke<BindingProfileSummary>("import_binding_profile", { sourcePath: src });
    await loadProfiles();
    bKey.value = `${PROFILE_PREFIX}${s.file}`;
    await runCompare();
    emit("notify", `Imported ${s.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function exportProfile() {
  const m = bProfile.value;
  if (!m) return;
  busy.value = true;
  try {
    const dest = await save({ defaultPath: m.file, filters: [{ name: "Binding profile", extensions: ["xml"] }] });
    if (!dest) return;
    await invoke("export_binding_profile", { file: m.file, destPath: dest });
    emit("notify", `Exported ${m.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// --- backups ---------------------------------------------------------------

async function createBackup() {
  busy.value = true;
  try {
    await invoke<BackupSummary>("create_backup", { reason: "manual" });
    await loadBackups();
    emit("notify", "Backup created", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function restoreBackup(b: BackupSummary) {
  const choice = await ask("Restore backup?", [
    { label: "Restore", kind: "primary", value: "restore" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice !== "restore") return;
  busy.value = true;
  try {
    const s = await invoke<LoadStatus>("restore_backup", { id: b.id });
    emit("restored", s);
    // Restoring takes a safety backup of its own — the list has a new entry.
    await loadBackups();
    ensureKeys();
    await runCompare();
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function deleteBackup(b: BackupSummary) {
  const choice = await ask("Delete backup?", [
    { label: "Delete", kind: "danger", value: "delete" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice !== "delete") return;
  busy.value = true;
  try {
    await invoke("delete_backup", { id: b.id });
    await loadBackups();
    ensureKeys();
    await runCompare();
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// --- diff filters ----------------------------------------------------------

const kindFilter = ref<DiffKind | "all">("all");
const instanceFilter = ref<number[]>([]);
const search = ref("");

// Joystick instances present in the report, ascending.
const instances = computed<number[]>(() => {
  const set = new Set<number>();
  for (const r of report.value?.rows ?? []) if (r.instance !== null) set.add(r.instance);
  return [...set].sort((a, b) => a - b);
});

function toggleInstance(n: number) {
  instanceFilter.value = instanceFilter.value.includes(n)
    ? instanceFilter.value.filter((x) => x !== n)
    : [...instanceFilter.value, n];
}

function refText(r: ActionRef): string {
  return r.label ?? r.action;
}

// Haystack for the search box: token plus every action name and label.
function haystack(row: DiffRow): string {
  const refs = [...row.a, ...row.b];
  return [row.token, ...refs.map((r) => r.action), ...refs.map((r) => r.label ?? "")].join(" ").toLowerCase();
}

const COLUMNS: ColumnSpec[] = [
  { key: "sign", label: "", width: 28 },
  { key: "device", label: "DEVICE", width: 70 },
  { key: "input", label: "INPUT", width: 130 },
  { key: "action", label: "ACTION", width: 300 },
  { key: "a", label: "A", width: 260 },
  { key: "b", label: "B", width: null },
];
const cols = useTableColumns("bindsight.columns.compare", COLUMNS, { key: "input", dir: "asc" });
// The A/B headers carry the source names.
const columns = computed<ColumnSpec[]>(() =>
  COLUMNS.map((c) =>
    c.key === "a" || c.key === "b" ? { ...c, label: nameFor(c.key === "a" ? aKey.value : bKey.value).toUpperCase() } : c,
  ),
);

function cellValue(r: DiffRow, key: string): string | number {
  switch (key) {
    case "sign":
      return r.kind;
    case "device":
      return r.instance ?? -1;
    case "input":
      return inputPart(r.token);
    case "action":
      return rowAction(r);
    case "a":
      return cellText(r.a);
    default:
      return cellText(r.b);
  }
}

const filteredRows = computed<DiffRow[]>(() => {
  const q = search.value.trim().toLowerCase();
  const rows = (report.value?.rows ?? []).filter((r) => {
    if (kindFilter.value !== "all" && r.kind !== kindFilter.value) return false;
    if (instanceFilter.value.length && (r.instance === null || !instanceFilter.value.includes(r.instance))) return false;
    return !q || haystack(r).includes(q);
  });
  return sortRows(rows, cols.sort.value, cellValue, (a, b) => collator.compare(a.token, b.token));
});

// The action a row is about: the A side names it, else the B side.
function rowAction(row: DiffRow): string {
  const r = row.a[0] ?? row.b[0];
  return r ? refText(r) : "—";
}

function cellText(refs: ActionRef[]): string {
  return refs.length ? refs.map(refText).join(", ") : "—";
}

const SIGNS: Record<DiffKind, string> = { added: "+", removed: "−", changed: "~" };

// Input token without its jsN_ prefix, e.g. "js1_button5" -> "button5".
function inputPart(token: string): string {
  return token.replace(/^js\d+_/, "");
}

const sameSource = computed(() => aKey.value === bKey.value);

// --- wiring ----------------------------------------------------------------

watch([aKey, bKey], runCompare);

// The live bindings changed (reload, restore, resort) — re-diff if a side is Current.
watch(
  () => props.bindings,
  () => {
    if (aKey.value === CURRENT || bKey.value === CURRENT) runCompare();
  },
);

onMounted(async () => {
  await Promise.all([loadProfiles(), loadBackups()]);
  // B starts on the newest layout, so the first view says something.
  const newest = [...profiles.value].sort((a, b) => b.modified - a.modified)[0];
  if (newest) bKey.value = `${PROFILE_PREFIX}${newest.file}`;
  else await runCompare();
});
</script>

<template>
  <div class="tools">
    <div class="left">
      <!-- game bindings: SC's action master list -->
      <section class="panel game">
        <div class="head">
          <Icon name="list" :size="15" />
          <span class="head-title">Game bindings</span>
          <span class="head-count">{{ actionCount }}</span>
        </div>
        <div class="rows scroll actions">
          <div v-for="m in actionMaps" :key="m.name" class="action-group">
            <div class="group-head">{{ m.label ?? m.name }}</div>
            <div v-for="a in m.actions" :key="a.name" class="action-item">{{ a.label ?? a.name }}</div>
          </div>
          <div v-if="!actionMaps.length" class="row-none">None</div>
        </div>
      </section>

      <!-- binding profiles -->
      <section class="panel">
        <div class="head">
          <Icon name="file" :size="15" />
          <span class="head-title">Binding profiles</span>
          <span class="head-count">{{ profiles.length }}</span>
        </div>
        <div class="rows">
          <div class="row-item" :class="{ a: aKey === CURRENT, b: bKey === CURRENT }" @click="bKey = CURRENT">
            <span class="dot" />
            <div class="lines">
              <span class="line-title">Current</span>
              <span class="mono line-sub">actionmaps.xml · {{ bindings.length }}</span>
            </div>
          </div>
          <div
            v-for="m in profiles"
            :key="m.file"
            class="row-item"
            :class="{ a: aKey === `${PROFILE_PREFIX}${m.file}`, b: bKey === `${PROFILE_PREFIX}${m.file}` }"
            @click="bKey = `${PROFILE_PREFIX}${m.file}`"
          >
            <Icon name="file" :size="14" />
            <div class="lines">
              <span class="line-title">{{ m.name }}</span>
              <span class="mono line-sub">{{ m.file }} · {{ m.bindings }} · {{ shortDate(m.modified) }}</span>
            </div>
          </div>
          <div v-if="!profiles.length" class="row-none">None</div>
        </div>
        <div class="foot">
          <button type="button" class="btn outline" :disabled="busy" @click="importProfile">
            <Icon name="download" :size="14" />Import
          </button>
          <button type="button" class="btn outline" :disabled="busy || !bProfile" @click="exportProfile">
            <Icon name="upload" :size="14" />Export
          </button>
          <button type="button" class="btn outline" disabled title="Not yet">
            <Icon name="plus" :size="14" />New
          </button>
        </div>
      </section>

      <!-- backups -->
      <section class="panel grow">
        <div class="head">
          <Icon name="history" :size="15" />
          <span class="head-title">Backups</span>
          <button type="button" class="btn primary small" :disabled="busy" @click="createBackup">
            <Icon name="plus" :size="12" />Backup now
          </button>
        </div>
        <div class="rows scroll">
          <div
            v-for="b in backups"
            :key="b.id"
            class="row-item backup"
            :class="{ a: aKey === `backup:${b.id}`, b: bKey === `backup:${b.id}` }"
            @click="bKey = `backup:${b.id}`"
          >
            <div class="lines">
              <span class="mono line-stamp">{{ stamp(b.created) }}</span>
              <span class="line-sub">{{ b.reason }}</span>
            </div>
            <span class="line-sub">{{ b.bindings }}</span>
            <button type="button" class="icon-btn" title="Restore" :disabled="busy" @click.stop="restoreBackup(b)">
              <Icon name="rotate" :size="14" />
            </button>
            <button type="button" class="icon-btn" title="Delete" :disabled="busy" @click.stop="deleteBackup(b)">
              <Icon name="trash" :size="14" />
            </button>
          </div>
          <div v-if="!backups.length" class="row-none">None</div>
        </div>
      </section>
    </div>

    <!-- compare -->
    <section class="panel compare" :style="{ '--cols': cols.template.value, '--cols-min': `${cols.minWidth.value}px` }">
      <div class="head compare-head">
        <Icon name="compare" :size="16" />
        <span class="head-title no-grow">Compare</span>
        <label class="sel">
          <select v-model="aKey">
            <option v-for="o in sourceOptions" :key="o.key" :value="o.key">{{ o.name }}</option>
          </select>
          <span class="sel-name">{{ nameFor(aKey) }}</span>
          <Icon name="chevron-down" :size="12" />
        </label>
        <Icon name="arrow-right" :size="18" class="dim" />
        <label class="sel">
          <select v-model="bKey">
            <option v-for="o in sourceOptions" :key="o.key" :value="o.key">{{ o.name }}</option>
          </select>
          <span class="sel-name">{{ nameFor(bKey) }}</span>
          <Icon name="chevron-down" :size="12" />
        </label>
        <div class="spacer" />
        <div class="chips">
          <button type="button" class="chip" :class="{ active: kindFilter === 'all' }" @click="kindFilter = 'all'">
            All <span class="count">{{ report?.rows.length ?? 0 }}</span>
          </button>
          <button
            type="button"
            class="chip added"
            :class="{ active: kindFilter === 'added' }"
            @click="kindFilter = 'added'"
          >
            +{{ report?.added ?? 0 }}
          </button>
          <button
            type="button"
            class="chip removed"
            :class="{ active: kindFilter === 'removed' }"
            @click="kindFilter = 'removed'"
          >
            −{{ report?.removed ?? 0 }}
          </button>
          <button
            type="button"
            class="chip changed"
            :class="{ active: kindFilter === 'changed' }"
            @click="kindFilter = 'changed'"
          >
            ~{{ report?.changed ?? 0 }}
          </button>
        </div>
        <div v-if="instances.length" class="divider" />
        <div v-if="instances.length" class="chips">
          <button
            v-for="n in instances"
            :key="n"
            type="button"
            class="chip mono"
            :class="{ active: instanceFilter.includes(n) }"
            @click="toggleInstance(n)"
          >
            js{{ n }}
          </button>
        </div>
        <div class="search">
          <Icon name="search" :size="14" />
          <input v-model="search" placeholder="Find…" />
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
        <div v-for="r in filteredRows" :key="r.token" class="row diff-row" :class="r.kind">
          <span class="sign">{{ SIGNS[r.kind] }}</span>
          <span class="mono dim">{{ r.instance === null ? "—" : `js${r.instance}` }}</span>
          <span class="mono dim" :title="r.token">{{ inputPart(r.token) }}</span>
          <span>{{ rowAction(r) }}</span>
          <span :class="r.a.length ? 'side' : 'empty'">{{ cellText(r.a) }}</span>
          <span :class="r.b.length ? 'side' : 'empty'">{{ cellText(r.b) }}</span>
        </div>
        <div v-if="!filteredRows.length" class="empty-line">
          {{ sameSource ? "Same source" : "No differences" }}
        </div>
      </div>
    </section>

    <ConfirmDialog v-if="confirm" :title="confirm.title" :buttons="confirm.buttons" @choose="onConfirm" />
  </div>
</template>

<style scoped>
.tools {
  flex: 1;
  display: grid;
  grid-template-columns: 360px minmax(0, 1fr);
  gap: 16px;
  padding: 12px 16px 16px;
  min-height: 0;
}

.left {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-height: 0;
}

.panel {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.panel.grow {
  flex: 1;
  min-height: 0;
}

/* Capped so profiles and backups stay in view. */
.panel.game {
  flex: 0 1 40%;
  min-height: 0;
}

.rows.actions {
  padding: 10px 14px;
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
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border-dim);
}

.head-title {
  flex: 1;
  font-weight: 600;
  font-size: 14px;
}

.head-title.no-grow {
  flex: none;
}

.head-count {
  font-size: 12px;
  color: var(--text-2);
}

/* --- left column rows --- */

.rows {
  display: flex;
  flex-direction: column;
  padding: 6px;
}

.rows.scroll {
  overflow-y: auto;
  min-height: 0;
  flex: 1;
}

.row-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 10px;
  border-radius: 6px;
  border: 1px solid transparent;
  color: var(--text-2);
  cursor: pointer;
}

.row-item.a {
  background: var(--bg-surface-2);
}

.row-item.b {
  border-color: var(--accent);
}

.row-item.backup {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto auto;
  gap: 10px;
  align-items: center;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--live);
}

.lines {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.line-title {
  font-weight: 600;
  font-size: 13px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.line-stamp {
  font-weight: 600;
  font-size: 13px;
  color: var(--text);
}

.line-sub {
  font-size: 11px;
  color: var(--text-2);
}

.row-none {
  padding: 9px 10px;
  font-size: 13px;
  color: var(--text-3);
}

.foot {
  display: flex;
  gap: 6px;
  padding: 6px 12px 12px;
}

.foot .btn {
  flex: 1;
  height: 34px;
  padding: 0 8px;
}

/* --- buttons --- */

.btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  height: var(--h-control);
  padding: 0 16px;
  border-radius: var(--radius-control);
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  cursor: pointer;
  white-space: nowrap;
}

.btn.small {
  height: var(--h-chip-sm);
  padding: 0 10px;
  gap: 6px;
  font-size: 12px;
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

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}

.icon-btn:hover {
  color: var(--text);
}

.icon-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

/* --- compare --- */

.compare {
  min-height: 0;
}

.compare-head {
  gap: 10px;
  padding: 12px 16px;
  flex-wrap: wrap;
  row-gap: 8px;
}

.dim {
  color: var(--text-2);
}

/* A native select painted as a chip: the real control sits on top, invisible. */
.sel {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-chip);
  padding: 0 12px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  font-size: 13px;
  color: var(--text-2);
  cursor: pointer;
  max-width: 240px;
}

.sel select {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  opacity: 0;
  border: none;
  font-family: inherit;
  cursor: pointer;
}

.sel-name {
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
  border: 1px solid rgba(173, 211, 235, 0.35);
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  cursor: pointer;
}

.chip .count {
  color: var(--text-2);
  font-weight: 400;
}

.chip.added {
  color: var(--ok);
  border-color: color-mix(in srgb, var(--ok) 50%, transparent);
}

.chip.removed {
  color: var(--err);
  border-color: color-mix(in srgb, var(--err) 50%, transparent);
}

.chip.changed {
  color: var(--warn);
  border-color: color-mix(in srgb, var(--warn) 50%, transparent);
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

.divider {
  width: 1px;
  height: 20px;
  background: var(--border-dim);
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
  color: rgba(173, 211, 235, 0.6);
}

/* --- diff table --- */

.row {
  display: grid;
  grid-template-columns: var(--cols);
  gap: 12px;
  padding: 8px 16px;
  align-items: center;
  /* Content-box: padding comes on top. Rows widen, the table scrolls. */
  min-width: var(--cols-min);
}

.cols-head {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--bg-surface);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-2);
  border-bottom: 1px solid var(--border-dim);
}

.table {
  overflow: auto;
  min-height: 0;
  flex: 1;
  font-size: 13px;
}

.diff-row span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sign {
  font-weight: 700;
}

.diff-row.added {
  background: color-mix(in srgb, var(--ok) 6%, transparent);
}

.diff-row.added .sign,
.diff-row.added .side {
  color: var(--ok);
}

.diff-row.removed {
  background: color-mix(in srgb, var(--err) 6%, transparent);
}

.diff-row.removed .sign,
.diff-row.removed .side {
  color: var(--err);
}

.diff-row.changed {
  background: color-mix(in srgb, var(--warn) 6%, transparent);
}

.diff-row.changed .sign,
.diff-row.changed .side {
  color: var(--warn);
}

.diff-row .dim {
  color: var(--text-2);
}

.diff-row .empty {
  color: var(--text-3);
}

.empty-line {
  padding: 24px 16px;
  text-align: center;
  color: var(--text-3);
}
</style>
