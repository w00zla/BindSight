<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import ColumnHead from "./ColumnHead.vue";
import Splitter from "./Splitter.vue";
import { collator, sortRows, useTableColumns, type ColumnSpec } from "../tableColumns";
import ConfirmDialog, { type ConfirmButton, type ConfirmIcon } from "./ConfirmDialog.vue";
import { AXIS_PRESS, KIND_RANK, inputIdentity, kindIcon, recordEdge } from "../devices";
import Dropdown, { type DropdownOption } from "./Dropdown.vue";
import ConsoleCommandDialog from "./ConsoleCommandDialog.vue";
import InputOverlay from "./InputOverlay.vue";
import { persistedRef } from "../persist";
import { recording } from "../keyboard";
import { NAME_MAX, sanitizeName, stripNameChars } from "../names";
import { inputKeysForToken, type OverlayTarget } from "../imagemap";
import type {
  ActionMap,
  ActionRef,
  BackupSummary,
  BindingProfileSummary,
  BoundAction,
  ClashReport,
  DeviceKind,
  DeviceSel,
  DiffKind,
  DiffReport,
  DiffRow,
  DiffSource,
  JoyInput,
  LoadStatus,
  CurrentBindingsInfo,
  OverlayPosition,
  RebindChange,
  ResolvedBinding,
} from "../types";

// `hasCurrent`: the live actionmaps.xml is loaded (else there is no Current
// source and no list). Keys never reach the backend: App hands them to
// `takeInput` directly. `inputToken`: the full SC token a keyboard / gamepad press
// stands for, modifiers included; joystick inputs are resolved by the backend.
const props = defineProps<{
  bindings: ResolvedBinding[];
  actionMaps: ActionMap[];
  // The device-order report: names the joystick the game ranks at each jsN.
  clash: ClashReport | null;
  // Whether the joystick the game ranks at this jsN is one SDL does not
  // list (a `LogOnlyJoystick`): no input reaches the app from it.
  isLogOnly: (instance: number) => boolean;
  hasCurrent: boolean;
  // SC's label for an input token; echoes the token when there is none.
  tokenLabel: (token: string) => string;
  inputToken: (p: JoyInput) => string | null;
  // The input-preview overlay: resolve a device slot to its chosen map, and
  // the overlay's persisted size (px) and placement.
  overlayFor: (kind: DeviceKind, instance: number) => OverlayTarget | null;
  overlaySize: number;
  overlayPosition: OverlayPosition;
}>();
const emit = defineEmits<{
  notify: [message: string, type: "ok" | "error"];
  restored: [status: LoadStatus];
  applied: [status: LoadStatus];
  saved: [status: LoadStatus];
  copy: [command: string];
}>();

// --- reorder: swap two joystick slots ----------------------------------------

// The user's own resort, one swap at a time: the two slots picked from the
// joysticks the game ranks right now, applied either to the file (the order
// fix's rewrite, `apply_reorder`) or as the console command.
const reorderOpen = ref(false);
const reorderA = ref("");
const reorderB = ref("");
const reorderCommand = ref<string | null>(null);
const reorderOptions = computed<DropdownOption[]>(() =>
  (props.clash?.connected ?? []).map((s) => ({ value: String(s.effective_instance), label: `js${s.effective_instance} · ${s.name ?? "?"}` })),
);
const reorderValid = computed(() => !!reorderA.value && !!reorderB.value && reorderA.value !== reorderB.value);

function openReorder() {
  const o = reorderOptions.value;
  reorderA.value = o[0]?.value ?? "";
  reorderB.value = o[1]?.value ?? "";
  reorderOpen.value = true;
}

async function onReorderChoose(value: string) {
  reorderOpen.value = false;
  if (!reorderValid.value) return;
  const a = Number(reorderA.value);
  const b = Number(reorderB.value);
  if (value === "console") {
    reorderCommand.value = `pp_resortdevices joystick ${a} ${b}`;
    return;
  }
  if (value !== "config") return;
  busy.value = true;
  try {
    emit("applied", await invoke<LoadStatus>("apply_reorder", { a, b }));
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// --- layout: the left column's width and the Binding Profiles panel's
// height, both dragged at a splitter and remembered --------------------------

const LEFT_W = { min: 280, max: 640, def: 380 };
const leftWidth = persistedRef<number>("bindsight.bindings.leftWidth", LEFT_W.def);
let leftStart: number | null = null;
function dragLeft(delta: number) {
  leftStart ??= leftWidth.value;
  leftWidth.value = Math.min(LEFT_W.max, Math.max(LEFT_W.min, Math.round(leftStart + delta)));
}
const PROFILES_H = { min: 120, max: 700, def: 260 };
const profilesHeight = persistedRef<number>("bindsight.bindings.profilesHeight", PROFILES_H.def);
let profilesStart: number | null = null;
function dragProfiles(delta: number) {
  profilesStart ??= profilesHeight.value;
  profilesHeight.value = Math.min(PROFILES_H.max, Math.max(PROFILES_H.min, Math.round(profilesStart + delta)));
}
function endDrag() {
  leftStart = null;
  profilesStart = null;
}

const profiles = ref<BindingProfileSummary[]>([]);
const backups = ref<BackupSummary[]>([]);
const report = ref<DiffReport | null>(null);
// A command is running; the action buttons stay out of the way until it is done.
const busy = ref(false);

// The right-hand tile: the bindings list (Current) or Compare (any other
// source picked on the left).
const view = ref<"list" | "compare">("list");

// --- confirm dialog --------------------------------------------------------

const confirm = ref<{ title: string; icon: ConfirmIcon; buttons: ConfirmButton[]; subtitle?: string } | null>(null);
let confirmResolve: ((value: string) => void) | null = null;

// Every write into the game's bindings file needs a game restart to show.
const RESTART_NOTE = "If game is running, restart for changes to take effect";

function ask(title: string, icon: ConfirmIcon, buttons: ConfirmButton[], subtitle?: string): Promise<string> {
  confirm.value = { title, icon, buttons, subtitle };
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

// A source is addressed by a flat key: "current", "profile:<file>" or
// "backup:<id>". Compare always sets the source picked on the left against
// Current, seen from Current: the backend's A is the source, its B is
// Current, so "added" means the source adds it, "removed" that Current has
// it and the source does not.
const CURRENT = "current";
const PROFILE_PREFIX = "profile:";
const BACKUP_PREFIX = "backup:";

const bKey = ref(CURRENT);

interface SourceOption {
  key: string;
  name: string;
}

const sourceOptions = computed<SourceOption[]>(() => [
  ...(props.hasCurrent ? [{ key: CURRENT, name: "Current" }] : []),
  ...profiles.value.map((m) => ({ key: `${PROFILE_PREFIX}${m.file}`, name: m.name })),
  ...backups.value.map((b) => ({ key: `${BACKUP_PREFIX}${b.id}`, name: `${stamp(b.created)} · ${b.reason}` })),
]);

function sourceFor(key: string): DiffSource {
  if (key.startsWith(PROFILE_PREFIX)) return { kind: "profile", file: key.slice(PROFILE_PREFIX.length) };
  if (key.startsWith(BACKUP_PREFIX)) return { kind: "backup", id: key.slice(BACKUP_PREFIX.length) };
  return { kind: "current" };
}

function nameFor(key: string): string {
  return sourceOptions.value.find((o) => o.key === key)?.name ?? "—";
}

// The picked source as a profile / a backup (null when it is the other).
const bProfile = computed<BindingProfileSummary | null>(() => {
  const s = sourceFor(bKey.value);
  return s.kind === "profile" ? profiles.value.find((m) => m.file === s.file) ?? null : null;
});
const bBackup = computed<BackupSummary | null>(() => {
  const s = sourceFor(bKey.value);
  return s.kind === "backup" ? backups.value.find((b) => b.id === s.id) ?? null : null;
});

// A source that vanished (deleted backup, reloaded list, no Current) falls
// back to Current, else to the first source there is.
function ensureKeys() {
  const keys = sourceOptions.value.map((o) => o.key);
  if (!keys.includes(bKey.value)) {
    bKey.value = CURRENT;
    view.value = "list";
  }
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
  if (bKey.value === CURRENT) {
    report.value = null;
    return;
  }
  try {
    report.value = await invoke<DiffReport>("compare_bindings", {
      a: sourceFor(bKey.value),
      b: { kind: "current" },
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

// Save the live file as a new binding profile: the name is asked in a
// small dialog (letters, digits, space, _ - and brackets, like image-maps).
const nameDialog = ref<{ name: string } | null>(null);
const nameInput = ref<HTMLInputElement | null>(null);

function openSaveProfile() {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  nameDialog.value = { name: `Bindings ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}` };
  nextTick(() => {
    nameInput.value?.focus();
    nameInput.value?.select();
  });
}

async function onNameChoose(value: string) {
  const d = nameDialog.value;
  nameDialog.value = null;
  if (!d || value !== "save") return;
  busy.value = true;
  try {
    const s = await invoke<BindingProfileSummary>("save_binding_profile", { name: sanitizeName(d.name, "") });
    await loadProfiles();
    emit("notify", `Saved ${s.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function deleteProfile(m: BindingProfileSummary) {
  const choice = await ask(`Delete ${m.name}?`, "trash", [
    { label: "Delete", kind: "danger", value: "delete" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice !== "delete") return;
  busy.value = true;
  try {
    await invoke("delete_binding_profile", { file: m.file });
    await loadProfiles();
    ensureKeys();
    await runCompare();
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

// Create Backup: the description is asked in a small dialog, "manual" by
// default (the backend keeps that for an empty one).
const backupDialog = ref<{ reason: string } | null>(null);
const backupInput = ref<HTMLInputElement | null>(null);
const BACKUP_REASON_MAX = 64;

function openCreateBackup() {
  backupDialog.value = { reason: "manual" };
  nextTick(() => {
    backupInput.value?.focus();
    backupInput.value?.select();
  });
}

async function onBackupChoose(value: string) {
  const d = backupDialog.value;
  backupDialog.value = null;
  if (!d || value !== "create") return;
  busy.value = true;
  try {
    await invoke<BackupSummary>("create_backup", { reason: d.reason.trim() || "manual" });
    await loadBackups();
    emit("notify", "Backup created", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// --- apply ----------------------------------------------------------------

// The devices a profile / backup and the live file bind, for the Apply
// dialog: kb1 and gp1 always, every jsN either side names, all ticked.
interface ApplyDevice {
  key: string;
  sel: DeviceSel;
  label: string;
}

// `targets`: the live slot a source joystick's bindings land on (the
// resort that goes with an apply), by device key; unset = the same slot.
const applyDialog = ref<{ devices: ApplyDevice[]; on: Set<string>; targets: Record<string, number> } | null>(null);

// The live file's joystick slots a source joystick can land on: its own
// slot plus every slot the live file names, ascending.
function slotOptions(instance: number): { value: string; label: string }[] {
  const slots = new Set<number>([instance, ...(info.value?.joysticks ?? []).map((j) => j.instance)]);
  return [...slots].sort((a, b) => a - b).map((n) => ({ value: String(n), label: `js${n}` }));
}

// The live slot a source joystick lands on; unset = its own.
function targetOf(d: ApplyDevice): number {
  return applyDialog.value?.targets[d.key] ?? d.sel.instance;
}

function setTarget(key: string, value: string) {
  const d = applyDialog.value;
  if (!d) return;
  applyDialog.value = { ...d, targets: { ...d.targets, [key]: Number(value) } };
}

// Ticked joysticks that would land on the same slot: marked, and Apply
// stays off until the user sorts it out.
const slotClashes = computed<Set<string>>(() => {
  const d = applyDialog.value;
  const out = new Set<string>();
  if (!d) return out;
  const sticks = d.devices.filter((x) => x.sel.kind === "joystick" && d.on.has(x.key));
  for (const a of sticks) {
    if (sticks.some((b) => b !== a && targetOf(b) === targetOf(a))) out.add(a.key);
  }
  return out;
});

function applyDeviceList(): ApplyDevice[] {
  const instances = new Set<number>();
  for (const r of report.value?.rows ?? []) {
    if (r.device_kind === "joystick" && r.instance !== null) instances.add(r.instance);
  }
  for (const j of info.value?.joysticks ?? []) instances.add(j.instance);
  // A joystick row is the source's slot; the device it lands on is the
  // live file's, named at the target slot (`slotName`).
  return [
    { key: "kb1", sel: { kind: "keyboard", instance: 1 }, label: "Keyboard/Mouse" },
    { key: "gp1", sel: { kind: "gamepad", instance: 1 }, label: "Gamepad" },
    ...[...instances].sort((a, b) => a - b).map((n) => ({ key: `js${n}`, sel: { kind: "joystick" as DeviceKind, instance: n }, label: "" })),
  ];
}

// The live file's device on a joystick slot, or nothing when it names none.
function slotName(n: number): string {
  return info.value?.joysticks.find((j) => j.instance === n)?.product_name ?? "";
}

// The source profile/backup's own device on a joystick slot (from the diff's
// A side); empty for keyboard/gamepad (SC names only one of each) or when the
// source lists no name.
function sourceName(d: ApplyDevice): string {
  if (d.sel.kind !== "joystick") return "";
  return report.value?.a_joysticks.find((j) => j.instance === d.sel.instance)?.product_name ?? "";
}

function openApply() {
  if (bKey.value === CURRENT) return;
  const devices = applyDeviceList();
  applyDialog.value = { devices, on: new Set(devices.map((d) => d.key)), targets: {} };
}

function toggleApplyDevice(key: string) {
  const d = applyDialog.value;
  if (!d) return;
  const on = new Set(d.on);
  if (!on.delete(key)) on.add(key);
  applyDialog.value = { ...d, on };
}

// A backup is "restored", a profile "applied" — same mechanics.
const applyWord = computed(() => (bBackup.value ? "Restore" : "Apply"));

const applyButtons = computed<ConfirmButton[]>(() => [
  { label: applyWord.value, kind: "primary", value: "apply", disabled: !applyDialog.value?.on.size || slotClashes.value.size > 0 },
  { label: "Cancel", kind: "outline", value: "cancel" },
]);

// A backup with every device ticked is put back byte for byte (`restore`);
// anything else is merged device by device (`apply`).
async function onApplyChoose(value: string) {
  const d = applyDialog.value;
  applyDialog.value = null;
  if (!d || value !== "apply") return;
  const source = sourceFor(bKey.value);
  const devices = d.devices
    .filter((x) => d.on.has(x.key))
    .map((x) => {
      const target = d.targets[x.key];
      return target !== undefined && target !== x.sel.instance ? { ...x.sel, target } : x.sel;
    });
  const resorted = devices.some((x) => x.target !== undefined);
  busy.value = true;
  try {
    if (source.kind === "backup" && devices.length === d.devices.length && !resorted) {
      const s = await invoke<LoadStatus>("restore_backup", { id: source.id });
      emit("restored", s);
    } else {
      const s = await invoke<LoadStatus>("apply_bindings", { source, devices });
      emit("applied", s);
    }
    // The write takes a safety backup of its own — the list has a new entry.
    await loadBackups();
    await loadInfo();
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
    emit("notify", "Backup deleted", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// --- diff filters ----------------------------------------------------------

// Toggled chips; none toggled = everything. Not remembered across restarts:
// a filter on a device the next compare lacks would show nothing. The kind
// chips live per device (`js1` -> ["changed"]).
const kindFilter = ref<Record<string, DiffKind[]>>({});
const deviceFilter = ref<string[]>([]);
// Rows grouped under one collapsible head per input (device + input, like
// the Monitor's deck), the actions inside — or flat. Grouped and open by
// default.
const grouped = persistedRef<boolean>("bindsight.compare.grouped", true);

// The kinds toggled on for a device that this compare actually has rows
// for — a toggle left from an earlier compare on a kind with no rows (its
// chip is disabled) must not filter, or nothing would show.
function kindsOf(device: string): DiffKind[] {
  const counts = deviceChips.value.find((d) => d.label === device)?.counts;
  return (kindFilter.value[device] ?? []).filter((k) => !!counts?.[k]);
}

function toggleKind(device: string, kind: DiffKind) {
  kindFilter.value = { ...kindFilter.value, [device]: toggleIn(kindsOf(device), kind) };
}

const search = ref("");

// Written out, seen from Current: the source adds it / binds it to other
// actions / lacks it. Unchanged rows are not listed.
const KIND_CHIPS: { kind: DiffKind; label: string }[] = [
  { kind: "added", label: "Added" },
  { kind: "changed", label: "Modified" },
  { kind: "removed", label: "Deleted" },
];

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

// Every row that differs; unchanged tokens only feed the counts below.
const diffRows = computed<DiffRow[]>(() => (report.value?.rows ?? []).filter((r) => r.kind !== "same"));

// Distinct devices present in the report, in tile order, each with its
// differing rows counted per kind.
interface DeviceGroup {
  label: string;
  kind: DeviceKind;
  counts: Record<DiffKind, number>;
}
const deviceChips = computed<DeviceGroup[]>(() => {
  const by = new Map<string, { rank: number; kind: DeviceKind; counts: Record<DiffKind, number> }>();
  for (const r of report.value?.rows ?? []) {
    const label = deviceLabel(r);
    const hit = by.get(label) ?? { rank: deviceRank(r), kind: r.device_kind, counts: { added: 0, removed: 0, changed: 0, same: 0 } };
    hit.counts[r.kind] += 1;
    by.set(label, hit);
  }
  return [...by.entries()].sort((a, b) => a[1].rank - b[1].rank).map(([label, d]) => ({ label, kind: d.kind, counts: d.counts }));
});

// What the compare found, for the source tile: "3 added · 1 modified · 2 deleted".
const diffSummary = computed(() => {
  const r = report.value;
  if (!r) return "";
  return [`${r.added} added`, `${r.changed} modified`, `${r.removed} deleted`].join(" · ");
});

// Bindings of the picked source, total and per device, like the Game
// Bindings tile's summary — from the report's A side (the source).
const sourceCountSummary = computed(() => {
  const counts = new Map<string, number>();
  let total = 0;
  for (const r of report.value?.rows ?? []) {
    if (!r.a.length) continue;
    const label = deviceLabel(r);
    counts.set(label, (counts.get(label) ?? 0) + r.a.length);
    total += r.a.length;
  }
  const perDevice = deviceChips.value.filter((d) => counts.has(d.label)).map((d) => `${counts.get(d.label)} ${d.label}`);
  return [`${total} total`, ...perDevice].join(" · ");
});

function refText(r: ActionRef): string {
  return r.label ?? r.action;
}

// Haystack for the search box: token plus every action name and label.
// The game's category of an actionmap (its label, else its name).
const categoryOf = computed(() => {
  const m = new Map<string, string>();
  for (const am of props.actionMaps) m.set(am.name, am.label ?? am.name);
  return m;
});

function categoryLabel(actionmap: string): string {
  return categoryOf.value.get(actionmap) ?? actionmap;
}

// The category a row is about: the A side's first action, else the B side's.
function rowCategory(row: DiffRow): string {
  const r = row.a[0] ?? row.b[0];
  return r ? categoryLabel(r.actionmap) : "—";
}

function haystack(row: DiffRow): string {
  const refs = [...row.a, ...row.b];
  return [
    row.token,
    inputText(row.token),
    ...refs.map((r) => r.action),
    ...refs.map((r) => r.label ?? ""),
    ...refs.map((r) => categoryLabel(r.actionmap)),
  ]
    .join(" ")
    .toLowerCase();
}

const COLUMNS: ColumnSpec[] = [
  { key: "sign", label: "", width: 28 },
  { key: "input", label: "INPUT", width: 220, icon: "bolt" },
  { key: "action", label: "ACTION", width: 300, icon: "target" },
  { key: "category", label: "CATEGORY", width: 200, icon: "list" },
  { key: "a", label: "A", width: 300, icon: "file" },
  { key: "b", label: "B", width: null, icon: "file" },
];
const cols = useTableColumns("bindsight.columns.compare", COLUMNS, { key: "action", dir: "asc" });
// A is always Current; the B header carries the source's name.
const columns = computed<ColumnSpec[]>(() =>
  COLUMNS.map((c) => (c.key === "a" ? { ...c, label: "CURRENT" } : c.key === "b" ? { ...c, label: nameFor(bKey.value).toUpperCase() } : c)),
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
    case "category":
      return rowCategory(r);
    // Column "a" is CURRENT = the report's B side; column "b" the source = A.
    case "a":
      return cellLines(r.b).join(", ");
    default:
      return cellLines(r.a).join(", ");
  }
}

const filteredRows = computed<DiffRow[]>(() => {
  const q = search.value.trim().toLowerCase();
  const rows = diffRows.value.filter((r) => {
    const device = deviceLabel(r);
    if (deviceFilter.value.length && !deviceFilter.value.includes(device)) return false;
    const kinds = kindsOf(device);
    if (kinds.length && !kinds.includes(r.kind)) return false;
    return !q || haystack(r).includes(q);
  });
  return sortRows(rows, cols.sort.value, cellValue, (a, b) => collator.compare(a.token, b.token));
});

// Grouped view: one bucket per row (= per input), the actions of both
// sides inside — each action once, with a tick in the side that binds it.
interface DiffLine {
  key: string;
  label: string;
  category: string;
  inA: boolean;
  inB: boolean;
}
function diffLines(r: DiffRow): DiffLine[] {
  const by = new Map<string, DiffLine>();
  const add = (ref: ActionRef, side: "a" | "b") => {
    const key = `${ref.actionmap}/${ref.action}`;
    const line = by.get(key) ?? { key, label: refText(ref), category: categoryLabel(ref.actionmap), inA: false, inB: false };
    if (side === "a") line.inA = true;
    else line.inB = true;
    by.set(key, line);
  };
  for (const ref of r.a) add(ref, "a");
  for (const ref of r.b) add(ref, "b");
  return [...by.values()];
}
// Buckets start open; the closed ones are remembered per token, a search
// forces everything open.
const closedGroups = ref(new Set<string>());
function isGroupOpen(r: DiffRow): boolean {
  return !!search.value.trim() || !closedGroups.value.has(r.token);
}
function toggleDiffGroup(token: string) {
  const s = new Set(closedGroups.value);
  if (!s.delete(token)) s.add(token);
  closedGroups.value = s;
}
const allGroupsOpen = computed(() => filteredRows.value.every((r) => !closedGroups.value.has(r.token)));
function openAllGroups() {
  closedGroups.value = new Set();
}
function closeAllGroups() {
  closedGroups.value = new Set(filteredRows.value.map((r) => r.token));
}

// The action a row is about: the A side names it, else the B side.
function rowAction(row: DiffRow): string {
  const r = row.a[0] ?? row.b[0];
  return r ? refText(r) : "—";
}

// One line per action in a side's cell.
function cellLines(refs: ActionRef[]): string[] {
  return refs.map(refText);
}

const SIGNS: Record<DiffKind, string> = { added: "+", removed: "−", changed: "~", same: "=" };

// Input token without its device prefix, e.g. "js1_button5" -> "button5".
function inputPart(token: string): string {
  return token.replace(/^(js\d+|kb1|gp1)_/, "");
}

// The INPUT cell: SC's label, else the bare token (like the deck).
function inputText(token: string): string {
  const l = props.tokenLabel(token);
  return l === token ? inputPart(token) : l;
}

// --- bindings list ---------------------------------------------------------

// Facts about the live file; null while nothing is loaded.
const info = ref<CurrentBindingsInfo | null>(null);

async function loadInfo() {
  try {
    info.value = await invoke<CurrentBindingsInfo | null>("get_current_bindings_info");
  } catch (e) {
    info.value = null;
    emit("notify", String(e), "error");
  }
}

// One column per device the file knows: the keyboard and the gamepad (SC
// has exactly one of each), then every joystick slot named in <options> or
// carrying a binding (the shipped defaults sit on js1 whether or not the
// file names a device there). A joystick column is named after the device
// the game ranks at that jsN (its own order), never after the saved
// <options> entry, which may be stale; a device saved under another slot
// (the order clash) or not at all says so, as does a missing order.
interface DeviceCol {
  key: string;
  kind: DeviceKind;
  instance: number;
  label: string;
  // The order problem with this slot, shown in the warn colour — or, with
  // `dim`, that no input reaches the app from the device.
  note?: string;
  dim?: boolean;
}

const deviceCols = computed<DeviceCol[]>(() => {
  const ranked = new Map((props.clash?.connected ?? []).map((s) => [s.effective_instance, s]));
  const instances = new Set((info.value?.joysticks ?? []).map((j) => j.instance));
  for (const b of props.bindings) {
    if (b.device_kind === "joystick") instances.add(b.instance);
  }
  const joystick = (n: number): DeviceCol => {
    const slot = ranked.get(n);
    const col: DeviceCol = { key: `js${n}`, kind: "joystick", instance: n, label: `js${n}` };
    if (props.clash?.order_error) {
      col.label += " ·";
      col.note = "no joystick order";
    }
    else if (slot) {
      col.label += ` · ${slot.name ?? "?"}`;
      if (slot.stored_instance === null) col.note = "(not saved)";
      else if (slot.stored_instance !== n) col.note = `(saved js${slot.stored_instance})`;
      else if (props.isLogOnly(n)) {
        col.note = "(no input)";
        col.dim = true;
      }
    }
    return col;
  };
  return [
    { key: "kb1", kind: "keyboard", instance: 1, label: "kb1 · Keyboard/Mouse" },
    { key: "gp1", kind: "gamepad", instance: 1, label: "gp1 · Gamepad" },
    ...[...instances].sort((a, b) => a - b).map(joystick),
  ];
});

// Columns the user switched off. Not remembered: a device column hidden
// once would stay hidden after a restart with no hint why.
const hiddenCols = ref<string[]>([]);
// No column toggled on means every column, like the other chip filters.
const visibleCols = computed(() => {
  const shown = deviceCols.value.filter((c) => !hiddenCols.value.includes(c.key));
  return shown.length ? shown : deviceCols.value;
});

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

// Grouped by category (the game's keybinding screen), or one flat, sortable
// table. The device columns follow the game's order and never sort; the last
// visible device column is the filler.
const listGrouped = persistedRef<boolean>("bindsight.list.grouped", true);
const listColumns = computed<ColumnSpec[]>(() => [
  // Both sortable in both views: flat sorts the whole list; grouped, ACTION
  // sorts the rows inside each category and CATEGORY orders the groups.
  { key: "category", label: "CATEGORY", width: 220, icon: "list" },
  { key: "action", label: "ACTION", width: 320, icon: "target" },
  ...visibleCols.value.map((d, i, all) => ({
    key: d.key,
    label: d.label.toUpperCase(),
    note: d.note?.toUpperCase(),
    dim: d.dim,
    width: i === all.length - 1 ? null : 200,
    sortable: false,
    icon: d.kind === "keyboard" ? "keyboard" : d.kind === "gamepad" ? "gamepad" : "devices",
  })),
]);
const listCols = useTableColumns("bindsight.columns.bindingslist", listColumns, { key: "category", dir: "asc" });

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
  // Categories listed A–Z (the game's own order is arbitrary).
  return out.sort((a, b) => collator.compare(a.label, b.label));
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

// Flat view: every row across all (filtered) groups, sorted by the active
// column. The group is kept so the row still knows its category and rebind
// context.
interface FlatListRow {
  row: ListRow;
  group: ListGroup;
}
const flatRows = computed<FlatListRow[]>(() => {
  const rows: FlatListRow[] = [];
  for (const g of shownGroups.value) for (const r of g.rows) rows.push({ row: r, group: g });
  return sortRows(
    rows,
    listCols.sort.value,
    (fr, key) => (key === "category" ? fr.group.label : fr.row.label),
    (a, b) => collator.compare(a.row.label, b.row.label),
  );
});

// Grouped view: ACTION sorts the rows inside each group; CATEGORY orders the
// groups themselves (any other sort leaves them A–Z from `groups`).
const sortedGroups = computed<ListGroup[]>(() => {
  const s = listCols.sort.value;
  const list = shownGroups.value.map((g) => ({
    ...g,
    rows: sortRows(g.rows, s, (r, key) => (key === "category" ? g.label : r.label), (a, b) => collator.compare(a.label, b.label)),
  }));
  if (s.key === "category") {
    const dir = s.dir === "desc" ? -1 : 1;
    list.sort((a, b) => dir * collator.compare(a.label, b.label));
  }
  return list;
});

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

// SC's "deliberately unbound" rebind: a blank token naming the device the
// binding was on (`js2_ `).
function blankToken(kind: DeviceKind, instance: number): string {
  return `${kind === "keyboard" ? "kb" : kind === "gamepad" ? "gp" : "js"}${instance}_ `;
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

// Current bindings of the action on every device slot (pending ones included),
// one line per slot — empty ones show too, so every device is always visible.
const rebindBefore = computed<RebindLine[]>(() => {
  const r = rebind.value;
  if (!r) return [];
  return deviceCols.value.map((col) => ({
    device: col.key,
    kind: col.kind,
    text: cellTokens(r.row, col).tokens.map(inputText).join(", "),
    changed: false,
  }));
});

// The same once the dialog's changes apply: a kind with a change shows its
// new token on its device (empty for a clear, or on the kind's other slots
// the rebind replaced), the other kinds stay.
const rebindAfter = computed<RebindLine[]>(() => {
  const r = rebind.value;
  if (!r) return [];
  return deviceCols.value.map((col) => {
    const change = r.changes.get(col.kind);
    if (change === undefined) {
      return { device: col.key, kind: col.kind, text: rebindBefore.value.find((b) => b.device === col.key)?.text ?? "", changed: false };
    }
    const target = parseToken(change);
    const landsHere = !isBlank(change) && !!target && deviceKeyOf(col.kind, target.instance) === col.key;
    return { device: col.key, kind: col.kind, text: landsHere ? inputText(change) : "", changed: landsHere };
  });
});

const rebindButtons = computed<ConfirmButton[]>(() => [
  { label: "Clear All", kind: "danger", value: "clearall", side: "left", disabled: !rebindAfter.value.some((l) => l.text) },
  { label: "Apply", kind: "primary", value: "apply", disabled: !rebind.value?.changes.size },
  { label: "Cancel", kind: "outline", value: "cancel" },
]);

// Clear one kind: no binding on any of its devices. The blank goes on the
// device that shows the binding (a joystick default sits on js1).
function clearKind(kind: DeviceKind) {
  const shown = rebindBefore.value.find((l) => l.kind === kind && l.text);
  const instance = Number(/^js(\d+)$/.exec(shown?.device ?? "")?.[1] ?? 1);
  rebind.value?.changes.set(kind, blankToken(kind, instance));
}

// Clear every kind that still has a binding on show.
function clearAll() {
  for (const kind of new Set(rebindAfter.value.filter((l) => l.text).map((l) => l.kind))) clearKind(kind);
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

// A press lights its rows in the list, a release puts the light out; while
// the dialog is recording, the event also feeds the recording.
async function takeInput(p: JoyInput) {
  if (rebind.value && recording.value) void recordInput(p);
  const { edge, momentary } = edgeOf(p);
  if (!edge) return;
  const token = await tokenOf(p);
  if (!token) return;
  if (edge === "release") {
    unlight(token);
    return;
  }
  light(token, momentary);
}

// Recording: every press replaces the candidate (its token taken with the
// modifiers held at that moment), the release of the candidate's input ends
// the recording with it — see `recordEdge` in devices.ts (dual-stage
// triggers, combos). The token lookup is asynchronous, so a release that
// beats it is remembered and applied once the token is in.
const candidate = ref<{ id: string; token: string } | null>(null);
let pendingPress: { id: string; released: boolean } | null = null;

async function recordInput(p: JoyInput) {
  const edge = recordEdge(p);
  const id = inputIdentity(p);
  if (edge === "press") {
    const pending = { id, released: false };
    pendingPress = pending;
    const token = await tokenOf(p);
    if (!token || pendingPress !== pending || !recording.value) return;
    candidate.value = { id, token };
    if (pending.released) takeCandidate();
  } else if (edge === "release") {
    if (candidate.value?.id === id) takeCandidate();
    else if (pendingPress?.id === id) pendingPress.released = true;
  }
}

function takeCandidate() {
  if (!candidate.value) return;
  setCaptured(candidate.value.token);
  recording.value = false;
}

// A recording that ends any way drops its candidate.
watch(recording, (on) => {
  if (!on) {
    candidate.value = null;
    pendingPress = null;
  }
});

// --- input-preview overlay -------------------------------------------------

interface OverlayState {
  target: OverlayTarget;
  active: Set<string>;
  anchor: { x: number; y: number } | null;
  position: OverlayPosition;
}
const overlay = ref<OverlayState | null>(null);

// The map + highlight for a device slot and its SC tokens, or null when the
// slot has no connected device / loaded map, or the tokens light nothing.
function overlayContent(kind: DeviceKind, instance: number, tokens: string[]): Pick<OverlayState, "target" | "active"> | null {
  const target = props.overlayFor(kind, instance);
  if (!target || !target.src || !tokens.length) return null;
  const active = new Set<string>();
  for (const t of tokens) for (const k of inputKeysForToken(t, target.device)) active.add(k);
  return active.size ? { target, active } : null;
}

// Hover a binding cell: the overlay appears once the pointer settles and
// vanishes on the next move (or on leave), so it never covers the label.
const HOVER_DELAY = 200;
let hoverTimer: ReturnType<typeof setTimeout> | null = null;
function clearHoverTimer() {
  if (hoverTimer) clearTimeout(hoverTimer);
  hoverTimer = null;
}
function onCellMove(e: MouseEvent, row: ListRow, col: DeviceCol) {
  clearHoverTimer();
  overlay.value = null;
  const x = e.clientX;
  const y = e.clientY;
  hoverTimer = setTimeout(() => {
    const s = overlayContent(col.kind, col.instance, cellTokens(row, col).tokens);
    if (s) overlay.value = { ...s, anchor: { x, y }, position: props.overlayPosition };
  }, HOVER_DELAY);
}
function onCellLeave() {
  clearHoverTimer();
  overlay.value = null;
}
onUnmounted(clearHoverTimer);

// The last cursor position, so the record overlay can honour a "near cursor"
// preference even though a recorded press carries no mouse move of its own.
let lastMouse = { x: 0, y: 0 };
function trackMouse(e: MouseEvent) {
  lastMouse = { x: e.clientX, y: e.clientY };
}
onMounted(() => window.addEventListener("mousemove", trackMouse));
onUnmounted(() => window.removeEventListener("mousemove", trackMouse));

// Record mode: the overlay follows the record buffer's candidate, placed by
// the same setting as the hover overlay (no cursor here, so "near cursor"
// lands at the offset from the top-left origin).
watch(candidate, (c) => {
  if (!c) {
    overlay.value = null;
    return;
  }
  const parsed = parseToken(c.token);
  const s = parsed ? overlayContent(parsed.kind, parsed.instance, [c.token]) : null;
  const anchor = props.overlayPosition === "mouse-offset" ? { ...lastMouse } : null;
  overlay.value = s ? { ...s, anchor, position: props.overlayPosition } : null;
});

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
  const choice = await ask(
    `Save ${changesText()}?`,
    "save",
    [
      { label: "Save", kind: "primary", value: "save" },
      { label: "Cancel", kind: "outline", value: "cancel" },
    ],
    RESTART_NOTE,
  );
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
  const choice = await ask(
    "Unsaved changes",
    "save",
    [
      { label: "Discard", kind: "danger", value: "discard" },
      { label: "Save", kind: "primary", value: "save" },
      { label: "Keep Editing", kind: "outline", value: "keep" },
    ],
    RESTART_NOTE,
  );
  if (choice === "keep") return false;
  if (choice === "save") return await writeChanges();
  pending.value.clear();
  return true;
}

defineExpose({ requestLeave, takeInput });

// --- wiring ----------------------------------------------------------------

watch(bKey, runCompare);
watch(() => props.hasCurrent, ensureKeys);
// The file changed underneath (reload, restore, resort, save): re-read its facts.
watch(() => props.bindings, loadInfo);

let unlisten: UnlistenFn[] = [];

// Escape is never an input (keyboard.ts): while the rebind dialog is open it
// stops a recording, else cancels the dialog.
function onEscape(e: KeyboardEvent) {
  if (e.key !== "Escape" || !rebind.value) return;
  if (recording.value) recording.value = false;
  else rebind.value = null;
}

onUnmounted(() => {
  window.removeEventListener("keydown", onEscape);
  unlisten.forEach((fn) => fn());
  unlisten = [];
  recording.value = false;
});

// The live bindings changed (reload, restore, resort) — re-diff, A is Current.
watch(
  () => props.bindings,
  () => runCompare(),
);

onMounted(async () => {
  window.addEventListener("keydown", onEscape);
  unlisten.push(await listen<JoyInput>("joy-input", (e) => takeInput(e.payload)));
  await Promise.all([loadProfiles(), loadBackups(), loadInfo()]);
});

// Left-hand rows: Current shows the list, anything else compares against it.
function showList() {
  bKey.value = CURRENT;
  view.value = "list";
}

// Compare replaces the list and its action tile, so pending rebinds are
// settled first.
async function compareWith(key: string) {
  if (!(await requestLeave())) return;
  bKey.value = key;
  view.value = "compare";
}
</script>

<template>
  <div class="bindings-view" :style="{ '--left-w': `${leftWidth}px` }">
    <div class="left">
      <!-- game bindings: the live file -->
      <section class="panel">
        <div class="head">
          <Icon name="list" :size="15" />
          <span class="head-title">Game Bindings</span>
        </div>
        <div class="rows">
          <div v-if="hasCurrent" class="row-item" :class="{ b: view === 'list' }" @click="showList">
            <div class="lines">
              <span class="line-title">Current</span>
              <span class="mono line-sub">actionmaps.xml · {{ info ? stamp(info.modified) : "—" }}</span>
            </div>
          </div>
          <div v-else class="row-none">None</div>
        </div>
      </section>

      <!-- binding profiles -->
      <section class="panel profiles" :style="{ height: `${profilesHeight}px` }">
        <div class="head">
          <Icon name="file" :size="15" />
          <span class="head-title">Binding Profiles</span>
          <button type="button" class="btn primary small" :disabled="busy || !hasCurrent" @click="openSaveProfile">
            <Icon name="plus" :size="12" />Save Profile
          </button>
        </div>
        <div class="rows scroll">
          <div
            v-for="m in profiles"
            :key="m.file"
            class="row-item"
            :class="{ b: view === 'compare' && bKey === `${PROFILE_PREFIX}${m.file}` }"
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
        </div>
      </section>

      <Splitter direction="row" @drag="dragProfiles" @end="endDrag" @reset="profilesHeight = PROFILES_H.def" />

      <!-- backups -->
      <section class="panel grow">
        <div class="head">
          <Icon name="history" :size="15" />
          <span class="head-title">Backups</span>
          <button type="button" class="btn primary small" :disabled="busy" @click="openCreateBackup">
            <Icon name="plus" :size="12" />Create Backup
          </button>
        </div>
        <div class="rows scroll">
          <div
            v-for="b in backups"
            :key="b.id"
            class="row-item"
            :class="{ b: view === 'compare' && bKey === `${BACKUP_PREFIX}${b.id}` }"
            @click="compareWith(`${BACKUP_PREFIX}${b.id}`)"
          >
            <div class="lines">
              <span class="mono line-stamp">{{ stamp(b.created) }}</span>
              <span class="line-sub">{{ b.reason }} · <span class="mono">{{ b.game_version ?? "—" }}</span></span>
            </div>
          </div>
          <div v-if="!backups.length" class="row-none">None</div>
        </div>
      </section>
    </div>

    <Splitter direction="col" class="col-split" @drag="dragLeft" @end="endDrag" @reset="leftWidth = LEFT_W.def" />

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
        <button type="button" class="btn outline small" :disabled="busy || !hasCurrent" @click="openSaveProfile">
          <Icon name="file" :size="14" />
          Save Profile
        </button>
        <button type="button" class="btn outline small" :disabled="busy || !hasCurrent" @click="openCreateBackup">
          <Icon name="history" :size="14" />
          Create Backup
        </button>
        <div class="tile-divider" />
        <button
          type="button"
          class="btn outline small"
          :disabled="busy || !hasCurrent || reorderOptions.length < 2"
          :title="reorderOptions.length < 2 ? 'Needs two joysticks the game sees' : 'Swap two joystick slots'"
          @click="openReorder"
        >
          <Icon name="swap" :size="14" />
          Resort Joysticks
        </button>
        <div class="tile-divider" />
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

    <!-- the picked profile / backup: facts and what can be done with it -->
    <div v-else class="action-tile">
      <div class="tile-name">
        <Icon :name="bBackup ? 'history' : 'file'" :size="14" />
        <span class="name-text">{{ bBackup ? stamp(bBackup.created) : (bProfile?.name ?? nameFor(bKey)) }}</span>
      </div>
      <div class="tile-btns">
        <span class="mono tile-facts">{{ sourceCountSummary }}</span>
        <div class="tile-divider" />
        <span class="mono tile-facts tile-diff">{{ diffSummary }}</span>
        <div class="spacer" />
        <button type="button" class="btn primary small" :disabled="busy || !hasCurrent" @click="openApply">
          <Icon :name="bBackup ? 'rotate' : 'check'" :size="14" />
          {{ applyWord }}
        </button>
        <button
          type="button"
          class="btn danger small"
          :disabled="busy"
          @click="bBackup ? deleteBackup(bBackup) : bProfile && deleteProfile(bProfile)"
        >
          <Icon name="trash" :size="14" />
          Delete
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
        <button
          type="button"
          class="btn small square"
          :class="listGrouped ? 'primary' : 'outline'"
          title="Group by category"
          @click="listGrouped = !listGrouped"
        >
          <Icon name="group" :size="14" />
        </button>
        <button type="button" class="btn outline small square" title="Expand all" :disabled="!listGrouped || allExpanded" @click="expandAll">
          <Icon name="unfold" :size="14" />
        </button>
        <button type="button" class="btn outline small square" title="Collapse all" :disabled="!listGrouped || !expanded.size" @click="collapseAll">
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
            :title="c.note ? `${c.label} ${c.note}` : c.label"
            @click="toggleCol(c.key)"
          >
            <Icon :name="kindIcon(c.kind)" :size="14" />
            {{ c.key }}
          </button>
        </div>
        <div class="divider" />
        <div class="search">
          <Icon name="search" :size="14" />
          <input v-model="listSearch" placeholder="Find…" />
          <button v-if="listSearch" type="button" class="clear" title="Clear" @click="listSearch = ''">
            <Icon name="close" :size="12" />
          </button>
        </div>
      </div>

      <div class="table">
        <ColumnHead
          :columns="listColumns"
          :sort="listCols.sort.value"
          @sort="listCols.toggleSort"
          @resize="listCols.startResize"
          @reset="listCols.resetWidth"
        />
        <template v-if="listGrouped">
          <template v-for="g in sortedGroups" :key="g.key">
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
                <span />
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
                  @dblclick.stop="openRebind(r, g)"
                  @mousemove="onCellMove($event, r, c)"
                  @mouseleave="onCellLeave"
                ><Icon name="bolt" :size="12" class="live-mark" /><span class="bind-text">{{ bindText(r, c) || "—" }}</span></span>
              </div>
            </template>
          </template>
        </template>
        <template v-else>
          <div
            v-for="fr in flatRows"
            :key="`${fr.row.actionmap} ${fr.row.action}`"
            class="row list-row"
            :class="{ live: liveOn && isLive(fr.row) }"
            @dblclick="openRebind(fr.row, fr.group)"
          >
            <span class="bucket-category">{{ fr.group.label }}</span>
            <span class="action-cell">
              <button type="button" class="icon-btn framed" title="Set binding" @click.stop="openRebind(fr.row, fr.group)" @dblclick.stop>
                <Icon name="target" :size="12" />
              </button>
              <span class="action-label" :title="fr.row.action">{{ fr.row.label }}</span>
            </span>
            <span
              v-for="c in visibleCols"
              :key="c.key"
              class="bind-cell"
              :class="{ pending: cellTokens(fr.row, c).pending, empty: !bindText(fr.row, c), live: liveOn && isLiveCell(fr.row, c) }"
              @dblclick.stop="openRebind(fr.row, fr.group)"
              @mousemove="onCellMove($event, fr.row, c)"
              @mouseleave="onCellLeave"
            ><Icon name="bolt" :size="12" class="live-mark" /><span class="bind-text">{{ bindText(fr.row, c) || "—" }}</span></span>
          </div>
        </template>
        <div v-if="!shownGroups.length" class="empty-line">{{ groups.length ? "No matches" : "No game data" }}</div>
      </div>
    </section>

    <!-- compare -->
    <section v-else class="panel compare" :style="{ '--cols': cols.template.value, '--cols-min': `${cols.minWidth.value}px` }">
      <div class="head compare-head">
        <Icon name="compare" :size="16" />
        <span class="head-title no-grow">Compare</span>
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
        <button type="button" class="btn outline small square" title="Expand all" :disabled="!grouped || allGroupsOpen" @click="openAllGroups">
          <Icon name="unfold" :size="14" />
        </button>
        <button type="button" class="btn outline small square" title="Collapse all" :disabled="!grouped || !closedGroups.size" @click="closeAllGroups">
          <Icon name="fold" :size="14" />
        </button>
        <div class="spacer" />
        <div class="search">
          <Icon name="search" :size="14" />
          <input v-model="search" placeholder="Find…" />
          <button v-if="search" type="button" class="clear" title="Clear" @click="search = ''">
            <Icon name="close" :size="12" />
          </button>
        </div>
      </div>
      <!-- one group of chips per device: the device, then its diffs by kind -->
      <div v-if="deviceChips.length" class="filters">
        <template v-for="(d, i) in deviceChips" :key="d.label">
          <div v-if="i > 0" class="divider" />
          <div class="chips">
            <button
              type="button"
              class="chip mono"
              :class="{ active: deviceFilter.includes(d.label) }"
              @click="deviceFilter = toggleIn(deviceFilter, d.label)"
            >
              <Icon :name="kindIcon(d.kind)" :size="14" />
              {{ d.label }}
            </button>
            <button
              v-for="c in KIND_CHIPS"
              :key="c.kind"
              type="button"
              class="chip"
              :class="[c.kind, { active: kindsOf(d.label).includes(c.kind) }]"
              :disabled="!d.counts[c.kind]"
              @click="toggleKind(d.label, c.kind)"
            >
              {{ c.label }} <span class="count">{{ d.counts[c.kind] }}</span>
            </button>
          </div>
        </template>
      </div>

      <div class="table">
        <ColumnHead
          :columns="columns"
          :sort="cols.sort.value"
          @sort="cols.toggleSort"
          @resize="cols.startResize"
          @reset="cols.resetWidth"
        />
        <template v-if="grouped">
          <template v-for="r in filteredRows" :key="r.token">
            <div class="row diff-row bucket-row" :class="r.kind" @click="toggleDiffGroup(r.token)">
              <span class="sign">{{ SIGNS[r.kind] }}</span>
              <span class="input-cell" :title="r.token">
                <Icon :name="isGroupOpen(r) ? 'chevron-down' : 'chevron-right'" :size="14" class="chevron" />
                <span class="mono dim">{{ deviceLabel(r) }}</span>
                <span class="bucket-input" :class="{ mono: inputText(r.token) === inputPart(r.token) }">{{ inputText(r.token) }}</span>
              </span>
              <span />
              <span />
              <span />
              <span />
            </div>
            <template v-if="isGroupOpen(r)">
              <div v-for="line in diffLines(r)" :key="line.key" class="row diff-row line-row" :class="r.kind">
                <span />
                <span />
                <span class="bucket-action">{{ line.label }}</span>
                <span class="bucket-category">{{ line.category }}</span>
                <span :class="line.inB ? 'side' : 'empty'">{{ line.inB ? line.label : "—" }}</span>
                <span :class="line.inA ? 'side' : 'empty'">{{ line.inA ? line.label : "—" }}</span>
              </div>
            </template>
          </template>
        </template>
        <template v-else>
          <div v-for="r in filteredRows" :key="r.token" class="row diff-row" :class="r.kind">
            <span class="sign">{{ SIGNS[r.kind] }}</span>
            <span class="input-cell dim" :title="r.token">
              <span class="mono">{{ deviceLabel(r) }}</span>
              <span :class="{ mono: inputText(r.token) === inputPart(r.token) }">{{ inputText(r.token) }}</span>
            </span>
            <span>{{ rowAction(r) }}</span>
            <span class="bucket-category">{{ rowCategory(r) }}</span>
            <span v-for="side in (['b', 'a'] as const)" :key="side" class="refs" :class="r[side].length ? 'side' : 'empty'">
              <template v-if="r[side].length">
                <span v-for="(line, i) in cellLines(r[side])" :key="i" class="ref-line">{{ line }}</span>
              </template>
              <template v-else>—</template>
            </span>
          </div>
        </template>
        <div v-if="!filteredRows.length" class="empty-line">{{ diffRows.length ? "No matches" : "No differences" }}</div>
      </div>
    </section>
    </div>

    <ConfirmDialog
      v-if="confirm"
      :title="confirm.title"
      :subtitle="confirm.subtitle"
      :icon="confirm.icon"
      :buttons="confirm.buttons"
      @choose="onConfirm"
    />

    <!-- save the live file as a binding profile -->
    <ConfirmDialog
      v-if="nameDialog"
      title="Save Profile"
      icon="file"
      :buttons="[
        { label: 'Save', kind: 'primary', value: 'save', disabled: !sanitizeName(nameDialog.name, '') },
        { label: 'Cancel', kind: 'outline', value: 'cancel' },
      ]"
      @choose="onNameChoose"
    >
      <input
        ref="nameInput"
        class="name-in"
        :value="nameDialog.name"
        :maxlength="NAME_MAX"
        spellcheck="false"
        placeholder="Name"
        @input="nameDialog.name = stripNameChars(($event.target as HTMLInputElement).value)"
        @keydown.enter="sanitizeName(nameDialog.name, '') && onNameChoose('save')"
      />
    </ConfirmDialog>

    <!-- back the live file up, with a description -->
    <ConfirmDialog
      v-if="backupDialog"
      title="Create Backup"
      icon="history"
      :buttons="[
        { label: 'Create', kind: 'primary', value: 'create' },
        { label: 'Cancel', kind: 'outline', value: 'cancel' },
      ]"
      @choose="onBackupChoose"
    >
      <label class="backup-row">
        <span class="reorder-label">Description</span>
        <input
          ref="backupInput"
          class="name-in"
          v-model="backupDialog.reason"
          :maxlength="BACKUP_REASON_MAX"
          spellcheck="false"
          @keydown.enter="onBackupChoose('create')"
        />
      </label>
    </ConfirmDialog>

    <!-- apply a profile / backup: which devices' bindings to take over -->
    <ConfirmDialog
      v-if="applyDialog"
      :title="`${applyWord} ${nameFor(bKey)}`"
      icon="check"
      :buttons="applyButtons"
      :width="800"
      @choose="onApplyChoose"
    >
      <!-- one row per device: take it over or not, and for a joystick the
           live slot its bindings land on (a swap keeps the slots unique) -->
      <div class="apply-table">
        <div class="apply-head">
          <span />
          <span>Source</span>
          <span>Apply to</span>
        </div>
        <div
          v-for="d in applyDialog.devices"
          :key="d.key"
          class="apply-row"
          :class="{ off: !applyDialog.on.has(d.key), clash: slotClashes.has(d.key) }"
        >
          <label class="check apply-check">
            <input type="checkbox" :checked="applyDialog.on.has(d.key)" @change="toggleApplyDevice(d.key)" />
          </label>
          <span class="apply-device">
            <span class="mono dim">{{ d.key }}</span>
            <span v-if="sourceName(d)" class="check-name">{{ sourceName(d) }}</span>
          </span>
          <span class="apply-slot">
            <template v-if="d.sel.kind === 'joystick'">
              <Dropdown
                variant="mono"
                :modelValue="String(targetOf(d))"
                :options="slotOptions(d.sel.instance)"
                title="Slot to apply to"
                @update:modelValue="setTarget(d.key, $event)"
              />
              <span class="check-name">{{ slotName(targetOf(d)) }}</span>
              <Icon v-if="slotClashes.has(d.key)" name="warning" :size="14" class="clash-mark" />
            </template>
            <template v-else>
              <span class="mono dim">{{ d.key }}</span>
              <span class="check-name">{{ d.label }}</span>
            </template>
          </span>
        </div>
      </div>
      <p class="dialog-note">{{ RESTART_NOTE }}</p>
    </ConfirmDialog>

    <!-- rebind: every device at once; Record or Clear changes a kind, Apply queues the changes -->
    <ConfirmDialog
      v-if="rebind"
      :title="rebind.row.label"
      :subtitle="rebind.category"
      icon="target"
      :buttons="rebindButtons"
      :width="760"
      captureKeys
      @choose="onRebindChoose"
    >
      <div class="rb-columns">
        <div class="rb-side">
          <span class="rb-label">Before</span>
          <div class="rb-block">
            <div v-for="b in rebindBefore" :key="b.device" class="rb-line">
              <span class="mono dim">{{ b.device }}</span>
              <span class="rb-text" :class="{ dim: !b.text }">{{ b.text || "—" }}</span>
              <button v-if="b.text" type="button" class="icon-btn rb-clear" title="Clear" @click="clearKind(b.kind)">
                <Icon name="close" :size="12" />
              </button>
            </div>
            <div v-if="!rebindBefore.length" class="rb-line dim">—</div>
          </div>
        </div>
        <Icon name="arrow-right" :size="18" class="dim rb-arrow" />
        <div class="rb-side">
          <span class="rb-label">After</span>
          <div class="rb-block">
            <div v-for="a in rebindAfter" :key="a.device" class="rb-line">
              <span class="mono dim">{{ a.device }}</span>
              <span class="rb-text" :class="{ 'rb-new': a.changed, dim: !a.text }">{{ a.text || "—" }}</span>
            </div>
            <div v-if="!rebindAfter.length" class="rb-line dim">—</div>
          </div>
        </div>
      </div>
      <div class="rb-record">
        <button type="button" class="btn" :class="recording ? 'primary' : 'outline'" @click="recording = true">
          <Icon name="target" :size="15" />
          {{ recording ? "Recording…" : "Record Input" }}
        </button>
        <!-- one line of fixed height, always there, so the dialog does not jump -->
        <div class="rb-status">
          <template v-if="recording && candidate">
            <span class="rec-chip mono">{{ inputText(candidate.token) }}</span>
            <span class="rb-sep">·</span>
          </template>
          <span class="rb-hint" :class="{ on: recording }">Esc to cancel</span>
        </div>
      </div>
    </ConfirmDialog>

    <ConfirmDialog
      v-if="reorderOpen"
      title="Resort joysticks"
      icon="swap"
      :buttons="[
        { label: 'Cancel', kind: 'outline', value: 'cancel', side: 'left' },
        { label: 'Console command', kind: 'outline', value: 'console', disabled: !reorderValid },
        { label: 'Apply to config', kind: 'primary', value: 'config', disabled: !reorderValid },
      ]"
      @choose="onReorderChoose"
    >
      <div class="reorder-row">
        <span class="reorder-label">Swap</span>
        <Dropdown v-model="reorderA" class="reorder-pick" variant="small" :options="reorderOptions" />
      </div>
      <div class="reorder-row">
        <span class="reorder-label">with</span>
        <Dropdown v-model="reorderB" class="reorder-pick" variant="small" :options="reorderOptions" />
      </div>
      <p class="reorder-note">If game is running, restart for changes to take effect</p>
    </ConfirmDialog>

    <ConsoleCommandDialog
      v-if="reorderCommand"
      :command="reorderCommand"
      @close="reorderCommand = null"
      @copy="emit('copy', $event)"
    />

    <InputOverlay
      v-if="overlay"
      :map="overlay.target.map"
      :src="overlay.target.src"
      :imageUrl="overlay.target.imageUrl"
      :active="overlay.active"
      :size="overlaySize"
      :position="overlay.position"
      :anchor="overlay.anchor"
    />
  </div>
</template>

<style scoped>
/* The splitter is its own 16px column between the two. */
.bindings-view {
  flex: 1;
  display: grid;
  grid-template-columns: var(--left-w, 360px) 16px minmax(0, 1fr);
  padding: 12px 16px 16px;
  min-height: 0;
}

.left,
.right {
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.right {
  gap: 16px;
  min-width: 0;
}

/* Game Bindings, Binding Profiles (dragged height), the row splitter, Backups. */
.left .panel.profiles {
  margin-top: 16px;
  flex: none;
}

.name-in {
  width: 100%;
  height: var(--h-control);
  padding: 0 10px;
  box-sizing: border-box;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-family: inherit;
  font-size: 14px;
  outline: none;
}

.name-in:focus {
  border-color: var(--accent);
}

/* Own checkbox look (mirrors SettingsDialog): WebKitGTK would paint GTK's. */
/* Device · Apply · To slot, one line per device. */
/* Under the device table: the restart reminder, like the Fix via Config dialog. */
.dialog-note {
  margin: 12px 0 0;
  font-size: 12px;
  color: var(--text-3);
}

.apply-table {
  display: flex;
  flex-direction: column;
  margin-bottom: 16px;
}

.apply-head,
.apply-row {
  display: grid;
  grid-template-columns: 24px minmax(240px, 1fr) minmax(0, 1fr);
  gap: 12px;
  align-items: center;
  padding: 8px 4px;
}

.apply-head {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-2);
  border-bottom: 1px solid var(--border-dim);
}

.apply-check {
  justify-content: center;
}

.apply-row + .apply-row {
  border-top: 1px solid var(--border-dim);
}

.apply-row:last-child {
  border-bottom: 1px solid var(--border-dim);
}

.apply-row.off .apply-device,
.apply-row.off .apply-slot {
  opacity: 0.45;
}

.apply-device {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  font-size: 14px;
}

.apply-device .check-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.apply-slot {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.apply-slot .check-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Two sticks on one slot: the user sorts it out, Apply waits. */
.apply-row.clash .apply-slot {
  color: var(--warn);
}

.apply-row.clash .apply-slot .check-name {
  color: var(--warn);
}

.clash-mark {
  flex-shrink: 0;
  color: var(--warn);
}

.check {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
  cursor: pointer;
  user-select: none;
}

.check .check-name {
  color: var(--text-2);
}

.check input {
  appearance: none;
  width: 16px;
  height: 16px;
  margin: 0;
  flex-shrink: 0;
  display: grid;
  place-content: center;
  border: 1px solid rgba(255, 255, 255, 0.5);
  border-radius: 3px;
  background: transparent;
  cursor: pointer;
}

.check input:hover {
  border-color: var(--accent);
}

.check input:checked {
  background: var(--accent);
  border-color: var(--accent);
}

.check input:checked::after {
  content: "";
  width: 8px;
  height: 4px;
  border-left: 2px solid var(--accent-text);
  border-bottom: 2px solid var(--accent-text);
  transform: translateY(-1px) rotate(-45deg);
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

.tile-divider {
  width: 1px;
  height: 20px;
  margin: 0 4px;
  background: var(--border-dim);
}

.tile-diff {
  color: var(--text);
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

/* The per-device diff chips, a row of their own under the Compare head. */
.filters {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 16px 16px;
  border-bottom: 1px solid var(--border-dim);
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

/* The kind chips stay quiet: regular weight, the colour muted; only the
   count is bold. */
.chip.added,
.chip.removed,
.chip.changed {
  font-weight: 400;
}

.chip.added .count,
.chip.removed .count,
.chip.changed .count {
  font-weight: 600;
  color: inherit;
}

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

/* Toggled on: the kind's own colour, full, on a light tint — not the accent. */
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

.chip:disabled {
  opacity: 0.4;
  cursor: default;
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

/* A side's actions, one per line; the row grows with them. */
.diff-row {
  align-items: start;
}

.diff-row .refs {
  display: flex;
  flex-direction: column;
  gap: 2px;
  white-space: normal;
}

.ref-line {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Grouped view: the bucket head is the token's row, its actions hang below
   it without the tint, tied to the head by a thin tree line under the chevron. */
.bucket-row {
  cursor: pointer;
  user-select: none;
  align-items: center;
  border-top: 1px solid var(--border-dim);
}

.bucket-row .chevron {
  flex-shrink: 0;
}

.bucket-row .bucket-input {
  font-weight: 600;
}

.line-row {
  position: relative;
  background: transparent;
  align-items: center;
}

.line-row::before {
  content: "";
  position: absolute;
  left: 50px;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--border-dim);
}

.bucket-action,
.bucket-category {
  color: var(--text-2);
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
  /* Fill the whole grid cell (the row centres its items, so a bare span would
     only be text-tall) so the hover preview triggers across the cell. */
  display: flex;
  align-items: center;
  align-self: stretch;
  min-width: 0;
  transition: color 600ms ease-out;
}

.bind-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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

/* Before and After side by side, an arrow between them; the labels sit
   above the tinted blocks. */
.rb-columns {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  gap: 12px;
  align-items: stretch;
  margin-bottom: 10px;
}

.rb-side {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.rb-arrow {
  align-self: center;
  margin-top: 22px;
}

/* As tall as its lines: one per device kind at most (joystick, keyboard,
   gamepad), so the dialog barely moves. */
.rb-block {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
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
  font-size: 16px;
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
  flex-direction: column;
  align-items: center;
  gap: 4px;
}

.rb-record .btn {
  padding: 0 22px;
}

/* The input a recording holds until it is released. */
.rec-chip {
  padding: 3px 8px;
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
  font-size: 12px;
  white-space: nowrap;
}

/* Reorder dialog body (slot content of ConfirmDialog). */
.reorder-row {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
}

.reorder-row + .reorder-row {
  margin-top: 8px;
}

.reorder-label {
  font-size: 13px;
  color: var(--text-2);
}

/* Create Backup dialog: label + description input on one line. */
.backup-row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 10px;
}

/* The dropdowns fill the row; "chip" would collide with this file's .chip. */
.reorder-pick {
  width: 100%;
  max-width: none;
}

.reorder-note {
  margin: 12px 0 0;
  font-size: 12px;
  color: var(--text-3);
}

/* Candidate chip, separator and Esc hint on one line; the height is the
   chip's, so the row neither grows when a chip appears nor collapses without one. */
.rb-status {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 22px;
}

.rb-sep {
  color: var(--text-3);
}

.rb-hint {
  color: var(--text-2);
  font-size: 13px;
  line-height: 1.2;
  visibility: hidden;
}

.rb-hint.on {
  visibility: visible;
}
</style>
