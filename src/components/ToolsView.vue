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

// Toggled chips; none toggled = everything.
const kindFilter = ref<DiffKind[]>([]);
const deviceFilter = ref<string[]>([]);

function toggleIn<T>(list: T[], item: T): T[] {
  return list.includes(item) ? list.filter((x) => x !== item) : [...list, item];
}
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
      return deviceRank(r);
    case "input":
      return inputText(r.token);
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

// "12.3 KB"
function fmtSize(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`;
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
  { key: "kb1", kind: "keyboard", instance: 1, label: "kb1" },
  { key: "gp1", kind: "gamepad", instance: 1, label: "gp1" },
  ...[...(info.value?.joysticks ?? [])]
    .sort((a, b) => a.instance - b.instance)
    .map((j) => ({
      key: `js${j.instance}`,
      kind: "joystick" as DeviceKind,
      instance: j.instance,
      label: `js${j.instance} · ${j.product_name}`,
    })),
]);

// The list is in the game's order and not sortable; the last device column
// is the filler.
const listColumns = computed<ColumnSpec[]>(() => [
  { key: "action", label: "ACTION", width: 320, sortable: false },
  ...deviceCols.value.map((d, i, all) => ({
    key: d.key,
    label: d.label.toUpperCase(),
    width: i === all.length - 1 ? null : 200,
    sortable: false,
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

const rowCount = computed(() => groups.value.reduce((n, g) => n + g.rows.length, 0));

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
        [r.label, r.action, g.label, ...deviceCols.value.map((c) => bindText(r, c))].join(" ").toLowerCase().includes(q),
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

// --- rebind dialog ---------------------------------------------------------

// Past half travel an axis counts as pressed.
const AXIS_PRESS = 16384;

interface RebindState {
  row: ListRow;
  category: string;
  // The captured input, once one arrived.
  after: { token: string; kind: DeviceKind; instance: number } | null;
}

const rebind = ref<RebindState | null>(null);

function openRebind(row: ListRow, group: ListGroup) {
  if (!props.hasCurrent) return;
  rebind.value = { row, category: group.label, after: null };
}

// Current bindings of the action (pending ones included): every device
// until an input arrived, then only the kind that input replaces.
const rebindBefore = computed(() => {
  const r = rebind.value;
  if (!r) return [];
  return deviceCols.value.flatMap((col) => {
    if (r.after && col.kind !== r.after.kind) return [];
    return cellTokens(r.row, col).tokens.map((t) => ({ device: col.key, text: inputText(t) }));
  });
});

const rebindAfter = computed(() => {
  const a = rebind.value?.after;
  return a ? { device: deviceKeyOf(a.kind, a.instance), text: inputText(a.token) } : null;
});

// One Unbind per device that has a binding in the Before list (one per kind,
// so the device names the kind), then Confirm / Cancel.
const rebindButtons = computed<ConfirmButton[]>(() => {
  const unbinds = new Map<DeviceKind, string>();
  for (const b of rebindBefore.value) {
    const kind = parseToken(`${b.device}_`)?.kind;
    if (kind && !unbinds.has(kind)) unbinds.set(kind, b.device);
  }
  return [
    ...[...unbinds].map(([kind, device]) => ({ label: `Unbind ${device}`, kind: "danger" as const, value: `unbind:${kind}` })),
    { label: "Confirm", kind: "primary", value: "confirm", disabled: !rebind.value?.after },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ];
});

function setAfter(token: string | null) {
  const r = rebind.value;
  const target = token ? parseToken(token) : null;
  if (!r || !token || !target) return;
  r.after = { token, ...target };
}

// A press while the dialog is open becomes the new binding: buttons and keys
// on the way down, hats off centre, axes past half travel. Joystick inputs
// take their jsN from the file (the backend resolves them), keyboard and
// gamepad tokens come with the held modifiers folded in. Escape cancels
// the dialog instead (so it cannot be bound here).
async function takeInput(p: JoyInput) {
  if (!rebind.value) return;
  switch (p.kind) {
    case "key":
      if (!p.pressed) return;
      if (p.name === "escape") rebind.value = null;
      else setAfter(props.inputToken(p));
      return;
    case "padbutton":
      if (p.pressed) setAfter(props.inputToken(p));
      return;
    case "padaxis":
      if (Math.abs(p.value) >= AXIS_PRESS) setAfter(props.inputToken(p));
      return;
    case "button":
      if (!p.pressed) return;
      break;
    case "hat":
      if (p.direction === "centered") return;
      break;
    case "axis":
      if (Math.abs(p.value) < AXIS_PRESS) return;
      break;
  }
  try {
    const res = await invoke<{ token: string | null; actions: BoundAction[] }>("resolve_input", {
      guid: p.guid,
      kind: p.kind,
      index: p.index,
      direction: p.kind === "hat" ? p.direction : null,
    });
    setAfter(res.token);
  } catch {
    /* ignore transient resolve errors */
  }
}

function onRebindChoose(value: string) {
  const r = rebind.value;
  rebind.value = null;
  if (!r) return;
  let change: { kind: DeviceKind; input: string } | null = null;
  if (value === "confirm" && r.after) change = { kind: r.after.kind, input: r.after.token };
  const unbind = value.startsWith("unbind:") ? (value.slice(7) as DeviceKind) : null;
  if (unbind) change = { kind: unbind, input: blankToken(unbind) };
  if (!change) return;
  const { actionmap, action } = r.row;
  const key = pendingKey(actionmap, action, change.kind);
  // Back to what the file has: no change to keep.
  const inFile = props.bindings.filter((b) => b.actionmap === actionmap && b.action === action && b.device_kind === change!.kind);
  const same = unbind ? inFile.length === 0 : inFile.length === 1 && inFile[0].token === change.input;
  if (same) {
    pending.value.delete(key);
    return;
  }
  pending.value.set(key, { actionmap, action, kind: change.kind, input: change.input });
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
      <!-- game bindings: SC's action master list -->
      <section class="panel game">
        <div class="head">
          <Icon name="list" :size="15" />
          <span class="head-title">Game Bindings</span>
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
          <span class="head-title">Binding Profiles</span>
        </div>
        <div class="rows">
          <div
            v-if="hasCurrent"
            class="row-item"
            :class="{ a: view === 'compare' && aKey === CURRENT, b: view === 'list' || bKey === CURRENT }"
            @click="showList"
          >
            <span class="dot" />
            <div class="lines">
              <span class="line-title">Current</span>
              <span class="mono line-sub">{{ bindings.length }} bindings · {{ info ? stamp(info.modified) : "—" }}</span>
            </div>
          </div>
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
          <div v-if="!profiles.length && !hasCurrent" class="row-none">None</div>
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
        <span class="name-text">Current</span>
        <span v-if="info" class="tile-facts">
          {{ stamp(info.modified) }} · {{ fmtSize(info.size) }} · {{ info.rebinds }} rebinds · {{ info.joysticks.length }} joysticks
        </span>
      </div>
      <div class="tile-btns">
        <span class="mono tile-path" :title="info?.path">{{ info?.path ?? "—" }}</span>
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
        <span class="head-count">{{ rowCount }}</span>
        <div class="spacer" />
        <button type="button" class="btn outline small" :disabled="allExpanded" @click="expandAll">
          <Icon name="chevron-down" :size="14" />Expand all
        </button>
        <button type="button" class="btn outline small" :disabled="!expanded.size" @click="collapseAll">
          <Icon name="chevron-up" :size="14" />Collapse all
        </button>
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
          <div class="group-row" @click="toggleGroup(g.key)">
            <Icon :name="isOpen(g) ? 'chevron-down' : 'chevron-right'" :size="14" />
            <span class="group-label">{{ g.label }}</span>
            <span class="head-count">{{ g.rows.length }}</span>
          </div>
          <template v-if="isOpen(g)">
            <div v-for="r in g.rows" :key="r.action" class="row list-row" @dblclick="openRebind(r, g)">
              <span class="action-cell" :title="r.action">{{ r.label }}</span>
              <span
                v-for="c in deviceCols"
                :key="c.key"
                class="bind-cell"
                :class="{ pending: cellTokens(r, c).pending, empty: !bindText(r, c) }"
                :title="cellTokens(r, c).tokens.join(', ')"
              >{{ bindText(r, c) || "—" }}</span>
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
          <span class="mono dim">{{ deviceLabel(r) }}</span>
          <span class="dim" :class="{ mono: inputText(r.token) === inputPart(r.token) }" :title="r.token">{{ inputText(r.token) }}</span>
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

    <!-- rebind: the next input pressed becomes the action's binding -->
    <ConfirmDialog v-if="rebind" :title="rebind.row.label" icon="edit" :buttons="rebindButtons" captureKeys @choose="onRebindChoose">
      <div class="rb-category">{{ rebind.category }}</div>
      <div class="rb-block">
        <span class="rb-label">Before</span>
        <div v-for="b in rebindBefore" :key="`${b.device}:${b.text}`" class="rb-line">
          <span class="mono dim">{{ b.device }}</span>
          <span>{{ b.text }}</span>
        </div>
        <div v-if="!rebindBefore.length" class="rb-line dim">—</div>
      </div>
      <div class="rb-block">
        <span class="rb-label">After</span>
        <div v-if="rebindAfter" class="rb-line">
          <span class="mono dim">{{ rebindAfter.device }}</span>
          <span class="rb-new">{{ rebindAfter.text }}</span>
        </div>
        <div v-else class="rb-line dim">Press an input…</div>
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

.tile-path {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
  cursor: default;
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
  padding-left: 22px;
}

.bind-cell.empty {
  color: var(--text-3);
}

.bind-cell.pending {
  color: var(--warn);
  font-weight: 600;
}

/* --- rebind dialog --- */

.rb-category {
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  color: var(--text-2);
}

.rb-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
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
  gap: 12px;
  font-size: 14px;
}

.rb-line .mono {
  width: 40px;
  flex-shrink: 0;
}

.rb-new {
  color: var(--warn);
  font-weight: 600;
}
</style>
