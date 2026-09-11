<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import Dropdown from "./Dropdown.vue";
import ColumnHead from "./ColumnHead.vue";
import { collator, sortRows, useTableColumns, type ColumnSpec } from "../tableColumns";
import ConfirmDialog, { type ConfirmButton, type ConfirmIcon } from "./ConfirmDialog.vue";
import { KIND_RANK } from "../devices";
import { persistedRef } from "../persist";
import { recording } from "../keyboard";
import type {
  ActionMap,
  ActionRef,
  BackupSummary,
  BindingProfileSummary,
  BoundAction,
  DeviceKind,
  DiffKind,
  DiffReport,
  DiffRow,
  DiffSource,
  JoyInput,
  LoadStatus,
  ProfileInfo,
  RebindChange,
  ResolvedBinding,
} from "../types";

// `hasCurrent`: the live actionmaps.xml is loaded (else there is no Current
// source and no list). `keyInput`: the last captured key (only the webview
// sees keys). `inputToken`: the full SC token a keyboard / gamepad press
// stands for, modifiers included; joystick inputs are resolved by the backend.
const props = defineProps<{
  bindings: ResolvedBinding[];
  actionMaps: ActionMap[];
  hasCurrent: boolean;
  keyInput: JoyInput | null;
  // SC's label for an input token; echoes the token when there is none.
  tokenLabel: (token: string) => string;
  inputToken: (p: JoyInput) => string | null;
}>();
const emit = defineEmits<{
  notify: [message: string, type: "ok" | "error"];
  restored: [status: LoadStatus];
  saved: [status: LoadStatus];
}>();

const profiles = ref<BindingProfileSummary[]>([]);
const backups = ref<BackupSummary[]>([]);
const report = ref<DiffReport | null>(null);
// A command is running; the action buttons stay out of the way until it is done.
const busy = ref(false);

// The right-hand tile: the bindings list (Current) or Compare (any other
// source picked on the left).
const view = ref<"list" | "compare">("list");

// --- confirm dialog --------------------------------------------------------

const confirm = ref<{ title: string; icon: ConfirmIcon; buttons: ConfirmButton[] } | null>(null);
let confirmResolve: ((value: string) => void) | null = null;

function ask(title: string, icon: ConfirmIcon, buttons: ConfirmButton[]): Promise<string> {
  confirm.value = { title, icon, buttons };
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

function toggleIn<T>(list: T[], item: T): T[] {
  return list.includes(item) ? list.filter((x) => x !== item) : [...list, item];
}

// --- time formatting -------------------------------------------------------

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

// "2026-09-09 13:40:05", local time.
function stamp(unixSecs: number): string {
  const d = new Date(unixSecs * 1000);
  const date = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  return `${date} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
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
  ...(props.hasCurrent ? [{ key: CURRENT, name: "Current" }] : []),
  ...profiles.value.map((m) => ({ key: `${PROFILE_PREFIX}${m.file}`, name: m.name })),
  ...backups.value.map((b) => ({ key: `backup:${b.id}`, name: `${stamp(b.created)} · ${b.reason}` })),
]);

const sourceDropdown = computed(() => sourceOptions.value.map((o) => ({ value: o.key, label: o.name })));

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

// A source that vanished (deleted backup, reloaded list, no Current) falls
// back to Current, else to the first source there is.
function ensureKeys() {
  const keys = sourceOptions.value.map((o) => o.key);
  const fallback = keys.includes(CURRENT) ? CURRENT : (keys[0] ?? "");
  if (!keys.includes(aKey.value)) aKey.value = fallback;
  if (!keys.includes(bKey.value)) bKey.value = fallback;
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
  if (!aKey.value || !bKey.value) {
    report.value = null;
    return;
  }
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
    // Import writes into the game's controls/mappings folder: ask first.
    const name = src.split(/[\\/]/).pop() ?? src;
    const choice = await ask(`Import ${name}?`, "download", [
      { label: "Import", kind: "primary", value: "import" },
      { label: "Cancel", kind: "outline", value: "cancel" },
    ]);
    if (choice !== "import") return;
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
  const choice = await ask("Restore backup?", "rotate", [
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
  const choice = await ask("Delete backup?", "trash", [
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

// Immediate: opens the backup's folder in the file manager.
async function openBackupDir(b: BackupSummary) {
  try {
    await invoke("open_backup_dir", { id: b.id });
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// --- diff filters ----------------------------------------------------------

// Toggled chips; none toggled = everything. Remembered across restarts.
const kindFilter = persistedRef<DiffKind[]>("bindsight.compare.kinds", []);
const deviceFilter = persistedRef<string[]>("bindsight.compare.devices", []);

const search = ref("");

// Same order as the device tiles, joysticks by instance.
function deviceRank(r: DiffRow): number {
  return KIND_RANK[r.device_kind] * 100 + (r.instance ?? -1);
}

// SC's device name for a row: js1, js2, kb1, gp1 — or "—" for a token whose
// device the backend could not name.
function deviceLabel(r: DiffRow): string {
  if (r.device_kind === "keyboard") return "kb1";
  if (r.device_kind === "gamepad") return "gp1";
  return r.instance === null ? "—" : `js${r.instance}`;
}

// Distinct devices present in the report, in tile order, with counts.
const deviceCounts = computed(() => {
  const counts = new Map<string, { rank: number; count: number }>();
  for (const r of report.value?.rows ?? []) {
    const label = deviceLabel(r);
    const hit = counts.get(label);
    if (hit) hit.count += 1;
    else counts.set(label, { rank: deviceRank(r), count: 1 });
  }
  return [...counts.entries()].sort((a, b) => a[1].rank - b[1].rank);
});

function refText(r: ActionRef): string {
  return r.label ?? r.action;
}

// Haystack for the search box: token plus every action name and label.
function haystack(row: DiffRow): string {
  const refs = [...row.a, ...row.b];
  return [row.token, inputText(row.token), ...refs.map((r) => r.action), ...refs.map((r) => r.label ?? "")]
    .join(" ")
    .toLowerCase();
}

const COLUMNS: ColumnSpec[] = [
  { key: "sign", label: "", width: 28 },
  { key: "input", label: "INPUT", width: 200, icon: "bolt" },
  { key: "action", label: "ACTION", width: 300, icon: "target" },
  { key: "a", label: "A", width: 260, icon: "file" },
  { key: "b", label: "B", width: null, icon: "file" },
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
    // Device order first (like the tiles), then the input.
    case "input":
      return `${String(deviceRank(r)).padStart(4, "0")} ${inputText(r.token)}`;
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
    if (kindFilter.value.length && !kindFilter.value.includes(r.kind)) return false;
    if (deviceFilter.value.length && !deviceFilter.value.includes(deviceLabel(r))) return false;
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

// Input token without its device prefix, e.g. "js1_button5" -> "button5".
function inputPart(token: string): string {
  return token.replace(/^(js\d+|kb1|gp1)_/, "");
}

// The INPUT cell: SC's label, else the bare token (like the deck).
function inputText(token: string): string {
  const l = props.tokenLabel(token);
  return l === token ? inputPart(token) : l;
}

const sameSource = computed(() => aKey.value === bKey.value);

// --- bindings list ---------------------------------------------------------

// Facts about the live file; null while nothing is loaded.
const info = ref<ProfileInfo | null>(null);

async function loadInfo() {
  try {
    info.value = await invoke<ProfileInfo | null>("get_profile_info");
  } catch (e) {
    info.value = null;
    emit("notify", String(e), "error");
  }
}

// One column per device the file knows: the keyboard and the gamepad (SC
// has exactly one of each), then every joystick slot named in <options>.
interface DeviceCol {
  key: string;
  kind: DeviceKind;
  instance: number;
  label: string;
}

const deviceCols = computed<DeviceCol[]>(() => [
  { key: "kb1", kind: "keyboard", instance: 1, label: "kb1 · Keyboard/Mouse" },
  { key: "gp1", kind: "gamepad", instance: 1, label: "gp1 · Gamepad" },
  ...[...(info.value?.joysticks ?? [])]
    .sort((a, b) => a.instance - b.instance)
    .map((j) => ({
      key: `js${j.instance}`,
      kind: "joystick" as DeviceKind,
      instance: j.instance,
      label: `js${j.instance} · ${j.product_name}`,
    })),
]);

// Columns the user switched off (remembered; a device new to the file
// starts visible).
const hiddenCols = persistedRef<string[]>("bindsight.bindings.hidden", []);
const visibleCols = computed(() => deviceCols.value.filter((c) => !hiddenCols.value.includes(c.key)));

function toggleCol(key: string) {
  hiddenCols.value = toggleIn(hiddenCols.value, key);
}

// Bindings in the file, total and per device in column order:
// "522 total · 43 kb1 · 6 gp1 · 120 js1".
const countSummary = computed(() => {
  const counts = new Map<string, number>();
  for (const b of props.bindings) {
    const key = deviceKeyOf(b.device_kind, b.instance);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const perDevice = deviceCols.value.map((c) => `${counts.get(c.key) ?? 0} ${c.key}`);
  return [`${props.bindings.length} total`, ...perDevice].join(" · ");
});

// The list is in the game's order and not sortable; the last visible device
// column is the filler.
const listColumns = computed<ColumnSpec[]>(() => [
  { key: "action", label: "ACTION", width: 320, sortable: false, icon: "target" },
  ...visibleCols.value.map((d, i, all) => ({
    key: d.key,
    label: d.label.toUpperCase(),
    width: i === all.length - 1 ? null : 200,
    sortable: false,
    icon: d.kind === "keyboard" ? "keyboard" : d.kind === "gamepad" ? "gamepad" : "devices",
  })),
]);
const listCols = useTableColumns("bindsight.columns.bindings", listColumns, { key: "action", dir: "asc" });

interface ListRow {
  actionmap: string;
  action: string;
  label: string;
}

// The game's category: actionmaps sharing a label are one group in its
// keybinding screen too (the four "On Foot - All" maps).
interface ListGroup {
  key: string;
  label: string;
  rows: ListRow[];
}

const groups = computed<ListGroup[]>(() => {
  const out: ListGroup[] = [];
  const byLabel = new Map<string, ListGroup>();
  for (const m of props.actionMaps) {
    const label = m.label ?? m.name;
    let g = byLabel.get(label);
    if (!g) {
      g = { key: label, label, rows: [] };
      byLabel.set(label, g);
      out.push(g);
    }
    for (const a of m.actions) g.rows.push({ actionmap: m.name, action: a.name, label: a.label ?? a.name });
  }
  return out;
});

// Expanded categories; everything starts collapsed like in the game.
const expanded = ref(new Set<string>());

function toggleGroup(key: string) {
  const next = new Set(expanded.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  expanded.value = next;
}

const listSearch = ref("");

// While searching: only the matching rows (label, internal name, category,
// any cell text), their groups forced open.
const shownGroups = computed<ListGroup[]>(() => {
  const q = listSearch.value.trim().toLowerCase();
  if (!q) return groups.value;
  return groups.value
    .map((g) => ({
      ...g,
      rows: g.rows.filter((r) =>
        [r.label, r.action, g.label, ...visibleCols.value.map((c) => bindText(r, c))].join(" ").toLowerCase().includes(q),
      ),
    }))
    .filter((g) => g.rows.length);
});

function isOpen(g: ListGroup): boolean {
  return listSearch.value.trim() !== "" || expanded.value.has(g.key);
}

const allExpanded = computed(() => groups.value.length > 0 && groups.value.every((g) => expanded.value.has(g.key)));

function expandAll() {
  expanded.value = new Set(groups.value.map((g) => g.key));
}

function collapseAll() {
  expanded.value = new Set();
}

function rowKey(actionmap: string, action: string): string {
  return `${actionmap}\u0000${action}`;
}

// The file's tokens per action and device column.
const fileTokens = computed(() => {
  const m = new Map<string, string[]>();
  for (const b of props.bindings) {
    const key = `${rowKey(b.actionmap, b.action)}\u0000${deviceKeyOf(b.device_kind, b.instance)}`;
    const list = m.get(key);
    if (list) list.push(b.token);
    else m.set(key, [b.token]);
  }
  return m;
});

function deviceKeyOf(kind: DeviceKind, instance: number): string {
  return kind === "keyboard" ? "kb1" : kind === "gamepad" ? "gp1" : `js${instance}`;
}

// Device kind and instance from a full SC token (`js2_button5` -> joystick 2).
function parseToken(token: string): { kind: DeviceKind; instance: number } | null {
  const m = /^(js|kb|gp)(\d+)_/.exec(token);
  if (!m) return null;
  const kind: DeviceKind = m[1] === "js" ? "joystick" : m[1] === "kb" ? "keyboard" : "gamepad";
  return { kind, instance: Number(m[2]) };
}

// SC's "deliberately unbound" rebind for a kind: a blank token on instance 1.
function blankToken(kind: DeviceKind): string {
  return `${kind === "keyboard" ? "kb" : kind === "gamepad" ? "gp" : "js"}1_ `;
}

function isBlank(token: string): boolean {
  return /^(js|kb|gp)\d+_\s*$/.test(token);
}

// Unsaved rebinds, keyed by action and device kind: SC keeps one binding
// per kind, so a rebind replaces whatever the kind had, on any instance.
const pending = ref(new Map<string, RebindChange>());
const dirty = computed(() => pending.value.size > 0);

function pendingKey(actionmap: string, action: string, kind: DeviceKind): string {
  return `${rowKey(actionmap, action)}\u0000${kind}`;
}

// What a cell shows: the pending rebind of that kind when it lands on this
// device (nothing on the kind's other devices), else the file's tokens.
function cellTokens(row: ListRow, col: DeviceCol): { tokens: string[]; pending: boolean } {
  const p = pending.value.get(pendingKey(row.actionmap, row.action, col.kind));
  if (p) {
    const target = parseToken(p.input);
    // A pending unbind empties every column of the kind.
    return { tokens: target?.instance === col.instance && !isBlank(p.input) ? [p.input] : [], pending: true };
  }
  return { tokens: fileTokens.value.get(`${rowKey(row.actionmap, row.action)}\u0000${col.key}`) ?? [], pending: false };
}

function bindText(row: ListRow, col: DeviceCol): string {
  const tokens = cellTokens(row, col).tokens;
  return tokens.length ? tokens.map(inputText).join(", ") : "";
}

// --- live highlight --------------------------------------------------------

// The last press's SC token, and whether it is still lit: a button / key
// stays lit while held, a hat / axis pulses for PULSE_MS. Rows bound to the
// token (pending ones too) mark their cell and category while lit and fade
// afterwards.
const PULSE_MS = 600;
const liveToken = ref<string | null>(null);
const liveOn = ref(false);
let pulseTimer: ReturnType<typeof setTimeout> | null = null;

function light(token: string, momentary: boolean) {
  if (pulseTimer) clearTimeout(pulseTimer);
  pulseTimer = null;
  liveToken.value = token;
  liveOn.value = true;
  if (momentary) {
    pulseTimer = setTimeout(() => {
      liveOn.value = false;
    }, PULSE_MS);
  }
}

function unlight(token: string) {
  if (token === liveToken.value) liveOn.value = false;
}

const liveRows = computed<Set<string>>(() => {
  const t = liveToken.value;
  const rows = new Set<string>();
  if (!t) return rows;
  for (const b of props.bindings) if (b.token === t) rows.add(rowKey(b.actionmap, b.action));
  for (const p of pending.value.values()) if (p.input === t) rows.add(rowKey(p.actionmap, p.action));
  return rows;
});

function isLive(row: ListRow): boolean {
  return liveRows.value.has(rowKey(row.actionmap, row.action));
}

function isLiveCell(row: ListRow, col: DeviceCol): boolean {
  return liveToken.value !== null && cellTokens(row, col).tokens.includes(liveToken.value);
}

// --- rebind dialog ---------------------------------------------------------

// Past half travel an axis counts as pressed.
const AXIS_PRESS = 16384;

interface RebindState {
  row: ListRow;
  category: string;
  // The changes gathered so far, one per device kind (SC keeps one binding
  // per kind): the recorded token, or the blank token for a clear. Every
  // device the file names is on show at once.
  changes: Map<DeviceKind, string>;
}

const rebind = ref<RebindState | null>(null);

function openRebind(row: ListRow, group: ListGroup) {
  if (!props.hasCurrent) return;
  recording.value = false;
  rebind.value = { row, category: group.label, changes: new Map() };
}

// Nothing records once the dialog is gone.
watch(rebind, (r) => {
  if (!r) recording.value = false;
});

interface RebindLine {
  device: string;
  kind: DeviceKind;
  text: string;
  changed: boolean;
}

// Current bindings of the action on every device (pending ones included).
const rebindBefore = computed<RebindLine[]>(() => {
  const r = rebind.value;
  if (!r) return [];
  return deviceCols.value.flatMap((col) =>
    cellTokens(r.row, col).tokens.map((t) => ({ device: col.key, kind: col.kind, text: inputText(t), changed: false })),
  );
});

// The same once the dialog's changes apply: a kind with a change shows its
// new token on its device (nothing for a clear), the other kinds stay.
const rebindAfter = computed<RebindLine[]>(() => {
  const r = rebind.value;
  if (!r) return [];
  return deviceCols.value.flatMap((col) => {
    const change = r.changes.get(col.kind);
    if (change === undefined) return rebindBefore.value.filter((b) => b.device === col.key);
    const target = parseToken(change);
    if (isBlank(change) || !target || deviceKeyOf(col.kind, target.instance) !== col.key) return [];
    return [{ device: col.key, kind: col.kind, text: inputText(change), changed: true }];
  });
});

const rebindButtons = computed<ConfirmButton[]>(() => [
  { label: "Clear all", kind: "danger", value: "clearall", side: "left", disabled: !rebindAfter.value.length },
  { label: "Apply", kind: "primary", value: "apply", disabled: !rebind.value?.changes.size },
  { label: "Cancel", kind: "outline", value: "cancel" },
]);

// Clear one kind: no binding on any of its devices.
function clearKind(kind: DeviceKind) {
  rebind.value?.changes.set(kind, blankToken(kind));
}

// Clear every kind that still has a binding on show.
function clearAll() {
  for (const kind of new Set(rebindAfter.value.map((l) => l.kind))) clearKind(kind);
}

// A recorded input becomes its kind's change.
function setCaptured(token: string | null) {
  const r = rebind.value;
  const target = token ? parseToken(token) : null;
  if (!r || !token || !target) return;
  r.changes.set(target.kind, token);
}

// What an event is: a press (buttons / keys / pad buttons on the way down,
// hats off centre, axes past half travel, at most every AXIS_MS per axis),
// a release (the way back up), or nothing to act on. Hats and axes are
// momentary: they pulse instead of staying lit.
const AXIS_MS = 150;
const lastAxis = new Map<string, number>();

type Edge = "press" | "release" | null;

function edgeOf(p: JoyInput): { edge: Edge; momentary: boolean } {
  switch (p.kind) {
    case "key":
    case "padbutton":
    case "button":
      return { edge: p.pressed ? "press" : "release", momentary: false };
    case "hat":
      return { edge: p.direction === "centered" ? null : "press", momentary: true };
    case "padaxis":
      return { edge: Math.abs(p.value) >= AXIS_PRESS && axisDue(`${p.guid}#${p.name}`) ? "press" : null, momentary: true };
    case "axis":
      return { edge: Math.abs(p.value) >= AXIS_PRESS && axisDue(`${p.guid}#${p.index}`) ? "press" : null, momentary: true };
  }
}

function axisDue(key: string): boolean {
  const now = Date.now();
  if (now - (lastAxis.get(key) ?? 0) < AXIS_MS) return false;
  lastAxis.set(key, now);
  return true;
}

// The SC token of an event. Joystick inputs take their jsN from the file
// (the backend resolves them), keyboard and gamepad tokens come with the
// held modifiers folded in.
async function tokenOf(p: JoyInput): Promise<string | null> {
  if (p.kind === "key" || p.kind === "padbutton" || p.kind === "padaxis") return props.inputToken(p);
  try {
    const res = await invoke<{ token: string | null; actions: BoundAction[] }>("resolve_input", {
      guid: p.guid,
      kind: p.kind,
      index: p.index,
      direction: p.kind === "hat" ? p.direction : null,
    });
    return res.token;
  } catch {
    return null;
  }
}

// A press lights its rows in the list and, while the dialog is recording,
// becomes its change (and ends the recording); a release puts the light
// out. Escape stops a recording, else cancels the dialog (so it cannot be
// bound here).
async function takeInput(p: JoyInput) {
  if (p.kind === "key" && p.pressed && p.name === "escape" && rebind.value) {
    if (recording.value) recording.value = false;
    else rebind.value = null;
    return;
  }
  const { edge, momentary } = edgeOf(p);
  if (!edge) return;
  const token = await tokenOf(p);
  if (!token) return;
  if (edge === "release") {
    unlight(token);
    return;
  }
  light(token, momentary);
  if (rebind.value && recording.value) {
    setCaptured(token);
    recording.value = false;
  }
}

function onRebindChoose(value: string) {
  const r = rebind.value;
  if (!r) return;
  if (value === "clearall") {
    clearAll();
    return;
  }
  rebind.value = null;
  if (value !== "apply") return;
  const { actionmap, action } = r.row;
  for (const [kind, input] of r.changes) {
    const key = pendingKey(actionmap, action, kind);
    // Back to what the file has: no change to keep.
    const inFile = props.bindings.filter((b) => b.actionmap === actionmap && b.action === action && b.device_kind === kind);
    const same = isBlank(input) ? inFile.length === 0 : inFile.length === 1 && inFile[0].token === input;
    if (same) pending.value.delete(key);
    else pending.value.set(key, { actionmap, action, kind, input });
  }
}

// --- save / discard --------------------------------------------------------

function changesText(): string {
  const n = pending.value.size;
  return `${n} change${n === 1 ? "" : "s"}`;
}

// Write the pending rebinds into the file (auto-backup first, backend side).
async function writeChanges(): Promise<boolean> {
  busy.value = true;
  try {
    const s = await invoke<LoadStatus>("save_rebinds", { changes: [...pending.value.values()] });
    pending.value.clear();
    emit("saved", s);
    await Promise.all([loadBackups(), loadInfo()]);
    ensureKeys();
    return true;
  } catch (e) {
    emit("notify", String(e), "error");
    return false;
  } finally {
    busy.value = false;
  }
}

async function saveChanges() {
  const choice = await ask(`Save ${changesText()}?`, "save", [
    { label: "Save", kind: "primary", value: "save" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice === "save") await writeChanges();
}

async function discardChanges() {
  const choice = await ask(`Discard ${changesText()}?`, "trash", [
    { label: "Discard", kind: "danger", value: "discard" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice === "discard") pending.value.clear();
}

// True when it is fine to leave the mode: nothing pending, or the user chose
// Discard, or the save went through.
async function requestLeave(): Promise<boolean> {
  if (!dirty.value) return true;
  const choice = await ask("Unsaved changes", "save", [
    { label: "Discard", kind: "danger", value: "discard" },
    { label: "Save", kind: "primary", value: "save" },
    { label: "Keep editing", kind: "outline", value: "keep" },
  ]);
  if (choice === "keep") return false;
  if (choice === "save") return await writeChanges();
  pending.value.clear();
  return true;
}

defineExpose({ requestLeave });

// --- wiring ----------------------------------------------------------------

watch([aKey, bKey], runCompare);
watch(() => props.hasCurrent, ensureKeys);
// The file changed underneath (reload, restore, resort, save): re-read its facts.
watch(() => props.bindings, loadInfo);
watch(
  () => props.keyInput,
  (p) => {
    if (p) takeInput(p);
  },
);

let unlisten: UnlistenFn[] = [];

onUnmounted(() => {
  unlisten.forEach((fn) => fn());
  unlisten = [];
  recording.value = false;
});

// The live bindings changed (reload, restore, resort) — re-diff if a side is Current.
watch(
  () => props.bindings,
  () => {
    if (aKey.value === CURRENT || bKey.value === CURRENT) runCompare();
  },
);

onMounted(async () => {
  unlisten.push(await listen<JoyInput>("joy-input", (e) => takeInput(e.payload)));
  await Promise.all([loadProfiles(), loadBackups(), loadInfo()]);
  // B starts on the newest layout, so Compare says something when opened.
  const newest = [...profiles.value].sort((a, b) => b.modified - a.modified)[0];
  if (newest) bKey.value = `${PROFILE_PREFIX}${newest.file}`;
  else await runCompare();
});

// Left-hand rows: Current shows the list, anything else compares against it.
function showList() {
  view.value = "list";
}

function compareWith(key: string) {
  bKey.value = key;
  view.value = "compare";
}
</script>

<template>
  <div class="tools">
    <div class="left">
      <!-- game bindings: the live file -->
      <section class="panel">
        <div class="head">
          <Icon name="list" :size="15" />
          <span class="head-title">Game Bindings</span>
        </div>
        <div class="rows">
          <div
            v-if="hasCurrent"
            class="row-item"
            :class="{ a: view === 'compare' && aKey === CURRENT, b: view === 'list' || bKey === CURRENT }"
            @click="showList"
          >
            <div class="lines">
              <span class="line-title">Current</span>
              <span class="mono line-sub">actionmaps.xml · {{ info ? stamp(info.modified) : "—" }}</span>
            </div>
          </div>
          <div v-else class="row-none">None</div>
        </div>
      </section>

      <!-- binding profiles -->
      <section class="panel">
        <div class="head">
          <Icon name="file" :size="15" />
          <span class="head-title">Binding Profiles</span>
        </div>
        <div class="rows">
          <div
            v-for="m in profiles"
            :key="m.file"
            class="row-item"
            :class="{ a: view === 'compare' && aKey === `${PROFILE_PREFIX}${m.file}`, b: view === 'compare' && bKey === `${PROFILE_PREFIX}${m.file}` }"
            @click="compareWith(`${PROFILE_PREFIX}${m.file}`)"
          >
            <div class="lines">
              <span class="line-title">{{ m.name }}</span>
              <span class="mono line-sub">{{ m.file }} · {{ stamp(m.modified) }}</span>
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
            :class="{ a: view === 'compare' && aKey === `backup:${b.id}`, b: view === 'compare' && bKey === `backup:${b.id}` }"
            @click="compareWith(`backup:${b.id}`)"
          >
            <div class="lines">
              <span class="mono line-stamp">{{ stamp(b.created) }}</span>
              <span class="line-sub">{{ b.reason }} · <span class="mono">{{ b.game_version ?? "—" }}</span></span>
            </div>
            <button type="button" class="icon-btn" title="Open folder" @click.stop="openBackupDir(b)">
              <Icon name="folder" :size="14" />
            </button>
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

    <div class="right">
    <!-- the live file: facts and the pending rebinds -->
    <div v-if="view === 'list'" class="action-tile">
      <div class="tile-name">
        <Icon name="file" :size="14" />
        <span class="name-text">Game Bindings</span>
      </div>
      <div class="tile-btns">
        <span class="mono tile-facts">{{ countSummary }}</span>
        <div class="spacer" />
        <span v-if="dirty" class="tile-dirty">{{ changesText() }}</span>
        <button type="button" class="btn danger small" :disabled="!dirty || busy" @click="discardChanges">
          <Icon name="close" :size="14" />
          Discard
        </button>
        <button type="button" class="btn primary small" :disabled="!dirty || busy" @click="saveChanges">
          <Icon name="save" :size="14" />
          Save
        </button>
      </div>
    </div>

    <!-- bindings list: the game's keybinding screen, every device at once -->
    <section
      v-if="view === 'list'"
      class="panel compare"
      :style="{ '--cols': listCols.template.value, '--cols-min': `${listCols.minWidth.value}px` }"
    >
      <div class="head compare-head">
        <Icon name="bindings" :size="16" />
        <span class="head-title no-grow">Bindings List</span>
        <div class="divider" />
        <button type="button" class="btn outline small square" title="Expand all" :disabled="allExpanded" @click="expandAll">
          <Icon name="unfold" :size="14" />
        </button>
        <button type="button" class="btn outline small square" title="Collapse all" :disabled="!expanded.size" @click="collapseAll">
          <Icon name="fold" :size="14" />
        </button>
        <span class="head-hint">
          <Icon name="bolt" :size="13" />
          Press an input to highlight its bindings
        </span>
        <div class="spacer" />
        <div class="chips">
          <button
            v-for="c in deviceCols"
            :key="c.key"
            type="button"
            class="chip mono"
            :class="{ active: !hiddenCols.includes(c.key) }"
            :title="c.label"
            @click="toggleCol(c.key)"
          >
            {{ c.key }}
          </button>
        </div>
        <div class="divider" />
        <div class="search">
          <Icon name="search" :size="14" />
          <input v-model="listSearch" placeholder="Find…" />
        </div>
      </div>

      <div class="table">
        <ColumnHead
          :columns="listColumns"
          :sort="listCols.sort.value"
          @sort="() => {}"
          @resize="listCols.startResize"
          @reset="listCols.resetWidth"
        />
        <template v-for="g in shownGroups" :key="g.key">
          <div class="group-row" :class="{ live: liveOn && g.rows.some(isLive) }" @click="toggleGroup(g.key)">
            <Icon :name="isOpen(g) ? 'chevron-down' : 'chevron-right'" :size="14" />
            <span class="group-label">{{ g.label }}</span>
          </div>
          <template v-if="isOpen(g)">
            <div
              v-for="r in g.rows"
              :key="r.action"
              class="row list-row"
              :class="{ live: liveOn && isLive(r) }"
              @dblclick="openRebind(r, g)"
            >
              <span class="action-cell">
                <button type="button" class="icon-btn framed" title="Set binding" @click.stop="openRebind(r, g)" @dblclick.stop>
                  <Icon name="target" :size="12" />
                </button>
                <span class="action-label" :title="r.action">{{ r.label }}</span>
              </span>
              <span
                v-for="c in visibleCols"
                :key="c.key"
                class="bind-cell"
                :class="{ pending: cellTokens(r, c).pending, empty: !bindText(r, c), live: liveOn && isLiveCell(r, c) }"
                :title="cellTokens(r, c).tokens.join(', ')"
                @dblclick.stop="openRebind(r, g)"
              ><Icon name="bolt" :size="12" class="live-mark" />{{ bindText(r, c) || "—" }}</span>
            </div>
          </template>
        </template>
        <div v-if="!shownGroups.length" class="empty-line">{{ groups.length ? "No matches" : "No game data" }}</div>
      </div>
    </section>

    <!-- compare -->
    <section v-else class="panel compare" :style="{ '--cols': cols.template.value, '--cols-min': `${cols.minWidth.value}px` }">
      <div class="head compare-head">
        <Icon name="compare" :size="16" />
        <span class="head-title no-grow">Compare</span>
        <span class="head-count">{{ report?.rows.length ?? 0 }}</span>
        <Dropdown v-model="aKey" :options="sourceDropdown" title="Source A" />
        <Icon name="arrow-right" :size="18" class="dim" />
        <Dropdown v-model="bKey" :options="sourceDropdown" title="Source B" />
        <div class="spacer" />
        <div class="chips">
          <button
            type="button"
            class="chip added"
            :class="{ active: kindFilter.includes('added') }"
            @click="kindFilter = toggleIn(kindFilter, 'added')"
          >
            +{{ report?.added ?? 0 }}
          </button>
          <button
            type="button"
            class="chip removed"
            :class="{ active: kindFilter.includes('removed') }"
            @click="kindFilter = toggleIn(kindFilter, 'removed')"
          >
            −{{ report?.removed ?? 0 }}
          </button>
          <button
            type="button"
            class="chip changed"
            :class="{ active: kindFilter.includes('changed') }"
            @click="kindFilter = toggleIn(kindFilter, 'changed')"
          >
            ~{{ report?.changed ?? 0 }}
          </button>
        </div>
        <div v-if="deviceCounts.length" class="divider" />
        <div v-if="deviceCounts.length" class="chips">
          <button
            v-for="[label, d] in deviceCounts"
            :key="label"
            type="button"
            class="chip mono"
            :class="{ active: deviceFilter.includes(label) }"
            @click="deviceFilter = toggleIn(deviceFilter, label)"
          >
            {{ label }} <span class="count">{{ d.count }}</span>
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
          <span class="input-cell dim" :title="r.token">
            <span class="mono">{{ deviceLabel(r) }}</span>
            <span :class="{ mono: inputText(r.token) === inputPart(r.token) }">{{ inputText(r.token) }}</span>
          </span>
          <span>{{ rowAction(r) }}</span>
          <span :class="r.a.length ? 'side' : 'empty'">{{ cellText(r.a) }}</span>
          <span :class="r.b.length ? 'side' : 'empty'">{{ cellText(r.b) }}</span>
        </div>
        <div v-if="!filteredRows.length" class="empty-line">
          {{ sameSource ? "Same source" : "No differences" }}
        </div>
      </div>
    </section>
    </div>

    <ConfirmDialog v-if="confirm" :title="confirm.title" :icon="confirm.icon" :buttons="confirm.buttons" @choose="onConfirm" />

    <!-- rebind: every device at once; Record or Clear changes a kind, Apply queues the changes -->
    <ConfirmDialog
      v-if="rebind"
      :title="rebind.row.label"
      :subtitle="rebind.category"
      icon="target"
      :buttons="rebindButtons"
      captureKeys
      @choose="onRebindChoose"
    >
      <div class="rb-columns">
        <div class="rb-block">
          <span class="rb-label">Before</span>
          <div v-for="b in rebindBefore" :key="`${b.device}:${b.text}`" class="rb-line">
            <span class="mono dim">{{ b.device }}</span>
            <span class="rb-text">{{ b.text }}</span>
            <button type="button" class="icon-btn rb-clear" title="Clear" @click="clearKind(b.kind)">
              <Icon name="close" :size="12" />
            </button>
          </div>
          <div v-if="!rebindBefore.length" class="rb-line dim">—</div>
        </div>
        <Icon name="arrow-right" :size="18" class="dim" />
        <div class="rb-block">
          <span class="rb-label">After</span>
          <div v-for="a in rebindAfter" :key="`${a.device}:${a.text}`" class="rb-line">
            <span class="mono dim">{{ a.device }}</span>
            <span class="rb-text" :class="{ 'rb-new': a.changed }">{{ a.text }}</span>
          </div>
          <div v-if="!rebindAfter.length" class="rb-line dim">—</div>
        </div>
      </div>
      <div class="rb-record">
        <button type="button" class="btn small" :class="recording ? 'primary' : 'outline'" @click="recording = true">
          <Icon name="target" :size="13" />
          {{ recording ? "Recording…" : "Record" }}
        </button>
        <span class="rb-hint" :class="{ on: recording }">
          {{ recording ? "Press an input on any device · Esc stops" : "Record input from any device" }}
        </span>
      </div>
    </ConfirmDialog>
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

.left,
.right {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-height: 0;
}

.right {
  min-width: 0;
}

/* --- action tile (mirrors the Devices mode) --- */

.action-tile {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 16px 16px;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
}

.tile-name {
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-chip-sm);
  min-width: 0;
  color: var(--text);
}

.name-text {
  font-weight: 600;
  font-size: 14px;
}

.tile-facts {
  font-size: 12px;
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tile-btns {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tile-dirty {
  font-size: 12px;
  font-weight: 600;
  color: var(--warn);
  white-space: nowrap;
}

.btn.danger {
  background: transparent;
  color: var(--err);
  border: 1px solid color-mix(in srgb, var(--err) 60%, transparent);
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

/* Row lists styled like the Devices mode's image-map rows: outlined items
   with a little air between them. */
.rows {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
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
  padding: 10px 12px;
  border-radius: 6px;
  border: 1px solid var(--border);
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

/* Device and input side by side in the one INPUT cell. */
.input-cell {
  display: flex;
  align-items: center;
  gap: 8px;
}

.input-cell .mono {
  flex-shrink: 0;
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

/* --- bindings list --- */

.group-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  min-width: var(--cols-min);
  font-weight: 600;
  cursor: pointer;
  user-select: none;
  border-bottom: 1px solid var(--border-dim);
}

.group-row:hover {
  background: var(--bg-surface-2);
}

.group-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.list-row {
  cursor: pointer;
  user-select: none;
}

.list-row:hover {
  background: var(--bg-surface-2);
}

.list-row span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.action-cell {
  display: flex;
  align-items: center;
  gap: 16px;
  padding-left: 4px;
}

/* A small outlined square, like the outline buttons but row-sized. */
.icon-btn.framed {
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  color: var(--text-2);
}

.icon-btn.framed:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.action-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bind-cell.empty {
  color: var(--text-3);
}

.bind-cell.pending {
  color: var(--warn);
  font-weight: 600;
}

/* The last press: a live-coloured left edge on the row and its category,
   the matching cell in live colour with a bolt (always in the layout,
   invisible until lit). Lights up at once, fades out over the pulse time. */
.group-row,
.list-row {
  border-left: 3px solid transparent;
  transition: border-left-color 600ms ease-out;
}

.group-row.live,
.list-row.live {
  border-left-color: var(--live);
  transition: none;
}

.bind-cell {
  transition: color 600ms ease-out;
}

.bind-cell.live {
  color: var(--live);
  transition: none;
}

.live-mark {
  vertical-align: -2px;
  margin-right: 6px;
  opacity: 0;
  transition: opacity 600ms ease-out;
}

.bind-cell.live .live-mark {
  opacity: 1;
  transition: none;
}

.head-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: 4px;
  font-size: 12px;
  color: var(--text-3);
  white-space: nowrap;
}

/* --- rebind dialog --- */

/* Before and After side by side, an arrow between them. */
.rb-columns {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  gap: 12px;
  align-items: center;
}

.rb-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
  align-self: stretch;
  padding: 10px 12px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
}

.rb-label {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-3);
}

.rb-line {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 14px;
}

.rb-line .mono {
  flex-shrink: 0;
  min-width: 3ch;
}

.rb-clear {
  margin-left: auto;
}

.rb-text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rb-new {
  color: var(--warn);
  font-weight: 600;
}

.rb-record {
  display: flex;
  align-items: center;
  gap: 12px;
}

.rb-hint {
  color: var(--text-2);
  font-size: 13px;
}

.rb-hint.on {
  color: var(--accent);
}
</style>
