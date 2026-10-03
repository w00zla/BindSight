<script setup lang="ts">
// The Config mode: the game's device settings — inversion, sensitivity
// curves, deadzone / saturation, mouse and gamepad sensitivity — edited
// without starting the game. Left: the devices (like the Monitor knows
// them), profiles, backups. Right: the picked device's settings, edits kept
// pending until Save / Discard; or, with a profile / backup picked, its
// settings compared against Current.
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Icon from "./Icon.vue";
import Splitter from "./Splitter.vue";
import ConfirmDialog, { type ConfirmButton, type ConfirmIcon } from "./ConfirmDialog.vue";
import ApplyDialog from "./ApplyDialog.vue";
import ProfilesPanel from "./ProfilesPanel.vue";
import BackupsPanel, { stamp } from "./BackupsPanel.vue";
import AxisCard from "./AxisCard.vue";
import OptionTable from "./OptionTable.vue";
import CurveDialog from "./CurveDialog.vue";
import ConfigCompare from "./ConfigCompare.vue";
import YesNo from "./YesNo.vue";
import { persistedRef } from "../persist";
import { axisOutput, holdAxisStream } from "../axisStream";
import { deviceIcon, deviceName, kindIcon } from "../devices";
import { KEY_COUNT, MOUSE_INPUTS } from "../keyboard";
import {
  GAMEPAD_DEVICE,
  GAMEPAD_INPUTS,
  JOYSTICK_INPUTS,
  MOUSE_DEFAULTS,
  blockKey,
  compareRows,
  curveValue,
  diffChanges,
  effectiveCurve,
  guidMatches,
  nodeLabel,
  normProduct,
  parentCurveOf,
  r2,
  stateOf,
  workingState,
  type CompareDevice,
  type CompareRow,
  type Curve,
} from "../configModel";
import type {
  ActionMap,
  AxisRaw,
  BackupSummary,
  BindingProfileSummary,
  ConfigChange,
  ConfigLabels,
  DeviceConfigView,
  DeviceInfo,
  DeviceKind,
  DeviceOptionsView,
  DeviceSel,
  DiffSource,
  LoadStatus,
  LogOnlyJoystick,
  ManagedAttribute,
  NodeValue,
  OptionNode,
  OptionTree,
  OptionsBlock,
  ResolvedBinding,
  SlotStatus,
  ToastType,
} from "../types";

// `devices`: the SDL devices in display order; `logOnly`: the joysticks the
// game lists that SDL does not. `slotFor` / `isUnseen` / `noOrder`: the
// game's joystick order as App knows it. Live axes come from the raw stream
// (`axis-raw`), held on while the mode is up.
const props = defineProps<{
  devices: DeviceInfo[];
  logOnly: LogOnlyJoystick[];
  slotFor: (guid: string | null) => SlotStatus | null;
  isUnseen: (d: DeviceInfo) => boolean;
  noOrder: boolean;
  bindings: ResolvedBinding[];
  actionMaps: ActionMap[];
  hasCurrent: boolean;
}>();
const emit = defineEmits<{
  notify: [message: string, type: ToastType];
  saved: [status: LoadStatus];
  restored: [status: LoadStatus];
}>();

// Every write into the game's files needs a game restart to show.
const RESTART_NOTE = "If game is running, restart for changes to take effect";

// --- layout: the left column's width and the Devices / Profiles panels'
// heights, dragged at a splitter and remembered ------------------------------

const LEFT_W = { min: 220, max: 640, def: 300 };
const leftWidth = persistedRef<number>("bindsight.config.leftWidth", LEFT_W.def);
const DEVICES_H = { min: 120, max: 700, def: 260 };
const devicesHeight = persistedRef<number>("bindsight.config.devicesHeight", DEVICES_H.def);
const PROFILES_H = { min: 110, max: 700, def: 220 };
const profilesHeight = persistedRef<number>("bindsight.config.profilesHeight", PROFILES_H.def);
let dragStart: number | null = null;
function dragLeft(delta: number) {
  dragStart ??= leftWidth.value;
  leftWidth.value = Math.min(LEFT_W.max, Math.max(LEFT_W.min, Math.round(dragStart + delta)));
}
function dragDevices(delta: number) {
  dragStart ??= devicesHeight.value;
  devicesHeight.value = Math.min(DEVICES_H.max, Math.max(DEVICES_H.min, Math.round(dragStart + delta)));
}
function dragProfiles(delta: number) {
  dragStart ??= profilesHeight.value;
  profilesHeight.value = Math.min(PROFILES_H.max, Math.max(PROFILES_H.min, Math.round(dragStart + delta)));
}
function endDrag() {
  dragStart = null;
}

// --- confirm dialog --------------------------------------------------------

const confirm = ref<{ title: string; icon: ConfirmIcon; buttons: ConfirmButton[]; subtitle?: string } | null>(null);
let confirmResolve: ((value: string) => void) | null = null;

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

// --- game data and the live settings ----------------------------------------

const trees = ref<OptionTree[]>([]);
const labels = ref<ConfigLabels | null>(null);
// The live settings as saved; null while nothing is loaded.
const current = ref<DeviceConfigView | null>(null);
const busy = ref(false);

async function loadGameData() {
  try {
    trees.value = await invoke<OptionTree[]>("get_option_trees");
    labels.value = await invoke<ConfigLabels>("get_config_labels");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function loadCurrent() {
  if (!props.hasCurrent) {
    current.value = null;
    return;
  }
  try {
    current.value = await invoke<DeviceConfigView>("get_device_config", { source: { kind: "current" } });
  } catch (e) {
    current.value = null;
    emit("notify", String(e), "error");
  }
}

// --- devices -----------------------------------------------------------------

// A device as the Config mode lists it: the keyboard (kb1), the slotted pad
// (gp1), the joysticks the game sees in its order; `slot` null = no
// joystick order. `sdl` null = a log-only joystick (no input reaches the
// app, still configurable).
interface ConfigDevice {
  key: string;
  kind: DeviceKind;
  name: string;
  slot: number | null;
  sdl: DeviceInfo | null;
  guid: string | null;
}

const configDevices = computed<ConfigDevice[]>(() => {
  const out: ConfigDevice[] = [];
  const kb = props.devices.find((d) => d.kind === "keyboard");
  if (kb) out.push({ key: "kb1", kind: "keyboard", name: deviceName(kb), slot: 1, sdl: kb, guid: null });
  const gp = props.devices.find((d) => d.kind === "gamepad" && d.gamepad_slot !== null);
  if (gp) out.push({ key: "gp1", kind: "gamepad", name: deviceName(gp), slot: 1, sdl: gp, guid: null });
  const sticks: ConfigDevice[] = [];
  for (const d of props.devices) {
    if (d.kind !== "joystick" || props.isUnseen(d)) continue;
    const slot = props.noOrder ? null : (props.slotFor(d.sc_product_guid)?.effective_instance ?? null);
    sticks.push({ key: `sdl:${d.index}`, kind: "joystick", name: deviceName(d), slot, sdl: d, guid: d.sc_product_guid });
  }
  for (const j of props.logOnly) {
    const slot = props.slotFor(j.sc_product_guid)?.effective_instance ?? null;
    sticks.push({ key: `log:${j.sc_product_guid}`, kind: "joystick", name: j.sc_name ?? j.sdl_name, slot, sdl: null, guid: j.sc_product_guid });
  }
  sticks.sort((a, b) => (a.slot ?? Infinity) - (b.slot ?? Infinity));
  return [...out, ...sticks];
});

const selectedKey = ref("kb1");
const selected = computed<ConfigDevice | null>(() => configDevices.value.find((d) => d.key === selectedKey.value) ?? configDevices.value[0] ?? null);

function slotText(d: ConfigDevice): string {
  if (d.kind === "keyboard") return "kb1";
  if (d.kind === "gamepad") return "gp1";
  return `js${d.slot}`;
}

function counts(s: DeviceInfo): string[] {
  return s.kind === "keyboard" ? [`${KEY_COUNT} keys`, `${MOUSE_INPUTS.length} btns`] : [`${s.num_buttons} btns`, `${s.num_axes} axes`, `${s.num_hats} hats`];
}

function bindingCount(d: ConfigDevice): number {
  if (d.kind === "joystick") return props.bindings.filter((b) => b.device_kind === "joystick" && b.instance === d.slot).length;
  return props.bindings.filter((b) => b.device_kind === d.kind).length;
}

// The device list's second line.
function subLine(d: ConfigDevice): string {
  if (!d.sdl) return "no input";
  if (d.slot === null) return "no joystick order";
  return counts(d.sdl).join(" · ");
}

// The header's info line, like the Monitor's device tile.
function infoLine(d: ConfigDevice): string {
  if (!d.sdl) return "input handling not supported";
  if (d.slot === null) return counts(d.sdl).join(" · ");
  return [`${bindingCount(d)} bindings`, ...counts(d.sdl)].join(" · ");
}

// --- pending edits -------------------------------------------------------------

// The user's edits in order, as the changes they are; the working values are
// the saved ones with these applied (so a reload underneath keeps them on
// top), and Save writes the smallest set of changes that gets there.
const edits = ref<ConfigChange[]>([]);

const working = computed(() => (current.value ? workingState(current.value, edits.value, trees.value) : null));
const savedState = computed(() => (current.value ? stateOf(current.value) : null));
const changes = computed<ConfigChange[]>(() => (current.value && working.value ? diffChanges(current.value, working.value, trees.value) : []));
const dirty = computed(() => changes.value.length > 0);

// A slider drag sends a change per step: one that replaces the last edit's
// target replaces that edit.
function sameTarget(a: ConfigChange, b: ConfigChange): boolean {
  if (a.type !== b.type) return false;
  switch (a.type) {
    case "curve":
    case "invert":
      return b.type === a.type && a.kind === b.kind && a.instance === b.instance && a.node === b.node;
    case "deadzone":
    case "saturation":
      return b.type === a.type && a.device === b.device && a.input === b.input;
    case "attribute":
      return b.type === "attribute" && a.setting === b.setting;
    default:
      return true;
  }
}

function pushEdit(ch: ConfigChange) {
  const list = edits.value;
  const last = list[list.length - 1];
  edits.value = last && sameTarget(last, ch) ? [...list.slice(0, -1), ch] : [...list, ch];
}

// Devices with a pending change, for the dot in the device list.
function deviceDirty(d: ConfigDevice): boolean {
  return changes.value.some((c) => {
    if (c.type === "curve" || c.type === "invert") return c.kind === d.kind && c.instance === d.slot;
    if (c.type === "deadzone" || c.type === "saturation") return c.device === deviceOptionsName(d);
    if (c.type === "attribute") return c.setting === "gamepad_sensitivity" ? d.kind === "gamepad" : d.kind === "keyboard";
    return d.kind === "keyboard";
  });
}

function changesText(): string {
  const n = changes.value.length;
  return `${n} change${n === 1 ? "" : "s"}`;
}

// --- the picked device's settings ----------------------------------------------

const tree = computed(() => (selected.value ? trees.value.find((t) => t.kind === selected.value!.kind) ?? null : null));

// The option values of the device's slot (`<options type instance>`).
function blockValues(state: { values: Record<string, Record<string, NodeValue>> } | null, d: ConfigDevice | null): Record<string, NodeValue> {
  if (!state || !d || d.slot === null) return {};
  return state.values[blockKey(d.kind, d.slot)] ?? {};
}
const values = computed(() => blockValues(working.value, selected.value));
// The file has a settings block for the device's slot. Without one the
// option table is read-only: the backend never creates a slot, so an edit
// could never be saved.
const hasBlock = computed(() => {
  const d = selected.value;
  return !!d && d.slot !== null && !!current.value?.options.some((b) => b.kind === d.kind && b.instance === d.slot);
});
const savedValues = computed(() => blockValues(savedState.value, selected.value));

// The `<deviceoptions>` name of a device: the pad's fixed one, a joystick's
// raw Product (as the file has it, from its deviceoptions or its options).
function deviceOptionsName(d: ConfigDevice): string | null {
  if (d.kind === "gamepad") return GAMEPAD_DEVICE;
  if (d.kind !== "joystick" || !current.value) return null;
  const opt = current.value.device_options.find((o) => guidMatches(o.name, d.guid));
  if (opt) return opt.name;
  const block =
    current.value.options.find((b) => b.kind === "joystick" && b.instance === d.slot && guidMatches(b.product, d.guid)) ??
    current.value.options.find((b) => b.kind === "joystick" && guidMatches(b.product, d.guid));
  return block?.product ?? null;
}

interface AxisRow {
  input: string;
  title: string;
  deadzoneLabel: string;
  saturationLabel: string;
  deadzone: number;
  saturation: number | null;
  deadzoneChanged: boolean;
  saturationChanged: boolean;
}

// An axis card's title: the game's deadzone label without the words it
// shares with the other axes' deadzone labels of the kind ("Deadzone
// Joystick X Axis" -> "X Axis") — works in any language that keeps the axis
// part at the end. A title keeps at least two words where the label has
// them (the pad's two labels share "Thumb" too: "Thumb Left", not "Left");
// nothing left = the full label.
function cardTitle(label: string, all: string[]): string {
  const words = label.split(/\s+/).filter(Boolean);
  const others = all.map((l) => l.split(/\s+/).filter(Boolean));
  let n = 0;
  while (n < words.length && others.every((o) => o[n] === words[n])) n++;
  n = Math.min(n, Math.max(0, words.length - 2));
  return n > 0 && n < words.length ? words.slice(n).join(" ") : label;
}

const axisDevice = computed(() => (selected.value ? deviceOptionsName(selected.value) : null));

const axisRows = computed<AxisRow[]>(() => {
  const d = selected.value;
  if (!d || d.kind === "keyboard" || !working.value) return [];
  const name = axisDevice.value;
  const inputs = d.kind === "joystick" ? JOYSTICK_INPUTS : GAMEPAD_INPUTS;
  const dzLabelOf = (input: string) => (d.kind === "joystick" ? labels.value?.joystick_deadzone[input] : labels.value?.gamepad_deadzone[input]) || input;
  const dzLabels = inputs.map(dzLabelOf);
  return inputs.map((input) => {
    const v = name ? working.value!.devopts[name]?.[input] : undefined;
    const dzLabel = dzLabelOf(input);
    const satLabel = labels.value?.joystick_saturation[input] || input;
    const changed = (type: "deadzone" | "saturation") => changes.value.some((c) => c.type === type && c.device === name && c.input === input);
    return {
      input,
      title: cardTitle(dzLabel, dzLabels),
      deadzoneLabel: dzLabel,
      saturationLabel: satLabel,
      deadzone: r2(v?.deadzone ?? 0),
      saturation: d.kind === "joystick" ? r2(v?.saturation ?? 1) : null,
      deadzoneChanged: changed("deadzone"),
      saturationChanged: changed("saturation"),
    };
  });
});

function setAxis(type: "deadzone" | "saturation", input: string, value: number) {
  const device = axisDevice.value;
  if (device) pushEdit({ type, device, input, value });
}

// The actions bound to a joystick axis (the file's rebinds and the game's
// defaults), each once.
function boundTo(input: string): string[] {
  const d = selected.value;
  if (!d || d.kind !== "joystick" || d.slot === null) return [];
  const token = `js${d.slot}_${input}`;
  return [...new Set(props.bindings.filter((b) => b.token === token).map((b) => b.label ?? b.action))];
}

// --- mouse and pad settings --------------------------------------------------

interface SettingRow {
  key: string;
  label: string;
  yesno: boolean;
  min: number;
  max: number;
  step: number;
  // The value shown (a default while not set) and its text.
  shown: number;
  text: string;
  notSet: boolean;
  changed: boolean;
  disabled: boolean;
  set: (v: number) => void;
}

function attrChanged(setting: ManagedAttribute): boolean {
  return changes.value.some((c) => c.type === "attribute" && c.setting === setting);
}

const mouseRows = computed<SettingRow[]>(() => {
  const w = working.value;
  if (!w || !current.value) return [];
  const m = w.mouse;
  const l = labels.value;
  const noAttr = !current.value.has_attributes;
  const intRow = (key: ManagedAttribute, label: string, v: number | null, def: number | null, min: number, max: number): SettingRow => ({
    key,
    label,
    yesno: false,
    min,
    max,
    step: 1,
    shown: v ?? def ?? min,
    text: v === null && def === null ? "—" : String(Math.round(v ?? def ?? 0)),
    notSet: v === null,
    changed: attrChanged(key),
    disabled: noAttr,
    set: (value) => pushEdit({ type: "attribute", setting: key, value }),
  });
  const unitRow = (key: "mouse_acceleration" | "mouse_smoothing", label: string, v: number | null): SettingRow => ({
    key,
    label,
    yesno: false,
    min: 0,
    max: 1,
    step: 0.01,
    shown: v ?? 0,
    text: (v ?? 0).toFixed(2),
    notSet: v === null,
    changed: changes.value.some((c) => c.type === key),
    disabled: false,
    set: (value) => pushEdit({ type: key, value }),
  });
  const zoomOn = m.zoom_scaling_enabled ?? MOUSE_DEFAULTS.zoom_scaling_enabled;
  return [
    intRow("mouse_sensitivity", l?.mouse_sensitivity ?? "", m.sensitivity, null, 1, 100),
    intRow("ads_percent", l?.mouse_ads_percent ?? "", m.ads_percent, MOUSE_DEFAULTS.ads_percent, 0, 200),
    {
      key: "zoom_scaling_enabled",
      label: l?.mouse_zoom_scaling_enabled ?? "",
      yesno: true,
      min: 0,
      max: 1,
      step: 1,
      shown: zoomOn ? 1 : 0,
      text: "",
      notSet: m.zoom_scaling_enabled === null,
      changed: attrChanged("zoom_scaling_enabled"),
      disabled: noAttr,
      set: (value) => pushEdit({ type: "attribute", setting: "zoom_scaling_enabled", value }),
    },
    intRow("zoom_scaling_percent", l?.mouse_zoom_scaling_percent ?? "", m.zoom_scaling_percent, MOUSE_DEFAULTS.zoom_scaling_percent, 0, 200),
    unitRow("mouse_acceleration", l?.mouse_acceleration ?? "", m.acceleration),
    unitRow("mouse_smoothing", l?.mouse_smoothing ?? "", m.smoothing),
  ];
});

const padRow = computed<SettingRow | null>(() => {
  const w = working.value;
  if (!w || !current.value) return null;
  const v = w.gamepadSensitivity;
  return {
    key: "gamepad_sensitivity",
    label: labels.value?.gamepad_sensitivity ?? "",
    yesno: false,
    min: 0,
    max: 2,
    step: 0.01,
    shown: v ?? 0,
    text: v === null ? "—" : v.toFixed(2),
    notSet: v === null,
    changed: attrChanged("gamepad_sensitivity"),
    disabled: !current.value.has_attributes,
    set: (value) => pushEdit({ type: "attribute", setting: "gamepad_sensitivity", value }),
  };
});

function fill(r: SettingRow): string {
  return `${((r.shown - r.min) / (r.max - r.min)) * 100}%`;
}

function onRange(r: SettingRow, e: Event) {
  const v = Number((e.target as HTMLInputElement).value);
  r.set(r.step === 1 ? Math.round(v) : r2(v));
}

// --- option table ------------------------------------------------------------

function onExponent(n: OptionNode, value: number) {
  const d = selected.value;
  if (d?.slot) pushEdit({ type: "curve", kind: d.kind, instance: d.slot, node: n.name, value: { kind: "exponent", value } });
}

function onResetCurve(n: OptionNode) {
  const d = selected.value;
  if (d?.slot) pushEdit({ type: "curve", kind: d.kind, instance: d.slot, node: n.name, value: { kind: "default" } });
}

function onInvert(n: OptionNode, value: boolean | null) {
  const d = selected.value;
  if (d?.slot) pushEdit({ type: "invert", kind: d.kind, instance: d.slot, node: n.name, value });
}

// The curve dialog on one node, its effective curve to start from.
const curveEdit = ref<{ node: OptionNode; curve: Curve } | null>(null);

function onEditCurve(n: OptionNode) {
  const t = tree.value;
  if (!t) return;
  const parent = parentCurveOf(t.nodes, values.value, n.name) ?? null;
  curveEdit.value = { node: n, curve: effectiveCurve(n, values.value, parent) };
}

function onCurveApply(curve: Curve) {
  const e = curveEdit.value;
  const d = selected.value;
  curveEdit.value = null;
  if (e && d?.slot) pushEdit({ type: "curve", kind: d.kind, instance: d.slot, node: e.node.name, value: curveValue(curve) });
}

// --- live input --------------------------------------------------------------

// The picked device's axes, unfiltered (`axis-raw`, held on while the mode
// is up): a joystick axis -1..1 by the game's axis name, a pad stick 0..1
// (max(|x|, |y|) of its pair). `lastLive`: the axis that moved last — it
// takes over once it travelled LIVE_STEP from where it last took over, so
// the noise of a resting axis never steals the curve dialog's dot.
const LIVE_STEP = 0.05;
const live = ref<Record<string, number>>({});
const lastLive = ref<string | null>(null);
let anchors: Record<string, number> = {};
// The last payload per device ("guid#instance"): the stream sends a device
// only when it moves, so a device picked later starts from where it is.
const lastRaw = new Map<string, AxisRaw>();

function replay() {
  const d = selected.value?.sdl;
  const p = d ? lastRaw.get(`${d.sdl_guid}#${d.sdl_instance_id}`) : undefined;
  if (p) takeRaw(p);
}

watch(selectedKey, () => {
  live.value = {};
  lastLive.value = null;
  anchors = {};
  replay();
});

// Every raw axis payload; only the picked device's count.
function takeRaw(p: AxisRaw) {
  lastRaw.set(`${p.guid}#${p.instance_id}`, p);
  const d = selected.value?.sdl;
  if (!d || sourceKey.value || p.guid !== d.sdl_guid || p.instance_id !== d.sdl_instance_id) return;
  const next: Record<string, number> = {};
  if (d.kind === "joystick" && p.kind === "joystick") {
    p.values.forEach((v, i) => {
      const name = d.axes[i];
      if (name && (JOYSTICK_INPUTS as readonly string[]).includes(name)) next[name] = Math.max(-1, Math.min(1, v));
    });
  } else if (d.kind === "gamepad" && p.kind === "gamepad" && p.values.length >= 4) {
    const [lx, ly, rx, ry] = p.values;
    next.thumbl = Math.min(1, Math.max(Math.abs(lx), Math.abs(ly)));
    next.thumbr = Math.min(1, Math.max(Math.abs(rx), Math.abs(ry)));
  } else {
    return;
  }
  for (const [k, v] of Object.entries(next)) {
    const a = anchors[k];
    if (a === undefined) anchors[k] = v;
    else if (Math.abs(v - a) >= LIVE_STEP) {
      anchors[k] = v;
      lastLive.value = k;
    }
  }
  live.value = next;
}

let releaseStream: (() => void) | null = null;
let unlistenRaw: UnlistenFn | null = null;
let unmounted = false;

// The listener first: the stream's first payloads are the snapshot.
onMounted(async () => {
  const un = await listen<AxisRaw>("axis-raw", (e) => takeRaw(e.payload));
  // Left again while the listener was being set up.
  if (unmounted) {
    un();
    return;
  }
  unlistenRaw = un;
  releaseStream = holdAxisStream();
});

onUnmounted(() => {
  unmounted = true;
  unlistenRaw?.();
  unlistenRaw = null;
  releaseStream?.();
  releaseStream = null;
});

// The curve dialog's live dot: the last moved axis after its deadzone and
// saturation (the axis cards' rule, no rescale).
const liveX = computed<number | null>(() => {
  const k = lastLive.value;
  const a = axisRows.value.find((r) => r.input === k);
  if (!k || !a) return null;
  const cls = selected.value?.kind === "gamepad" ? "thumb" : "joystick";
  return Math.abs(axisOutput(cls, live.value[k] ?? 0, a.deadzone, a.saturation));
});

// --- save / discard ------------------------------------------------------------

async function writeChanges(): Promise<boolean> {
  busy.value = true;
  try {
    const s = await invoke<LoadStatus>("save_device_config", { changes: changes.value });
    await loadCurrent();
    edits.value = [];
    emit("saved", s);
    await loadBackups();
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
  if (choice === "discard") edits.value = [];
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
  edits.value = [];
  return true;
}

defineExpose({ requestLeave });

// --- profiles and backups ------------------------------------------------------

const PROFILE_PREFIX = "profile:";
const BACKUP_PREFIX = "backup:";

// The Profiles and Backups panels load their lists (into `profiles` /
// `backups`) and own Save Profile, Import, Export and Create Backup.
const profiles = ref<BindingProfileSummary[]>([]);
const backups = ref<BackupSummary[]>([]);
const profilesPanel = ref<InstanceType<typeof ProfilesPanel> | null>(null);
const backupsPanel = ref<InstanceType<typeof BackupsPanel> | null>(null);

async function loadProfiles() {
  await profilesPanel.value?.load();
}

async function loadBackups() {
  await backupsPanel.value?.load();
}

// The picked profile / backup ("profile:<file>", "backup:<id>"); null = the
// device editor.
const sourceKey = ref<string | null>(null);
const sourceView = ref<DeviceConfigView | null>(null);

function sourceFor(key: string): DiffSource {
  if (key.startsWith(PROFILE_PREFIX)) return { kind: "profile", file: key.slice(PROFILE_PREFIX.length) };
  return { kind: "backup", id: key.slice(BACKUP_PREFIX.length) };
}

const sourceProfile = computed(() => {
  const k = sourceKey.value;
  return k?.startsWith(PROFILE_PREFIX) ? (profiles.value.find((p) => p.file === k.slice(PROFILE_PREFIX.length)) ?? null) : null;
});
const sourceBackup = computed(() => {
  const k = sourceKey.value;
  return k?.startsWith(BACKUP_PREFIX) ? (backups.value.find((b) => b.id === k.slice(BACKUP_PREFIX.length)) ?? null) : null;
});

const sourceName = computed(() => (sourceBackup.value ? stamp(sourceBackup.value.created) : (sourceProfile.value?.name ?? "—")));
const sourceSub = computed(() => {
  if (sourceBackup.value) return `${sourceBackup.value.reason} · ${sourceBackup.value.game_version ?? "—"}`;
  if (sourceProfile.value) return `${sourceProfile.value.file} · ${stamp(sourceProfile.value.modified)}`;
  return "";
});

async function loadSource() {
  const k = sourceKey.value;
  if (!k) {
    sourceView.value = null;
    return;
  }
  try {
    sourceView.value = await invoke<DeviceConfigView>("get_device_config", { source: sourceFor(k) });
  } catch (e) {
    sourceView.value = null;
    emit("notify", String(e), "error");
  }
}

// Compare replaces the editor, so pending edits are settled first.
async function pickSource(key: string) {
  if (key === sourceKey.value) return;
  if (!sourceKey.value && !(await requestLeave())) return;
  sourceKey.value = key;
  sourceView.value = null;
  await loadSource();
}

function pickDevice(key: string) {
  sourceKey.value = null;
  sourceView.value = null;
  selectedKey.value = key;
}

// An import picks the new profile, settling pending edits first.
function afterImport(s: BindingProfileSummary) {
  void pickSource(`${PROFILE_PREFIX}${s.file}`);
}

async function deleteSource() {
  const p = sourceProfile.value;
  const b = sourceBackup.value;
  if (!p && !b) return;
  const choice = await ask(p ? `Delete ${p.name}?` : "Delete backup?", "trash", [
    { label: "Delete", kind: "danger", value: "delete" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice !== "delete") return;
  busy.value = true;
  try {
    if (p) {
      await invoke("delete_binding_profile", { file: p.file });
      await loadProfiles();
    } else if (b) {
      await invoke("delete_backup", { id: b.id });
      await loadBackups();
      emit("notify", "Backup deleted", "ok");
    }
    sourceKey.value = null;
    sourceView.value = null;
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// --- compare -----------------------------------------------------------------

// Find a source's counterpart of a raw Product: spelled the same, else the
// same GUID.
function byProduct<T>(list: T[], name: (x: T) => string | null, raw: string | null, guid: string | null): T | null {
  return list.find((x) => raw !== null && normProduct(name(x)) === normProduct(raw)) ?? list.find((x) => guidMatches(name(x), guid)) ?? null;
}

// Each current device with a slot, against its counterpart in the source:
// the keyboard and the pad by kind, a joystick by its Product.
const compareDevices = computed<CompareDevice[]>(() => {
  const cur = current.value;
  const src = sourceView.value;
  if (!cur || !src) return [];
  const out: CompareDevice[] = [];
  configDevices.value.forEach((d, rank) => {
    if (d.slot === null) return;
    if (d.kind !== "joystick") {
      const block = (v: DeviceConfigView) => v.options.find((b) => b.kind === d.kind && b.instance === 1) ?? null;
      const dev = (v: DeviceConfigView) => (d.kind === "gamepad" ? (v.device_options.find((o) => o.name === GAMEPAD_DEVICE) ?? null) : null);
      out.push({ slot: slotText(d), rank, kind: d.kind, current: block(cur), source: block(src), currentDev: dev(cur), sourceDev: dev(src) });
      return;
    }
    const curBlock = cur.options.find((b) => b.kind === "joystick" && b.instance === d.slot) ?? null;
    const sticks = src.options.filter((b) => b.kind === "joystick");
    const srcBlock: OptionsBlock | null = byProduct(sticks, (b) => b.product, curBlock?.product ?? null, d.guid);
    const curDev: DeviceOptionsView | null = byProduct(cur.device_options, (o) => o.name, curBlock?.product ?? null, d.guid);
    const srcDev: DeviceOptionsView | null = byProduct(src.device_options, (o) => o.name, curDev?.name ?? curBlock?.product ?? null, d.guid);
    out.push({ slot: slotText(d), rank, kind: "joystick", current: curBlock, source: srcBlock, currentDev: curDev, sourceDev: srcDev });
  });
  return out;
});

const rows = computed<CompareRow[]>(() =>
  current.value && sourceView.value ? compareRows(compareDevices.value, trees.value, labels.value, current.value, sourceView.value) : [],
);

// --- apply -----------------------------------------------------------------------

// The current devices the source has a slot for, all ticked: a joystick's
// source slot lands on its current one.
interface ApplyDevice {
  key: string;
  kind: DeviceKind;
  name: string;
  slot: string;
  sel: DeviceSel;
}

const applyDialog = ref<{ devices: ApplyDevice[]; on: Set<string> } | null>(null);

function openApply() {
  const devices: ApplyDevice[] = [];
  compareDevices.value.forEach((c) => {
    const d = configDevices.value[c.rank];
    if (!c.source || !d || d.slot === null) return;
    const sel: DeviceSel =
      d.kind === "joystick"
        ? c.source.instance !== d.slot
          ? { kind: "joystick", instance: c.source.instance, target: d.slot }
          : { kind: "joystick", instance: d.slot }
        : { kind: d.kind, instance: 1 };
    devices.push({ key: d.key, kind: d.kind, name: d.name, slot: c.slot, sel });
  });
  applyDialog.value = { devices, on: new Set(devices.map((d) => d.key)) };
}

function toggleApply(key: string) {
  const d = applyDialog.value;
  if (!d) return;
  const on = new Set(d.on);
  if (!on.delete(key)) on.add(key);
  applyDialog.value = { ...d, on };
}

async function onApply(what: { bindings: boolean; settings: boolean }) {
  const d = applyDialog.value;
  const key = sourceKey.value;
  applyDialog.value = null;
  if (!d || !key) return;
  const source = sourceFor(key);
  const devices = d.devices.filter((x) => d.on.has(x.key)).map((x) => x.sel);
  busy.value = true;
  try {
    const s = await invoke<LoadStatus>("apply_source", { source, devices, bindings: what.bindings, settings: what.settings });
    emit("restored", s);
    // The write takes a safety backup of its own — the list has a new entry.
    await Promise.all([loadBackups(), loadCurrent()]);
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// --- wiring ----------------------------------------------------------------------

// The file changed underneath (save, reload, apply, the game): re-read it;
// the pending edits stay on top.
watch(() => props.bindings, loadCurrent);
// The game data was (re)loaded.
watch(() => props.actionMaps, loadGameData);
// Back from Compare: the live bars start from where the device is.
watch(sourceKey, (k) => {
  if (!k) replay();
});

onMounted(async () => {
  await Promise.all([loadGameData(), loadCurrent()]);
});
</script>

<template>
  <div class="config-view" :style="{ '--left-w': `${leftWidth}px` }">
    <div class="left">
      <!-- devices -->
      <section class="panel fixed" :style="{ height: `${devicesHeight}px` }">
        <div class="head">
          <Icon name="devices" :size="15" />
          <span class="head-title">Devices</span>
          <span class="head-count">{{ configDevices.length }}</span>
        </div>
        <div class="rows scroll">
          <div
            v-for="d in configDevices"
            :key="d.key"
            class="row-item dev"
            :class="{ b: !sourceKey && selected?.key === d.key }"
            @click="pickDevice(d.key)"
          >
            <Icon :name="kindIcon(d.kind)" :size="15" class="dev-icon" />
            <div class="lines">
              <span class="line-title">
                <span class="dev-name">{{ d.name }}</span>
                <span v-if="deviceDirty(d)" class="dirty-dot" title="Unsaved changes" />
              </span>
              <span class="line-sub">{{ subLine(d) }}</span>
            </div>
            <span v-if="d.slot === null" class="no-order" title="no joystick order"><Icon name="warning" :size="15" /></span>
            <span v-else class="slot mono">{{ slotText(d) }}</span>
          </div>
          <div v-if="!configDevices.length" class="row-none">None</div>
        </div>
      </section>

      <Splitter direction="row" @drag="dragDevices" @end="endDrag" @reset="devicesHeight = DEVICES_H.def" />

      <!-- profiles -->
      <ProfilesPanel
        ref="profilesPanel"
        v-model:profiles="profiles"
        v-model:busy="busy"
        class="fixed"
        :style="{ height: `${profilesHeight}px` }"
        :picked="sourceKey?.startsWith(PROFILE_PREFIX) ? sourceKey.slice(PROFILE_PREFIX.length) : null"
        :target="sourceProfile?.file ?? null"
        :canSave="hasCurrent"
        :afterImport="afterImport"
        @notify="(m, t) => emit('notify', m, t)"
        @pick="pickSource(`${PROFILE_PREFIX}${$event}`)"
      />

      <Splitter direction="row" @drag="dragProfiles" @end="endDrag" @reset="profilesHeight = PROFILES_H.def" />

      <!-- backups -->
      <BackupsPanel
        ref="backupsPanel"
        v-model:backups="backups"
        v-model:busy="busy"
        :picked="sourceKey?.startsWith(BACKUP_PREFIX) ? sourceKey.slice(BACKUP_PREFIX.length) : null"
        @notify="(m, t) => emit('notify', m, t)"
        @pick="pickSource(`${BACKUP_PREFIX}${$event}`)"
      />
    </div>

    <Splitter direction="col" @drag="dragLeft" @end="endDrag" @reset="leftWidth = LEFT_W.def" />

    <div class="right">
      <!-- compare: the picked profile / backup against Current -->
      <template v-if="sourceKey">
        <div class="action-tile">
          <div class="tile-title">
            <div class="tile-name">
              <Icon :name="sourceBackup ? 'history' : 'file'" :size="16" />
              <span class="name-text">{{ sourceName }}</span>
            </div>
            <span class="tile-info mono">{{ sourceSub }}</span>
          </div>
          <div class="tile-btns">
            <button type="button" class="btn primary small" :disabled="busy || !hasCurrent || !sourceView" @click="openApply">
              <Icon name="check" :size="14" />
              Apply
            </button>
            <button type="button" class="btn danger small" :disabled="busy" @click="deleteSource">
              <Icon name="trash" :size="14" />
              Delete
            </button>
          </div>
        </div>
        <ConfigCompare v-if="sourceView && current" :rows="rows" :sourceName="sourceName" />
      </template>

      <!-- the picked device's settings -->
      <template v-else-if="selected && hasCurrent">
        <div class="action-tile">
          <div class="tile-title">
            <div class="tile-name">
              <Icon :name="deviceIcon(selected)" :size="16" />
              <span class="name-text">{{ selected.name }}</span>
              <span v-if="selected.slot === null" class="no-order" title="no joystick order"><Icon name="warning" :size="16" /></span>
              <span v-else class="slot mono">{{ slotText(selected) }}</span>
            </div>
            <span class="tile-info">{{ infoLine(selected) }}</span>
          </div>
          <div class="tile-btns">
            <button type="button" class="btn outline small" :disabled="busy || !hasCurrent" @click="backupsPanel?.openCreate()">
              <Icon name="history" :size="14" />
              Create Backup
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

        <!-- no slot: nothing to read or write for this joystick -->
        <section v-if="selected.slot === null" class="panel tile">
          <div class="blocked">
            <Icon name="warning" :size="22" />
            <div>
              <div class="blocked-title">No joystick order</div>
              <div class="blocked-note">Settings unavailable</div>
            </div>
          </div>
        </section>

        <template v-else-if="current">
          <!-- keyboard: the mouse settings -->
          <section v-if="selected.kind === 'keyboard'" class="panel tile">
            <div class="head">
              <Icon name="mouse" :size="16" />
              <span class="head-title">Mouse</span>
            </div>
            <div class="srows">
              <div v-for="r in mouseRows" :key="r.key" class="srow" :class="{ changed: r.changed }">
                <span class="s-label" :title="r.label">{{ r.label }}</span>
                <YesNo v-if="r.yesno" :value="r.shown === 1" :dim="r.notSet" :disabled="r.disabled" @set="r.set($event ? 1 : 0)" />
                <input
                  v-else
                  class="range"
                  :class="{ unset: r.notSet }"
                  type="range"
                  :min="r.min"
                  :max="r.max"
                  :step="r.step"
                  :value="r.shown"
                  :style="{ '--p': fill(r) }"
                  :aria-label="r.label"
                  :disabled="r.disabled"
                  @input="onRange(r, $event)"
                />
                <span class="s-val mono" :class="{ unset: r.notSet }">{{ r.text }}</span>
              </div>
            </div>
          </section>

          <!-- gamepad: sensitivity and the two sticks; joystick: its axes -->
          <section v-if="selected.kind !== 'keyboard'" class="panel tile">
            <div class="head">
              <Icon name="axis" :size="16" />
              <span class="head-title">{{ selected.kind === "gamepad" ? "Thumbsticks" : "Deadzone & Saturation" }}</span>
              <span v-if="!selected.sdl" class="badge-warn">no input</span>
            </div>
            <div v-if="selected.kind === 'gamepad' && padRow" class="srows pad-sens">
              <div class="srow narrow" :class="{ changed: padRow.changed }">
                <span class="s-label" :title="padRow.label">{{ padRow.label }}</span>
                <input
                  class="range"
                  :class="{ unset: padRow.notSet }"
                  type="range"
                  :min="padRow.min"
                  :max="padRow.max"
                  :step="padRow.step"
                  :value="padRow.shown"
                  :style="{ '--p': fill(padRow) }"
                  :aria-label="padRow.label"
                  :disabled="padRow.disabled"
                  @input="onRange(padRow, $event)"
                />
                <span class="s-val mono" :class="{ unset: padRow.notSet }">{{ padRow.text }}</span>
              </div>
            </div>
            <div class="axis-grid">
              <AxisCard
                v-for="a in axisRows"
                :key="a.input"
                :title="a.title"
                :deadzone="a.deadzone"
                :saturation="a.saturation"
                :deadzoneLabel="a.deadzoneLabel"
                :saturationLabel="a.saturationLabel"
                :deadzoneChanged="a.deadzoneChanged"
                :saturationChanged="a.saturationChanged"
                :live="!!selected.sdl"
                :raw="live[a.input] ?? null"
                :pad="selected.kind === 'gamepad'"
                :bound="selected.kind === 'joystick' && selected.sdl ? boundTo(a.input) : null"
                :disabled="!axisDevice"
                @deadzone="setAxis('deadzone', a.input, $event)"
                @saturation="setAxis('saturation', a.input, $event)"
              />
            </div>
          </section>

          <OptionTable
            v-if="tree"
            :key="selected.kind"
            :tree="tree"
            :values="values"
            :saved="savedValues"
            :disabled="!hasBlock"
            @exponent="onExponent"
            @invert="onInvert"
            @resetCurve="onResetCurve"
            @editCurve="onEditCurve"
          />
        </template>
      </template>

      <!-- no live file: nothing to edit, nothing to compare against -->
      <section v-if="!hasCurrent" class="panel tile">
        <div class="blocked">
          <Icon name="warning" :size="22" />
          <div>
            <div class="blocked-title">No bindings</div>
            <div class="blocked-note">Settings unavailable</div>
          </div>
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

    <CurveDialog
      v-if="curveEdit && selected"
      title="Edit Curve"
      :subtitle="`${nodeLabel(curveEdit.node)} · ${selected.name}`"
      :curve="curveEdit.curve"
      :liveX="selected.sdl ? liveX : null"
      @apply="onCurveApply"
      @cancel="curveEdit = null"
    />

    <!-- apply the picked profile / backup to the current devices -->
    <ApplyDialog
      v-if="applyDialog"
      :title="`Apply ${sourceName}`"
      applyLabel="Apply"
      :devicesOk="applyDialog.on.size > 0"
      @apply="onApply"
      @cancel="applyDialog = null"
    >
      <div class="apply-list">
        <span class="apply-head">Devices</span>
        <label v-for="d in applyDialog.devices" :key="d.key" class="check apply-row" :class="{ off: !applyDialog.on.has(d.key) }">
          <input type="checkbox" :checked="applyDialog.on.has(d.key)" @change="toggleApply(d.key)" />
          <Icon :name="kindIcon(d.kind)" :size="15" />
          <span class="check-name">{{ d.name }}</span>
          <span class="slot mono">{{ d.slot }}</span>
        </label>
        <div v-if="!applyDialog.devices.length" class="row-none">None</div>
      </div>
      <p class="dialog-note">{{ RESTART_NOTE }}</p>
    </ApplyDialog>
  </div>
</template>

<style scoped>
/* The splitter is its own 16px column between the two. */
.config-view {
  flex: 1;
  display: grid;
  grid-template-columns: var(--left-w, 300px) 16px minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr);
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
  overflow-y: auto;
}

.panel {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.panel.fixed {
  flex: none;
}

.panel.tile {
  flex-shrink: 0;
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

.tile .head-title {
  flex: none;
}

.head-count {
  font-size: 12px;
  color: var(--text-3);
}

/* --- left column rows (like the Bindings mode) --- */

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

.row-item.b {
  border-color: var(--accent);
}

.dev-icon {
  flex-shrink: 0;
  color: var(--text-2);
}

.lines {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.line-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
  font-size: 13px;
  color: var(--text);
  min-width: 0;
}

.dev-name,
.row-item .line-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.line-sub {
  font-size: 11px;
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* A device with pending changes. */
.dirty-dot {
  width: 7px;
  height: 7px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--warn);
}

.row-none {
  padding: 9px 10px;
  font-size: 13px;
  color: var(--text-3);
}

.slot {
  flex-shrink: 0;
  font-size: 12px;
  padding: 2px 8px;
  border-radius: 3px;
  background: var(--bg-surface-3);
  color: var(--text-2);
  white-space: nowrap;
}

.no-order {
  flex-shrink: 0;
  display: flex;
  color: var(--warn);
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

.btn.danger {
  background: transparent;
  color: var(--err);
  border: 1px solid color-mix(in srgb, var(--err) 60%, transparent);
}

.btn:disabled {
  opacity: 0.4;
  cursor: default;
}

/* --- header tile (like the Bindings mode's action tile) --- */

.action-tile {
  flex-shrink: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px 16px;
  padding: 12px 16px;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
}

.tile-title {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  margin-right: auto;
}

.tile-name {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.name-text {
  font-weight: 600;
  font-size: 16px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tile-info {
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

/* --- no slot --- */

.blocked {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 22px 18px;
  color: var(--warn);
}

.blocked-title {
  font-weight: 600;
  font-size: 14px;
  color: var(--text);
}

.blocked-note {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-3);
}

.badge-warn {
  display: inline-flex;
  align-items: center;
  height: 20px;
  padding: 0 7px;
  border-radius: 3px;
  background: color-mix(in srgb, var(--warn) 18%, transparent);
  color: var(--warn);
  font-size: 12px;
  font-weight: 600;
}

/* --- settings rows (mouse, pad): fixed label column, slider, value --- */

.srows {
  display: flex;
  flex-direction: column;
  padding: 6px 4px 8px;
}

.srows.pad-sens {
  border-bottom: 1px solid var(--border-dim);
}

.srow {
  position: relative;
  display: grid;
  grid-template-columns: 340px 260px 52px minmax(0, 1fr);
  gap: 10px;
  align-items: center;
  min-height: 32px;
  padding: 2px 12px;
  border-radius: 4px;
  font-size: 13px;
}

.srow.narrow {
  grid-template-columns: 170px 260px 52px minmax(0, 1fr);
}

.srow:hover {
  background: var(--bg-surface-2);
}

.srow.changed::before {
  content: "";
  position: absolute;
  left: 2px;
  top: 8px;
  bottom: 8px;
  width: 3px;
  border-radius: 2px;
  background: var(--warn);
}

.s-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.s-val {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.srow.changed .s-val {
  color: var(--warn);
}

/* Not set: the game screen's default, dimmed. */
.s-val.unset {
  color: var(--text-3);
}

.range.unset {
  opacity: 0.55;
}

/* --- axis cards --- */

.axis-grid {
  display: grid;
  gap: 10px;
  grid-template-columns: repeat(auto-fill, minmax(270px, 1fr));
  padding: 10px;
}

/* --- dialogs --- */

.dialog-note {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
}

.apply-list {
  display: flex;
  flex-direction: column;
}

.apply-head {
  padding: 2px 4px 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-2);
}

.apply-row {
  padding: 8px 4px;
}

.apply-row + .apply-row {
  border-top: 1px solid var(--border-dim);
}

.apply-row.off .check-name,
.apply-row.off .slot {
  opacity: 0.45;
}

.check-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Own checkbox look (mirrors SettingsDialog): WebKitGTK would paint GTK's. */
.check {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
  cursor: pointer;
  user-select: none;
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
</style>
