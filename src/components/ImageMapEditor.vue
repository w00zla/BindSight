<script setup lang="ts">
// Devices mode: pick a device, pick one of its image-maps, press a physical
// input and draw the shapes that belong to it. Konva does the canvas work;
// everything is stored normalized (0..1) in the image-map.
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { KonvaEventObject, Node as KonvaNode } from "konva/lib/Node";
import type { Stage } from "konva/lib/Stage";
import type { Arc } from "konva/lib/shapes/Arc";
import type { Ellipse } from "konva/lib/shapes/Ellipse";
import type { Wedge } from "konva/lib/shapes/Wedge";
import type { Transformer } from "konva/lib/shapes/Transformer";
import type { VueKonvaRef } from "vue-konva";
import Icon, { type IconName } from "./Icon.vue";
import Dropdown, { type DropdownOption } from "./Dropdown.vue";
import ConfirmDialog, { type ConfirmButton, type ConfirmIcon } from "./ConfirmDialog.vue";
import Splitter from "./Splitter.vue";
import ColumnHead from "./ColumnHead.vue";
import { collator, sortRows, useTableColumns, type ColumnSpec } from "../tableColumns";
import type { ClashReport, DeviceInfo, JoyInput, LoggedInput, ToastType } from "../types";
import { AXIS_PRESS, deviceIcon, deviceName } from "../devices";
import { KEY_COUNT, MOUSE_INPUTS, recording } from "../keyboard";
import { persistedRef } from "../persist";
import { NAME_MAX, sanitizeName, stripNameChars } from "../names";
import { colourAlpha, colourHex, composeColour, cssVar, parseHex, readPalette, rgbaToHex } from "../colour";
import {
  rectRadiusPx,
  symbolPx,
  pathData,
  inputKey,
  sameHardware,
  shapeImageFiles,
  type ArcGeometry,
  type Geometry,
  type ImageMap,
  type RectGeometry,
  type WedgeGeometry,
  type ImageMapSummary,
  type Shape,
  type SymbolKind,
} from "../imagemap";

// `keyInput`: the last key captured in the webview — the backend never sees
// keys, so App hands them over instead of an event.
const props = defineProps<{
  devices: DeviceInfo[];
  events: LoggedInput[];
  keyInput: JoyInput | null;
  // The image-map the Monitor shows for a device (the user's pick or the default).
  chosenMapId: (d: DeviceInfo) => string | null;
  // SC's label for an input token; echoes the token when there is none.
  tokenLabel: (token: string | null) => string;
  // The game does not see the device (SDL lists it, the game's enumeration
  // does not): it is left out of the image-map list (its maps show under
  // "unused"), Device Info still has it and says so.
  isUnseen: (d: DeviceInfo) => boolean;
  // The device-order report: which jsN the game gives each joystick.
  clash: ClashReport | null;
  // App version, OS, toolkit versions and keyboard layout, the head of every dump.
  systemLine: string;
}>();
const emit = defineEmits<{
  notify: [message: string, type: ToastType];
  saved: [];
  clearLog: [];
  choose: [hardwareId: string | null, id: string];
}>();

// No active tool == select/move mode. The image tool never stays active:
// it picks a file and places it at once. The text tool places its text as
// a path shape (glyph outlines from the backend, `text_path`).
type Tool = "rect" | "ellipse" | "polygon" | "arc" | "wedge" | "image" | "text" | SymbolKind;
// `divider`: a separator before the tool (image and text stand apart from
// the drawn shapes).
const TOOLS: { tool: Tool; icon: IconName; title: string; divider?: boolean }[] = [
  { tool: "rect", icon: "shape-rect", title: "Rectangle" },
  { tool: "ellipse", icon: "shape-ellipse", title: "Ellipse" },
  { tool: "polygon", icon: "shape-polygon", title: "Polygon" },
  { tool: "arc", icon: "shape-arc", title: "Arc" },
  { tool: "wedge", icon: "shape-wedge", title: "Wedge" },
  { tool: "arrow", icon: "shape-arrow", title: "Arrow" },
  { tool: "arrow2", icon: "shape-arrow2", title: "Double arrow" },
  { tool: "rotate", icon: "shape-rotate", title: "Rotation" },
  { tool: "image", icon: "shape-image", title: "Image", divider: true },
  { tool: "text", icon: "shape-text", title: "Text", divider: true },
];
// The text tool's input: what the next click places, in which of the
// system's font families (listed by the backend when the tool is first
// used), regular or bold. The map stores only the outlines, so the font
// choice is this machine's business alone; it is remembered.
const TEXT_MAX_LEN = 256;
const textInput = ref("");
const textFamily = persistedRef<string>("bindsight.editor.textFamily", "");
const textBold = persistedRef<boolean>("bindsight.editor.textBold", false);
const fontFamilies = ref<string[] | null>(null);
const fontOptions = computed<DropdownOption[]>(() => (fontFamilies.value ?? []).map((f) => ({ value: f, label: f })));

async function loadFonts() {
  if (fontFamilies.value) return;
  try {
    const list = await invoke<string[]>("list_fonts");
    fontFamilies.value = list;
    if (!list.includes(textFamily.value)) textFamily.value = list[0] ?? "";
  } catch (e) {
    fontFamilies.value = [];
    emit("notify", `Fonts unavailable: ${e}`, "error");
  }
}
const SYMBOLS: SymbolKind[] = ["arrow", "arrow2", "rotate"];

// A new symbol is 5% of the image width, square on screen; a new image
// shape 15% of the image width, keeping its own aspect; a new text 4% of
// the image height, as wide as its text.
const DEFAULT_SYMBOL_W = 0.05;
const DEFAULT_TEXT_H = 0.04;
const DEFAULT_IMAGE_W = 0.15;
const NEW_ARC = { inner: 0.6, angle: 270, rotation: 135 };
const NEW_WEDGE = { angle: 90, rotation: 225 };
const MIN_DRAW_PX = 4;
const ZOOMS = [0.5, 0.67, 0.8, 1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4];

// --- colours ---------------------------------------------------------------

// Konva needs literal colours; the tokens are the only place they are defined.
function withAlpha(colour: string, alpha: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(colour);
  if (!m) return colour;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}


// Filled once at mount — the stage only exists after an image has loaded.
const paint = ref({
  live: "",
  shapeStroke: "",
  shapeFill: "",
  anchor: "",
  tipBg: "",
  tipText: "",
});

// Swatches of the colour picker: the default lit colour first (clicking it
// drops the shape's own colour), then the `--shape-palette` token.
const swatches = ref<string[]>([]);

function readPaint() {
  const live = cssVar("--live");
  paint.value = {
    live,
    shapeStroke: cssVar("--shape-stroke"),
    shapeFill: cssVar("--shape-fill"),
    anchor: cssVar("--text"),
    tipBg: withAlpha(cssVar("--bg-base"), 0.85),
    tipText: cssVar("--text"),
  };
  const stroke = paint.value.shapeStroke.toLowerCase();
  swatches.value = [stroke, ...readPalette().filter((c) => c !== stroke)];
}

// --- device / image-map selection -----------------------------------------

// The selection is a connected device (by SDL GUID) or, with "Show unused
// image-maps" on, a hardware id no connected device has. Exactly one is set.
const selectedGuid = ref("");
const selectedUnusedHw = ref("");
// The devices the image-map list offers: the connected ones the game sees.
const listed = computed(() => props.devices.filter((d) => !props.isUnseen(d)));
const device = computed(() => listed.value.find((d) => d.sdl_guid === selectedGuid.value) ?? null);
// Hardware id the listed image-maps belong to.
const selectedHw = computed(() => device.value?.hardware_id ?? (selectedUnusedHw.value || null));

const summaries = ref<ImageMapSummary[]>([]);

function mapsFor(d: DeviceInfo): ImageMapSummary[] {
  return summaries.value.filter((s) => sameHardware(s.hardware_id, d.hardware_id));
}

// Listed alphabetically by name — bundled and user maps in one order.
const deviceMaps = computed(() =>
  selectedHw.value
    ? summaries.value
        .filter((s) => sameHardware(s.hardware_id, selectedHw.value))
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }))
    : [],
);

// Image-maps whose device is not connected, one entry per hardware id,
// listed after the devices while "Show unused image-maps" is on. Icon and
// name come from the maps themselves (a keyboard or pad by its literal id).
interface UnusedGroup {
  hw: string;
  name: string;
  icon: "keyboard" | "gamepad" | "devices";
  maps: ImageMapSummary[];
}
const showUnused = ref(false);
const unusedGroups = computed<UnusedGroup[]>(() => {
  const by = new Map<string, UnusedGroup>();
  for (const s of summaries.value) {
    if (listed.value.some((d) => sameHardware(d.hardware_id, s.hardware_id))) continue;
    const key = s.hardware_id.toLowerCase();
    let g = by.get(key);
    if (!g) {
      g = {
        hw: s.hardware_id,
        name: s.hardware_name || s.hardware_id,
        icon: key === "keyboard" ? "keyboard" : key === "gamepad" ? "gamepad" : "devices",
        maps: [],
      };
      by.set(key, g);
    }
    g.maps.push(s);
  }
  return [...by.values()].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
});
const selectedUnused = computed(() => unusedGroups.value.find((g) => sameHardware(g.hw, selectedUnusedHw.value)) ?? null);

const selectedName = computed(() => (device.value ? deviceName(device.value) : (selectedUnused.value?.name ?? "—")));

const openId = ref("");
const map = ref<ImageMap | null>(null);
// Serialized state as last loaded/saved, for the dirty check.
const savedJson = ref("");
const dirty = computed(() => !!map.value && JSON.stringify(map.value) !== savedJson.value);
const openSummary = computed(() => summaries.value.find((s) => s.id === openId.value) ?? null);
// Bundled image-maps are read-only, and so is everything without a map.
const locked = computed(() => !map.value || openSummary.value?.source === "bundled");

// A map opens read-only; "edit" is entered from the action tile, a fresh clone
// or a fresh new map. "new" is the map that does not exist yet (no image yet).
type EditorState = "view" | "edit" | "new";
const state = ref<EditorState>("view");
const editing = computed(() => state.value === "edit" && !locked.value);
const isNew = computed(() => state.value === "new");
// A device (or unused hardware id) is selected but no image-map is open and
// none is being created: the canvas column becomes one placeholder tile
// offering "New Image-Map", the shapes column stays hidden.
const noMap = computed(() => !showDeviceInfo.value && !map.value && !isNew.value && !!selectedHw.value);
// The map the Monitor shows for the selected device is the one open here.
const isChosen = computed(
  () => !!device.value && !!openId.value && props.chosenMapId(device.value) === openId.value,
);

// --- canvas state ----------------------------------------------------------

const shapes = computed<Shape[]>(() => map.value?.shapes ?? []);

// Loaded elements of the image shapes' files, by file name (Konva draws an
// image node from an element).
const shapeImgs = ref<Record<string, HTMLImageElement>>({});

const imgEl = ref<HTMLImageElement | null>(null);
const natW = ref(0);
const natH = ref(0);

const stageBox = ref<HTMLElement | null>(null);
const boxW = ref(0);
const boxH = ref(0);
const zoom = ref(1);
// Base size = the image fitted into the body; zoom scales it from there.
const fit = computed(() =>
  natW.value && natH.value && boxW.value && boxH.value
    ? Math.min(boxW.value / natW.value, boxH.value / natH.value)
    : 0,
);
const W = computed(() => Math.round(natW.value * fit.value * zoom.value));
const H = computed(() => Math.round(natH.value * fit.value * zoom.value));

const tool = ref<Tool | null>(null);
const selectMode = computed(() => tool.value === null);
const canDrag = computed(() => selectMode.value && editing.value);
const selectedId = ref<string | null>(null);
const selectedShape = computed(() => shapes.value.find((a) => a.id === selectedId.value) ?? null);
// Arcs and wedges stay circular under the transformer, text keeps its
// aspect.
const keepRatio = computed(() => {
  const k = selectedShape.value?.geometry.kind;
  return k === "arc" || k === "wedge" || k === "path";
});
const hover = ref<{ x: number; y: number; text: string } | null>(null);

// --- undo / redo -----------------------------------------------------------

// Snapshots of the shape list, one per settled change: a deep watch on the
// shapes, debounced so a slider drag or held arrow keys make one step, not
// fifty. The image, the name and hidden shapes are not history (view, not
// content). Undo / redo restore a snapshot without recording it. Cleared
// when a map opens or edit mode ends; a save keeps it (the dirty check
// compares against the saved state, not the history).
const HISTORY_MAX = 100;
const HISTORY_DEBOUNCE_MS = 250;
// Past snapshots, the last one is the current state.
const undoStack = ref<string[]>([]);
const redoStack = ref<string[]>([]);
let historyTimer: ReturnType<typeof setTimeout> | null = null;
let restoring = false;
const canUndo = computed(() => undoStack.value.length > 1);
const canRedo = computed(() => redoStack.value.length > 0);

function resetHistory() {
  if (historyTimer) clearTimeout(historyTimer);
  historyTimer = null;
  undoStack.value = map.value ? [JSON.stringify(map.value.shapes)] : [];
  redoStack.value = [];
}

function recordHistory() {
  const m = map.value;
  if (!m) return;
  const snap = JSON.stringify(m.shapes);
  if (snap === undoStack.value[undoStack.value.length - 1]) return;
  undoStack.value.push(snap);
  if (undoStack.value.length > HISTORY_MAX + 1) undoStack.value.shift();
  redoStack.value = [];
}

// A pending debounced change becomes a step now (so an undo right after a
// change undoes that change, not the one before).
function flushHistory() {
  if (!historyTimer) return;
  clearTimeout(historyTimer);
  historyTimer = null;
  recordHistory();
}

watch(
  () => map.value?.shapes,
  () => {
    if (restoring || !editing.value) return;
    if (historyTimer) clearTimeout(historyTimer);
    historyTimer = setTimeout(() => {
      historyTimer = null;
      recordHistory();
    }, HISTORY_DEBOUNCE_MS);
  },
  { deep: true },
);

function applySnapshot(snap: string) {
  const m = map.value;
  if (!m) return;
  restoring = true;
  m.shapes = JSON.parse(snap) as Shape[];
  if (selectedId.value && !m.shapes.some((a) => a.id === selectedId.value)) selectedId.value = null;
  // The deep watch runs before the next tick; only then may it record again.
  void nextTick(() => {
    restoring = false;
  });
}

function undo() {
  flushHistory();
  if (!canUndo.value) return;
  const current = undoStack.value.pop();
  if (current !== undefined) redoStack.value.push(current);
  applySnapshot(undoStack.value[undoStack.value.length - 1]);
}

function redo() {
  flushHistory();
  const next = redoStack.value.pop();
  if (next === undefined) return;
  undoStack.value.push(next);
  applySnapshot(next);
}

// The recorded input new shapes go to. Only Record changes it; it is dropped
// whenever the map, the device, the view or the edit mode changes.
const currentKey = ref<string | null>(null);
// Headline of the input card: the input's native name. No SC token or
// label — an image-map does not know which jsN its device is.
const currentText = computed(() => (currentKey.value ? keyName(currentKey.value) : ""));
// The line under the Record button: can shapes be drawn right now?
const inputStatus = computed<{ kind: "warn" | "ok"; icon: IconName; text: string }>(() => {
  if (!device.value) return { kind: "warn", icon: "warning", text: "Device not connected" };
  if (!currentKey.value) return { kind: "warn", icon: "warning", text: "Record an input before adding shapes" };
  return { kind: "ok", icon: "check", text: "Ready to add shapes" };
});

function dropInput() {
  currentKey.value = null;
  recording.value = false;
}

// Widths of the device list and of the input / shapes column, each dragged
// at the splitter next to it.
const LEFT_W = { min: 240, max: 520, def: 300 };
const leftWidth = persistedRef<number>("bindsight.devices.leftWidth", LEFT_W.def);
let leftStart: number | null = null;
function dragLeft(delta: number) {
  leftStart ??= leftWidth.value;
  leftWidth.value = Math.min(LEFT_W.max, Math.max(LEFT_W.min, Math.round(leftStart + delta)));
}
function endDragLeft() {
  leftStart = null;
  measureBox?.();
}
const RIGHT_W = { min: 300, max: 700, def: 400 };
const rightWidth = persistedRef<number>("bindsight.devices.rightWidth", RIGHT_W.def);
let rightStart: number | null = null;
// Refitting the canvas re-lays out every shape; while the splitter moves
// the canvas keeps its size and is fitted once on release.
let measureBox: (() => void) | null = null;
function dragRight(delta: number) {
  rightStart ??= rightWidth.value;
  rightWidth.value = Math.min(RIGHT_W.max, Math.max(RIGHT_W.min, Math.round(rightStart + delta)));
}
function endDragRight() {
  rightStart = null;
  measureBox?.();
}

// --- shapes table ------------------------------------------------------------

// One bucket per input (always grouped), the same table as the bindings.
const COLUMNS: ColumnSpec[] = [
  { key: "input", label: "INPUT", width: 140, icon: "bolt" },
  { key: "shape", label: "SHAPE", width: null, icon: "shape" },
];
const cols = useTableColumns("bindsight.columns.shapes", COLUMNS, { key: "input", dir: "asc" });

const filter = ref("");

interface Bucket {
  key: string;
  text: string;
  rows: Shape[];
}

// An input key's native name, as the Device Events tile writes it:
// `button:3` -> "button 3", `hat:0:up` -> "hat 0 up", `axis:2` -> "axis 2
// (rotz)", `key:lshift` -> "key lshift", `pad:a` -> "pad a".
function keyName(key: string): string {
  const [kind, a, b] = key.split(":");
  if (kind === "axis") {
    const sc = device.value?.axes[Number(a)];
    return `axis ${a}${sc ? ` (${sc})` : ""}`;
  }
  return [kind, a, b].filter((x) => x !== undefined).join(" ");
}

// Sorting by INPUT orders the buckets, by SHAPE the rows inside each bucket
// (buckets then stay in natural input order).
const buckets = computed<Bucket[]>(() => {
  const f = filter.value.trim().toLowerCase();
  const by = new Map<string, Bucket>();
  for (const a of shapes.value) {
    let b = by.get(a.input);
    if (!b) {
      b = { key: a.input, text: keyName(a.input), rows: [] };
      by.set(a.input, b);
    }
    b.rows.push(a);
  }
  const sort = cols.sort.value;
  const list = [...by.values()].filter((b) => !f || b.key.toLowerCase().includes(f) || b.text.toLowerCase().includes(f));
  const dir = sort.key === "input" && sort.dir === "desc" ? -1 : 1;
  list.sort((x, y) => (collator.compare(x.text, y.text) || collator.compare(x.key, y.key)) * dir);
  if (sort.key === "shape") {
    for (const b of list) b.rows = sortRows(b.rows, sort, (a) => shapeKind(a));
  }
  return list;
});

// Shapes hidden on the canvas while working (editor state only, never
// saved): by shape id, dropped whenever another map opens.
const hidden = ref(new Set<string>());
const visibleShapes = computed(() => shapes.value.filter((a) => !hidden.value.has(a.id)));
const anyHidden = computed(() => shapes.value.some((a) => hidden.value.has(a.id)));
function toggleHidden(id: string) {
  const s = new Set(hidden.value);
  if (!s.delete(id)) s.add(id);
  hidden.value = s;
}
// Show everything again when anything is hidden, else hide everything.
function toggleAllHidden() {
  hidden.value = anyHidden.value ? new Set() : new Set(shapes.value.map((a) => a.id));
}

// Expanded buckets; everything starts collapsed. A filter forces them open,
// and so does recording an input or selecting one of a bucket's shapes.
const expanded = ref(new Set<string>());
function isOpen(b: Bucket): boolean {
  return !!filter.value.trim() || expanded.value.has(b.key);
}
function toggleBucket(key: string) {
  const s = new Set(expanded.value);
  if (!s.delete(key)) s.add(key);
  expanded.value = s;
}
function openBucket(key: string) {
  if (!expanded.value.has(key)) expanded.value = new Set(expanded.value).add(key);
}
const allExpanded = computed(() => buckets.value.length > 0 && buckets.value.every((b) => expanded.value.has(b.key)));
function expandAll() {
  expanded.value = new Set(buckets.value.map((b) => b.key));
}
function collapseAll() {
  expanded.value = new Set();
}
watch(currentKey, (k) => {
  if (k) openBucket(k);
});
watch(selectedId, (id) => {
  const a = shapes.value.find((x) => x.id === id);
  if (a) openBucket(a.input);
});

// Drag-drawn rect/ellipse in progress, in stage pixels.
const draft = ref<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
// Polygon under construction, flat [x0, y0, ...] in stage pixels.
const draftPoly = ref<number[]>([]);
const pointer = ref<{ x: number; y: number } | null>(null);

const stageRef = ref<VueKonvaRef<Stage> | null>(null);
const trRef = ref<VueKonvaRef<Transformer> | null>(null);
const nameInput = ref<HTMLInputElement | null>(null);

let areaSeq = 0;
function newId(): string {
  areaSeq += 1;
  return `a${Date.now().toString(36)}${areaSeq}`;
}

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

// True when it is fine to drop what is on screen: not dirty, or the user chose
// Discard, or the save went through.
async function requestLeave(): Promise<boolean> {
  if (!dirty.value) return true;
  const choice = await ask("Unsaved changes", "save", [
    { label: "Discard", kind: "danger", value: "discard" },
    { label: "Save", kind: "primary", value: "save" },
    { label: "Keep Editing", kind: "outline", value: "keep" },
  ]);
  if (choice === "keep") return false;
  if (choice === "save") return await saveMap();
  return true;
}

defineExpose({ requestLeave });

// --- loading ---------------------------------------------------------------

async function loadSummaries() {
  try {
    summaries.value = await invoke<ImageMapSummary[]>("list_imagemaps");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function loadMap(id: string) {
  try {
    const m = await invoke<ImageMap>("get_imagemap", { id });
    map.value = m;
    openId.value = m.id;
    savedJson.value = JSON.stringify(m);
  } catch (e) {
    map.value = null;
    openId.value = "";
    savedJson.value = "";
    emit("notify", String(e), "error");
  }
  selectedId.value = null;
  tool.value = null;
  zoom.value = 1;
  hidden.value = new Set();
  dropInput();
  // A map always opens read-only; the callers that want the editor say so.
  state.value = "view";
  resetHistory();
}

function closeMap() {
  map.value = null;
  openId.value = "";
  savedJson.value = "";
  selectedId.value = null;
  tool.value = null;
  dropInput();
  state.value = "view";
}

// Data URLs of image-map images, keyed by `<map id>/<file>`.
const imgCache = new Map<string, string>();
async function imageUrl(id: string, file: string): Promise<string> {
  const key = `${id}/${file}`;
  const hit = imgCache.get(key);
  if (hit) return hit;
  const url = await invoke<string>("read_imagemap_image", { id, file });
  imgCache.set(key, url);
  return url;
}

async function loadCanvasImage() {
  imgEl.value = null;
  natW.value = 0;
  natH.value = 0;
  const m = map.value;
  if (!m) return;
  try {
    const url = await imageUrl(m.id, m.image.file);
    const el = new Image();
    el.onload = () => {
      natW.value = el.naturalWidth;
      natH.value = el.naturalHeight;
      imgEl.value = el;
    };
    el.onerror = () => emit("notify", `Cannot decode ${m.image.file}`, "error");
    el.src = url;
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// Keep an element loaded for every image shape's file of the open map.
watch(
  () => (map.value ? shapeImageFiles(map.value) : []),
  (files) => {
    const m = map.value;
    if (!m) return;
    for (const file of files) {
      if (shapeImgs.value[file]) continue;
      void imageUrl(m.id, file).then((url) => {
        const el = new Image();
        el.onload = () => {
          shapeImgs.value = { ...shapeImgs.value, [file]: el };
        };
        el.src = url;
      });
    }
  },
  { immediate: true },
);

// Breathing room between the canvas and the panel edges (`.stage-box`
// padding); clientWidth includes it, so the fit must not.
const STAGE_PAD = 16;

// The canvas only exists once an image-map is open; follow the element.
let ro: ResizeObserver | null = null;
watch(stageBox, (el) => {
  ro?.disconnect();
  ro = null;
  if (!el) return;
  const measure = () => {
    boxW.value = el.clientWidth - 2 * STAGE_PAD;
    boxH.value = el.clientHeight - 2 * STAGE_PAD;
  };
  measureBox = measure;
  measure();
  ro = new ResizeObserver(() => {
    if (rightStart === null && leftStart === null) measure();
  });
  ro.observe(el);
});

watch([() => map.value?.id, () => map.value?.image.file], () => {
  cancelDraw();
  shapeImgs.value = {};
  loadCanvasImage();
});

// Keep the device selection on something that exists — unless the open map
// has unsaved changes: the map does not depend on its device being plugged
// in, and a hot-plug must not throw the edits away.
watch(
  listed,
  (list) => {
    if (list.some((d) => d.sdl_guid === selectedGuid.value)) return;
    // An unused hardware id whose device just connected: same maps, now
    // under the device.
    const arrived = selectedUnusedHw.value ? list.find((d) => sameHardware(d.hardware_id, selectedUnusedHw.value)) : null;
    if (arrived) {
      selectedGuid.value = arrived.sdl_guid;
      selectedUnusedHw.value = "";
      return;
    }
    if (selectedUnusedHw.value || dirty.value) return;
    selectedGuid.value = list.find((d) => d.hardware_id)?.sdl_guid ?? "";
    currentKey.value = null;
    recording.value = false;
    closeMap();
    void openFirst();
  },
  { immediate: true },
);

// --- device / map switching -----------------------------------------------

async function openFirst() {
  const first = deviceMaps.value[0]?.id;
  if (first) await loadMap(first);
}

async function selectDevice(d: DeviceInfo) {
  showDeviceInfo.value = false;
  if (!d.hardware_id || d.sdl_guid === selectedGuid.value) return;
  if (!(await requestLeave())) return;
  selectedGuid.value = d.sdl_guid;
  selectedUnusedHw.value = "";
  closeMap();
  await openFirst();
}

async function selectUnused(g: UnusedGroup) {
  showDeviceInfo.value = false;
  if (sameHardware(g.hw, selectedUnusedHw.value)) return;
  if (!(await requestLeave())) return;
  selectedGuid.value = "";
  selectedUnusedHw.value = g.hw;
  closeMap();
  await openFirst();
}

// Hiding the unused entries while one is selected moves the selection to
// the first device (after the usual unsaved-changes question).
async function toggleShowUnused() {
  if (showUnused.value && selectedUnusedHw.value) {
    if (!(await requestLeave())) return;
    selectedUnusedHw.value = "";
    selectedGuid.value = listed.value.find((d) => d.hardware_id)?.sdl_guid ?? "";
    closeMap();
    await openFirst();
  }
  showUnused.value = !showUnused.value;
}

async function openMap(id: string) {
  showDeviceInfo.value = false;
  if (id === openId.value) return;
  if (!(await requestLeave())) return;
  await loadMap(id);
}

function focusName() {
  nextTick(() => {
    nameInput.value?.focus();
    nameInput.value?.select();
  });
}

// --- image-map actions -----------------------------------------------------

// Pick an image file, or null when the dialog was cancelled.
async function pickImage(): Promise<string | null> {
  const src = await open({
    multiple: false,
    filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp"] }],
  });
  return src ?? null;
}

// The empty slot for a map that does not exist yet: nothing is loaded, the
// image is still missing. "Choose image" turns it into a real map.
async function newMap() {
  if (!selectedHw.value) return;
  showDeviceInfo.value = false;
  if (!(await requestLeave())) return;
  closeMap();
  state.value = "new";
}

async function createMap() {
  const hw = selectedHw.value;
  if (!hw) return;
  try {
    const imagePath = await pickImage();
    if (!imagePath) return;
    const m = await invoke<ImageMap>("create_imagemap", {
      name: sanitizeName(selectedName.value, "image-map"),
      hardwareId: hw,
      hardwareName: device.value?.sc_name ?? selectedUnused.value?.name ?? "",
      imagePath,
    });
    await loadSummaries();
    await loadMap(m.id);
    state.value = "edit";
    focusName();
    emit("notify", "Image-map created", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// One button, two jobs: create the map around its image, or swap the image of
// the map being edited.
async function chooseImage() {
  if (isNew.value) await createMap();
  else await replaceImage();
}

async function cloneMap(s: ImageMapSummary) {
  if (!(await requestLeave())) return;
  try {
    const m = await invoke<ImageMap>("clone_imagemap", { id: s.id, name: `${s.name} copy` });
    await loadSummaries();
    await loadMap(m.id);
    state.value = "edit";
    focusName();
    emit("saved");
    emit("notify", "Image-map cloned", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

function startEdit() {
  if (locked.value) return;
  state.value = "edit";
  resetHistory();
}

function useForDevice() {
  const d = device.value;
  if (!d || !openId.value || isChosen.value) return;
  emit("choose", d.hardware_id, openId.value);
}

// Leaving edit mode: the same Save/Discard/Cancel question as switching away,
// but a discard has to put the saved state back on screen.
// Back to view mode; the caller has settled the changes.
function leaveEdit() {
  state.value = "view";
  selectedId.value = null;
  tool.value = null;
  zoom.value = 1;
  dropInput();
  cancelDraw();
  resetHistory();
}

// Cancel drops the unsaved changes without asking — that is what the button
// says.
async function cancelEdit() {
  if (dirty.value && openId.value) await loadMap(openId.value);
  leaveEdit();
}

async function finishEdit() {
  if (dirty.value && !(await saveMap())) return;
  leaveEdit();
}

async function deleteMap(s: ImageMapSummary) {
  const choice = await ask("Delete image-map?", "trash", [
    { label: "Delete", kind: "danger", value: "delete" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice !== "delete") return;
  try {
    await invoke("delete_imagemap", { id: s.id });
    for (const key of [...imgCache.keys()]) if (key.startsWith(`${s.id}/`)) imgCache.delete(key);
    const wasOpen = s.id === openId.value;
    if (wasOpen) closeMap();
    await loadSummaries();
    if (wasOpen) await openFirst();
    emit("saved");
    emit("notify", "Image-map deleted", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function saveMap(): Promise<boolean> {
  const m = map.value;
  if (!m || locked.value) return true;
  m.name = sanitizeName(m.name, "image-map");
  try {
    const saved = await invoke<ImageMap>("save_imagemap", { map: m });
    map.value = saved;
    savedJson.value = JSON.stringify(saved);
    await loadSummaries();
    emit("saved");
    emit("notify", "Image-map saved", "ok");
    return true;
  } catch (e) {
    emit("notify", String(e), "error");
    return false;
  }
}

// Export any image-map by id: the open one (Export button) or an unused one.
async function exportMap(id: string, name: string) {
  try {
    const dest = await save({
      defaultPath: `${name || "image-map"}.zip`,
      filters: [{ name: "Image-map", extensions: ["zip"] }],
    });
    if (!dest) return;
    await invoke("export_imagemap", { id, destPath: dest });
    emit("notify", "Image-map exported", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function importMap() {
  if (!(await requestLeave())) return;
  try {
    const src = await open({ multiple: false, filters: [{ name: "Image-map", extensions: ["zip"] }] });
    if (!src) return;
    const s = await invoke<ImageMapSummary>("import_imagemap", { sourcePath: src });
    imgCache.clear();
    await loadSummaries();
    const d = listed.value.find((dev) => sameHardware(dev.hardware_id, s.hardware_id));
    if (d) {
      selectedGuid.value = d.sdl_guid;
      selectedUnusedHw.value = "";
    } else {
      showUnused.value = true;
      selectedGuid.value = "";
      selectedUnusedHw.value = s.hardware_id;
    }
    await loadMap(s.id);
    emit("saved");
    emit("notify", `Imported ${s.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// Replace the image-map's image. The shapes stay (a re-shot of the same view
// keeps them useful); the old file is removed once the new one is in.
async function replaceImage() {
  const m = map.value;
  if (!m || locked.value || !editing.value) return;
  try {
    const src = await pickImage();
    if (!src) return;
    const old = m.image.file;
    const img = await invoke<{ file: string; label: string }>("add_imagemap_image", {
      id: m.id,
      sourcePath: src,
    });
    m.image = img;
    if (old !== img.file) {
      await invoke("remove_imagemap_image", { id: m.id, file: old });
      imgCache.delete(`${m.id}/${old}`);
    }
    // The file is on disk already — keep imagemap.json in step with it.
    await saveMap();
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// --- live input capture ----------------------------------------------------

let unlisten: UnlistenFn[] = [];

// Record takes the next input of the selected device as the current one
// (mouse included, see keyboard.ts); an axis counts once it is pressed
// past AXIS_PRESS, like in the rebind dialog. Escape (a plain key event,
// never an input) stops a recording, see `onEditorKey`.
function takeInput(ev: JoyInput) {
  if (!recording.value || ev.guid !== selectedGuid.value) return;
  if ((ev.kind === "axis" || ev.kind === "padaxis") && Math.abs(ev.value) < AXIS_PRESS) return;
  const key = inputKey(ev);
  if (!key) return;
  currentKey.value = key;
  recording.value = false;
}

// Recording ends with editing.
watch(editing, (on) => {
  if (!on) recording.value = false;
});

// Keys arrive as a prop (App captures them), joystick and pad events directly.
watch(
  () => props.keyInput,
  (ev) => {
    if (ev) takeInput(ev);
  },
);

onMounted(async () => {
  readPaint();
  window.addEventListener("keydown", onEditorKey);
  unlisten.push(await listen<JoyInput>("joy-input", (e) => takeInput(e.payload)));
  await loadSummaries();
  if (!openId.value) await openFirst();
});

onUnmounted(() => {
  window.removeEventListener("keydown", onEditorKey);
  unlisten.forEach((fn) => fn());
  unlisten = [];
  recording.value = false;
  ro?.disconnect();
  ro = null;
});

// --- shapes ----------------------------------------------------------------

function addShape(geometry: Geometry) {
  const m = map.value;
  if (!m || !currentKey.value) return;
  const shape: Shape = { id: newId(), input: currentKey.value, geometry };
  m.shapes.push(shape);
  selectedId.value = shape.id;
  tool.value = null;
}

function setTool(t: Tool) {
  if (locked.value || !editing.value) return;
  cancelDraw();
  if (t === "image") {
    tool.value = null;
    void addImageShape();
    return;
  }
  // Clicking the active tool turns it off — no tool == select/move.
  tool.value = tool.value === t ? null : t;
  if (tool.value) selectedId.value = null;
  if (tool.value === "text") void loadFonts();
}

// Image tool: pick a file, copy it into the map folder and place it at the
// image centre, keeping its aspect. A file no shape references any more is
// removed at the next save (backend side).
async function addImageShape() {
  const m = map.value;
  if (!m || locked.value || !editing.value) return;
  if (!currentKey.value) {
    emit("notify", "Press an input first", "error");
    return;
  }
  try {
    const src = await pickImage();
    if (!src) return;
    const img = await invoke<{ file: string; label: string }>("add_imagemap_image", { id: m.id, sourcePath: src });
    const url = await imageUrl(m.id, img.file);
    const el = await new Promise<HTMLImageElement>((resolve, reject) => {
      const e = new Image();
      e.onload = () => resolve(e);
      e.onerror = () => reject(new Error(`Cannot decode ${img.file}`));
      e.src = url;
    });
    shapeImgs.value = { ...shapeImgs.value, [img.file]: el };
    const w = DEFAULT_IMAGE_W;
    // Same pixel aspect as the file, expressed in image-relative units.
    const h = ((w * natW.value * el.naturalHeight) / el.naturalWidth) / natH.value;
    addShape({ kind: "image", file: img.file, x: 0.5, y: 0.5, w, h, rotation: 0 });
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

function cancelDraw() {
  draft.value = null;
  draftPoly.value = [];
}

async function deleteShape(id: string) {
  const m = map.value;
  if (!m || locked.value || !editing.value) return;
  const choice = await ask("Delete shape?", "trash", [
    { label: "Delete", kind: "danger", value: "delete" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice !== "delete") return;
  m.shapes = m.shapes.filter((a) => a.id !== id);
  if (selectedId.value === id) selectedId.value = null;
}

// A row in the list, or a shape on the canvas: both select the shape. The
// recorded input stays what it is — only Record changes it.
function pickShape(a: Shape) {
  selectedId.value = a.id;
}

// Shift a shape by stage pixels.
function moveShape(a: Shape, dxPx: number, dyPx: number) {
  const dx = dxPx / W.value;
  const dy = dyPx / H.value;
  const s = a.geometry;
  if (s.kind === "polygon") {
    s.points = s.points.map(([x, y]) => [x + dx, y + dy] as [number, number]);
  } else if (s.kind === "ellipse" || s.kind === "arc" || s.kind === "wedge") {
    s.cx += dx;
    s.cy += dy;
  } else {
    s.x += dx;
    s.y += dy;
  }
}

// A copy of the shape, offset a little, under the recorded input when one
// is recorded (that is how a shape is cloned to another input), else under
// the same one; the copy is selected.
function duplicateShape(a: Shape) {
  const m = map.value;
  if (!m || locked.value || !editing.value) return;
  const copy: Shape = { ...JSON.parse(JSON.stringify(a)), id: newId(), input: currentKey.value ?? a.input };
  moveShape(copy, 12, 12);
  m.shapes.push(copy);
  selectedId.value = copy.id;
}

// Keyboard editing: Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y undo and redo; on the
// selected shape arrows nudge (Shift: 10 px), Delete / Backspace delete,
// Ctrl+D duplicates; Escape stops a recording, else deselects or drops the
// tool. Text fields and open dialogs keep their keys; while recording, the
// other keys are the input being recorded.
const NUDGE_PX = 1;
const NUDGE_SHIFT_PX = 10;

function onEditorKey(e: KeyboardEvent) {
  if (e.key === "Escape" && recording.value) {
    recording.value = false;
    return;
  }
  if (!editing.value || recording.value || confirm.value || document.querySelector('[role="dialog"]')) return;
  const t = e.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
  if (e.key === "Escape") {
    if (tool.value) {
      setTool(tool.value);
    } else if (selectedId.value) {
      selectedId.value = null;
    }
    return;
  }
  if ((e.ctrlKey || e.metaKey) && (e.key === "z" || e.key === "Z" || e.key === "y" || e.key === "Y")) {
    if (e.key === "y" || e.key === "Y" || e.shiftKey) redo();
    else undo();
    e.preventDefault();
    return;
  }
  const a = selectedShape.value;
  if (!a) return;
  const step = e.shiftKey ? NUDGE_SHIFT_PX : NUDGE_PX;
  switch (e.key) {
    case "ArrowUp":
      moveShape(a, 0, -step);
      break;
    case "ArrowDown":
      moveShape(a, 0, step);
      break;
    case "ArrowLeft":
      moveShape(a, -step, 0);
      break;
    case "ArrowRight":
      moveShape(a, step, 0);
      break;
    case "Delete":
    case "Backspace":
      void deleteShape(a.id);
      break;
    case "d":
    case "D":
      if (!(e.ctrlKey || e.metaKey)) return;
      duplicateShape(a);
      break;
    default:
      return;
  }
  e.preventDefault();
}

// The kind's tool icon.
function shapeIcon(a: Shape): IconName {
  const g = a.geometry;
  if (g.kind === "path") return "shape-text";
  return g.kind === "symbol" ? `shape-${g.symbol}` : `shape-${g.kind}`;
}

function shapeKind(a: Shape): string {
  const g = a.geometry;
  if (g.kind === "symbol") return g.symbol === "arrow2" ? "double arrow" : g.symbol === "rotate" ? "rotation" : g.symbol;
  if (g.kind === "path") return "text";
  return g.kind;
}

// --- shape panel ------------------------------------------------------------

type ColourRole = "stroke" | "fill";

// The colour a role shows: the shape's own, else the default lit colour.
function roleColour(a: Shape, role: ColourRole): string {
  const own = a[role];
  if (own) return own;
  if (role === "stroke") return paint.value.shapeStroke;
  // The fill token is an rgba(); its hex form with alpha for the picker.
  return rgbaToHex(paint.value.shapeFill);
}

function setRoleHex(a: Shape, role: ColourRole, hex: string) {
  const full = parseHex(hex);
  if (!full) return;
  a[role] = composeColour(full, colourAlpha(roleColour(a, role)));
}

// The first swatch: back to the token colour (no colour stored).
function pickSwatch(a: Shape, role: ColourRole, i: number, c: string) {
  if (i === 0) delete a[role];
  else setRoleHex(a, role, c);
}

function setRoleAlpha(a: Shape, role: ColourRole, alpha: number) {
  a[role] = composeColour(colourHex(roleColour(a, role)), alpha);
}

function onHexInput(a: Shape, role: ColourRole, e: Event) {
  setRoleHex(a, role, (e.target as HTMLInputElement).value);
}

function onAlphaInput(a: Shape, role: ColourRole, e: Event) {
  setRoleAlpha(a, role, Number((e.target as HTMLInputElement).value));
}

function onRangeInput(e: Event, apply: (v: number) => void) {
  apply(Number((e.target as HTMLInputElement).value));
}

// --- zoom ------------------------------------------------------------------

// Ctrl + wheel zooms a step; a plain wheel keeps scrolling the box.
function onWheel(e: WheelEvent) {
  if (!e.ctrlKey || !e.deltaY) return;
  e.preventDefault();
  zoomStep(e.deltaY < 0 ? 1 : -1);
}

function zoomStep(dir: number) {
  const i = ZOOMS.indexOf(zoom.value);
  const next = Math.min(ZOOMS.length - 1, Math.max(0, (i < 0 ? ZOOMS.indexOf(1) : i) + dir));
  zoom.value = ZOOMS[next];
}

// --- drawing ---------------------------------------------------------------

function stagePointer(): { x: number; y: number } | null {
  const stage = stageRef.value?.getStage();
  return stage?.getPointerPosition() ?? null;
}

// Drag the canvas around inside its scrolling box: the middle button or
// Shift + left button anywhere (caught before Konva sees the press, so no
// shape drag or draw starts), the left button on the bare image (select
// mode) or on the box around the canvas.
let pan: { x: number; y: number; left: number; top: number } | null = null;

function startPan(ev: MouseEvent) {
  const box = stageBox.value;
  if (!box) return;
  ev.preventDefault();
  pan = { x: ev.clientX, y: ev.clientY, left: box.scrollLeft, top: box.scrollTop };
  box.classList.add("panning");
  const move = (e: MouseEvent) => {
    if (!pan) return;
    box.scrollLeft = pan.left - (e.clientX - pan.x);
    box.scrollTop = pan.top - (e.clientY - pan.y);
  };
  const up = () => {
    pan = null;
    box.classList.remove("panning");
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}

function onBoxMouseDown(ev: MouseEvent) {
  if ((ev.target as HTMLElement | null)?.closest(".shape-panel")) return;
  const onBox = ev.target === stageBox.value;
  if (ev.button === 1 || (ev.button === 0 && (ev.shiftKey || onBox))) {
    ev.stopPropagation();
    startPan(ev);
  }
}

function onStageMouseDown(e: KonvaEventObject<MouseEvent>) {
  if (!W.value) return;
  const pos = stagePointer();
  if (!pos) return;
  if (selectMode.value) {
    // A click on the bare image clears the selection and starts a pan.
    if (e.target === e.target.getStage() || e.target.name() === "bg") {
      selectedId.value = null;
      startPan(e.evt);
    }
    return;
  }
  if (locked.value) return;
  if (!currentKey.value) {
    emit("notify", "Press an input first", "error");
    return;
  }
  if (tool.value === "rect" || tool.value === "ellipse" || tool.value === "arc" || tool.value === "wedge") {
    draft.value = { x0: pos.x, y0: pos.y, x1: pos.x, y1: pos.y };
  } else if (tool.value === "polygon") {
    draftPoly.value = [...draftPoly.value, pos.x, pos.y];
  } else if (tool.value === "text") {
    void addTextShape(pos.x / W.value, pos.y / H.value);
  } else if (tool.value && SYMBOLS.includes(tool.value as SymbolKind)) {
    addShape({
      kind: "symbol",
      symbol: tool.value as SymbolKind,
      x: pos.x / W.value,
      y: pos.y / H.value,
      w: DEFAULT_SYMBOL_W,
      h: (DEFAULT_SYMBOL_W * W.value) / H.value,
      rotation: 0,
    });
  }
}

// The text tool's click: the backend turns the text into path data in the
// 100x100 box and tells its aspect; the shape is DEFAULT_TEXT_H high.
async function addTextShape(x: number, y: number) {
  const text = textInput.value;
  if (!text.trim()) {
    emit("notify", "Type a text first", "error");
    return;
  }
  if (!textFamily.value) {
    emit("notify", "Choose a font first", "error");
    return;
  }
  try {
    const { d, aspect } = await invoke<{ d: string; aspect: number }>("text_path", {
      text,
      family: textFamily.value,
      bold: textBold.value,
    });
    const h = DEFAULT_TEXT_H;
    addShape({ kind: "path", d, x, y, w: (h * H.value * aspect) / W.value, h, rotation: 0 });
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

function onStageMouseMove() {
  const pos = stagePointer();
  pointer.value = pos;
  if (draft.value && pos) {
    draft.value.x1 = pos.x;
    draft.value.y1 = pos.y;
  }
}

function onStageMouseUp() {
  const d = draft.value;
  draft.value = null;
  if (!d || !W.value) return;
  const x = Math.min(d.x0, d.x1);
  const y = Math.min(d.y0, d.y1);
  const w = Math.abs(d.x1 - d.x0);
  const h = Math.abs(d.y1 - d.y0);
  if (w < MIN_DRAW_PX || h < MIN_DRAW_PX) return;
  const cx = (x + w / 2) / W.value;
  const cy = (y + h / 2) / H.value;
  if (tool.value === "rect") {
    addShape({ kind: "rect", x: x / W.value, y: y / H.value, w: w / W.value, h: h / H.value, rotation: 0, radius: 0 });
  } else if (tool.value === "ellipse") {
    addShape({ kind: "ellipse", cx, cy, rx: w / 2 / W.value, ry: h / 2 / H.value, rotation: 0 });
  } else if (tool.value === "arc") {
    // The dragged box's inscribed circle.
    addShape({ kind: "arc", cx, cy, r: Math.min(w, h) / 2 / W.value, ...NEW_ARC });
  } else if (tool.value === "wedge") {
    addShape({ kind: "wedge", cx, cy, r: Math.min(w, h) / 2 / W.value, ...NEW_WEDGE });
  }
}

function commitPolygon() {
  const pts = draftPoly.value;
  if (pts.length < 6 || !W.value || !editing.value) return;
  const points: [number, number][] = [];
  for (let i = 0; i < pts.length; i += 2) {
    // A double-click lands two mousedowns on the same spot — drop the repeat.
    const last = points[points.length - 1];
    if (last && Math.abs(last[0] * W.value - pts[i]) < 2 && Math.abs(last[1] * H.value - pts[i + 1]) < 2) continue;
    points.push([pts[i] / W.value, pts[i + 1] / H.value]);
  }
  draftPoly.value = [];
  if (points.length < 3) return;
  addShape({ kind: "polygon", points });
}

// --- shape configs ---------------------------------------------------------

// Shapes draw in their own lit colours (the tokens by default); the
// recorded input's shapes at full opacity, the rest faded; the selected one
// gets a dashed live-coloured outline on top.
function colours(a: Shape) {
  const isCurrent = a.input === currentKey.value;
  const selected = a.id === selectedId.value;
  return {
    stroke: selected ? paint.value.live : roleColour(a, "stroke"),
    strokeWidth: selected ? 3 : 2,
    fill: roleColour(a, "fill"),
    opacity: isCurrent || selected ? 1 : 0.45,
    dash: selected ? [6, 3] : undefined,
  };
}

function rectCfg(a: Shape) {
  if (a.geometry.kind !== "rect") return {};
  const s = a.geometry;
  const w = s.w * W.value;
  const h = s.h * H.value;
  // Placed at its centre with a half-size offset, so `rotation` turns around
  // the centre while the stored x/y stay the unrotated box's top-left.
  return {
    id: a.id,
    x: s.x * W.value + w / 2,
    y: s.y * H.value + h / 2,
    width: w,
    height: h,
    offsetX: w / 2,
    offsetY: h / 2,
    cornerRadius: rectRadiusPx(s, W.value, H.value),
    rotation: s.rotation,
    draggable: canDrag.value,
    ...colours(a),
  };
}

function ellipseCfg(a: Shape) {
  if (a.geometry.kind !== "ellipse") return {};
  const s = a.geometry;
  // Konva ellipses already draw around their origin — no offset needed.
  return {
    id: a.id,
    x: s.cx * W.value,
    y: s.cy * H.value,
    radiusX: s.rx * W.value,
    radiusY: s.ry * H.value,
    rotation: s.rotation,
    draggable: canDrag.value,
    ...colours(a),
  };
}

function polyCfg(a: Shape) {
  if (a.geometry.kind !== "polygon") return {};
  return {
    id: a.id,
    x: 0,
    y: 0,
    points: a.geometry.points.flatMap(([x, y]) => [x * W.value, y * H.value]),
    closed: true,
    draggable: canDrag.value,
    ...colours(a),
  };
}

function symbolCfg(a: Shape) {
  if (a.geometry.kind !== "symbol" && a.geometry.kind !== "path") return {};
  const s = a.geometry;
  // 100 path units == w * image width by h * image height.
  const px = symbolPx(s, W.value, H.value);
  return {
    id: a.id,
    data: pathData(s),
    x: px.x,
    y: px.y,
    offsetX: 50,
    offsetY: 50,
    scaleX: px.scaleX,
    scaleY: px.scaleY,
    rotation: s.rotation,
    draggable: canDrag.value,
    ...colours(a),
    strokeScaleEnabled: false,
  };
}

function arcCfg(a: Shape) {
  if (a.geometry.kind !== "arc") return {};
  const s = a.geometry;
  const r = s.r * W.value;
  return {
    id: a.id,
    x: s.cx * W.value,
    y: s.cy * H.value,
    innerRadius: r * s.inner,
    outerRadius: r,
    angle: s.angle,
    rotation: s.rotation,
    draggable: canDrag.value,
    ...colours(a),
  };
}

function wedgeCfg(a: Shape) {
  if (a.geometry.kind !== "wedge") return {};
  const s = a.geometry;
  return {
    id: a.id,
    x: s.cx * W.value,
    y: s.cy * H.value,
    radius: s.r * W.value,
    angle: s.angle,
    rotation: s.rotation,
    draggable: canDrag.value,
    ...colours(a),
  };
}

// An image shape has no colours of its own; it fades like the others.
function imageCfg(a: Shape) {
  if (a.geometry.kind !== "image") return {};
  const s = a.geometry;
  const w = s.w * W.value;
  const h = s.h * H.value;
  return {
    id: a.id,
    image: shapeImgs.value[s.file],
    x: s.x * W.value,
    y: s.y * H.value,
    width: w,
    height: h,
    offsetX: w / 2,
    offsetY: h / 2,
    rotation: s.rotation,
    draggable: canDrag.value,
    opacity: colours(a).opacity,
    stroke: a.id === selectedId.value ? paint.value.live : undefined,
    strokeWidth: 2,
    dash: [6, 3],
  };
}

// --- shape edits -----------------------------------------------------------

function onShapeClick(a: Shape) {
  if (!selectMode.value) return;
  pickShape(a);
}

function onShapeEnter(a: Shape) {
  const pos = stagePointer();
  hover.value = pos ? { x: pos.x + 10, y: pos.y + 10, text: keyName(a.input) } : null;
}

function onShapeLeave() {
  hover.value = null;
}

function onDragEnd(a: Shape, e: KonvaEventObject<DragEvent>) {
  const node = e.target;
  const s = a.geometry;
  if (s.kind === "rect") {
    s.x = (node.x() - (s.w * W.value) / 2) / W.value;
    s.y = (node.y() - (s.h * H.value) / 2) / H.value;
  } else if (s.kind === "ellipse" || s.kind === "arc" || s.kind === "wedge") {
    s.cx = node.x() / W.value;
    s.cy = node.y() / H.value;
  } else if (s.kind === "symbol" || s.kind === "image" || s.kind === "path") {
    s.x = node.x() / W.value;
    s.y = node.y() / H.value;
  } else {
    const dx = node.x() / W.value;
    const dy = node.y() / H.value;
    s.points = s.points.map(([x, y]) => [x + dx, y + dy] as [number, number]);
    node.position({ x: 0, y: 0 });
  }
}

function onTransformEnd(a: Shape, e: KonvaEventObject<Event>) {
  const node = e.target;
  const s = a.geometry;
  const sx = node.scaleX();
  const sy = node.scaleY();
  if (s.kind === "polygon") {
    // The points are stage coordinates: bake the node's transform into them
    // and put the node back at the origin.
    const m = node.getTransform();
    s.points = s.points.map(([x, y]) => {
      const p = m.point({ x: x * W.value, y: y * H.value });
      return [p.x / W.value, p.y / H.value] as [number, number];
    });
    node.setAttrs({ x: 0, y: 0, rotation: 0, scaleX: 1, scaleY: 1 });
    return;
  }
  if (s.kind === "rect") {
    const w = Math.max(1, node.width() * sx);
    const h = Math.max(1, node.height() * sy);
    node.scaleX(1);
    node.scaleY(1);
    node.width(w);
    node.height(h);
    node.offsetX(w / 2);
    node.offsetY(h / 2);
    s.w = w / W.value;
    s.h = h / H.value;
    s.x = (node.x() - w / 2) / W.value;
    s.y = (node.y() - h / 2) / H.value;
    s.rotation = node.rotation();
  } else if (s.kind === "ellipse") {
    const el = node as unknown as Ellipse;
    const rx = Math.max(1, el.radiusX() * sx);
    const ry = Math.max(1, el.radiusY() * sy);
    node.scaleX(1);
    node.scaleY(1);
    el.radiusX(rx);
    el.radiusY(ry);
    s.rx = rx / W.value;
    s.ry = ry / H.value;
    s.cx = node.x() / W.value;
    s.cy = node.y() / H.value;
    s.rotation = node.rotation();
  } else if (s.kind === "symbol" || s.kind === "path") {
    s.w = (sx * 100) / W.value;
    s.h = (sy * 100) / H.value;
    s.x = node.x() / W.value;
    s.y = node.y() / H.value;
    s.rotation = node.rotation();
  } else if (s.kind === "image") {
    const w = Math.max(1, node.width() * sx);
    const h = Math.max(1, node.height() * sy);
    node.scaleX(1);
    node.scaleY(1);
    node.width(w);
    node.height(h);
    node.offsetX(w / 2);
    node.offsetY(h / 2);
    s.w = w / W.value;
    s.h = h / H.value;
    s.x = node.x() / W.value;
    s.y = node.y() / H.value;
    s.rotation = node.rotation();
  } else if (s.kind === "arc") {
    // Circular (keepRatio): one scale factor for both radii.
    const arc = node as unknown as Arc;
    const r = Math.max(1, arc.outerRadius() * sx);
    node.scaleX(1);
    node.scaleY(1);
    arc.outerRadius(r);
    arc.innerRadius(r * s.inner);
    s.r = r / W.value;
    s.cx = node.x() / W.value;
    s.cy = node.y() / H.value;
    s.rotation = node.rotation();
  } else if (s.kind === "wedge") {
    const wedge = node as unknown as Wedge;
    const r = Math.max(1, wedge.radius() * sx);
    node.scaleX(1);
    node.scaleY(1);
    wedge.radius(r);
    s.r = r / W.value;
    s.cx = node.x() / W.value;
    s.cy = node.y() / H.value;
    s.rotation = node.rotation();
  }
}

// Polygon vertex anchors (select mode, polygon selected).
const vertexAnchors = computed(() => {
  const a = selectedShape.value;
  if (!a || a.geometry.kind !== "polygon" || !canDrag.value || hidden.value.has(a.id)) return [];
  return a.geometry.points.map(([x, y], i) => ({ i, x: x * W.value, y: y * H.value }));
});

function onVertexDrag(i: number, e: KonvaEventObject<DragEvent>) {
  const a = selectedShape.value;
  if (!a || a.geometry.kind !== "polygon") return;
  a.geometry.points[i] = [e.target.x() / W.value, e.target.y() / H.value];
}

// Attach/detach the transformer whenever the selection or the shapes change.
watch(
  [selectedId, canDrag, shapes, W],
  async () => {
    await nextTick();
    const tr = trRef.value?.getNode();
    const stage = stageRef.value?.getStage();
    if (!tr || !stage) return;
    const a = selectedShape.value;
    if (!a || !canDrag.value) {
      tr.nodes([]);
      return;
    }
    const node = stage.findOne<KonvaNode>(`#${a.id}`);
    tr.nodes(node ? [node] : []);
  },
  { deep: true },
);

const drawPreview = computed(() => {
  const d = draft.value;
  if (!d) return null;
  return {
    x: Math.min(d.x0, d.x1),
    y: Math.min(d.y0, d.y1),
    width: Math.abs(d.x1 - d.x0),
    height: Math.abs(d.y1 - d.y0),
  };
});

const polyPreview = computed(() => {
  if (!draftPoly.value.length) return null;
  const pts = [...draftPoly.value];
  if (pointer.value) pts.push(pointer.value.x, pointer.value.y);
  return pts;
});

// --- left column text ------------------------------------------------------

// --- Device Info (Device List + Device Events, replaces the canvas) ---------

const showDeviceInfo = ref(false);

// Device Info replaces the map on screen, so unsaved changes are settled
// first; a discard puts the saved state back, like Cancel does.
async function toggleDeviceInfo() {
  if (!showDeviceInfo.value) {
    if (!(await requestLeave())) return;
    if (dirty.value) await cancelEdit();
  }
  dropInput();
  showDeviceInfo.value = !showDeviceInfo.value;
}

// The device an event came from: SDL GUID plus instance id — two sticks of
// the same type share the GUID, the instance id tells them apart.
function deviceOfEvent(ev: JoyInput): DeviceInfo | undefined {
  return props.devices.find((dev) => dev.sdl_guid === ev.guid && dev.sdl_instance_id === ev.instance_id);
}

function nameOfEvent(ev: JoyInput): string {
  const d = deviceOfEvent(ev);
  return d ? deviceName(d) : ev.guid;
}

// Pad buttons the backend derives from an axis (a trigger past its hold, a
// stick direction, both triggers), marked in the log to tell them from real ones.
const DERIVED_PAD = new Set([
  "triggerl_btn",
  "triggerr_btn",
  "triggerl_r_btn",
  "thumbl_left",
  "thumbl_right",
  "thumbl_up",
  "thumbl_down",
  "thumbr_left",
  "thumbr_right",
  "thumbr_up",
  "thumbr_down",
]);

// Event as one line: the SC axis name comes from the device's derived axes.
function eventText(ev: JoyInput): string {
  switch (ev.kind) {
    case "button":
      return `button ${ev.index} ${ev.pressed ? "down" : "up"}`;
    case "axis": {
      const sc = deviceOfEvent(ev)?.axes[ev.index];
      return `axis ${ev.index}${sc ? ` (${sc})` : ""} = ${ev.value} (${(ev.value / 32767).toFixed(3)})`;
    }
    case "padbutton":
      return `pad ${ev.name}${DERIVED_PAD.has(ev.name) ? " (derived)" : ""} ${ev.pressed ? "down" : "up"}`;
    case "padaxis":
      return `pad ${ev.name} = ${ev.value} (${(ev.value / 32767).toFixed(3)})`;
    case "key":
      return `key ${ev.name} ${ev.pressed ? "down" : "up"}`;
    default:
      return `hat ${ev.index} ${ev.direction} (raw ${ev.raw})`;
  }
}

// Wall-clock time of a logged event, HH:MM:SS.mmm.
function clock(at: number): string {
  const d = new Date(at);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

function axesText(d: DeviceInfo): string {
  if (d.axes.length) return d.axes.join(" ");
  return d.axes_error ? `error: ${d.axes_error}` : "—";
}

const hex4 = (n: number) => n.toString(16).padStart(4, "0");

// Collapse runs of consecutive buttons: "Btn1 Btn2 … Btn128" -> "Btn1-128".
function compactUsages(usages: string[]): string {
  const out: string[] = [];
  let i = 0;
  while (i < usages.length) {
    const m = /^Btn(\d+)$/.exec(usages[i]);
    if (!m) {
      out.push(usages[i++]);
      continue;
    }
    const first = Number(m[1]);
    let last = first;
    let j = i + 1;
    while (j < usages.length && usages[j] === `Btn${last + 1}`) {
      last++;
      j++;
    }
    out.push(last > first ? `Btn${first}-${last}` : `Btn${first}`);
    i = j;
  }
  return out.join(" ");
}

// The jsN the game gives a joystick (its own order from Game.log), and how
// that relates to the slot saved in the bindings file.
function gameSlotText(d: DeviceInfo): string {
  const r = props.clash;
  if (!r) return "—";
  if (r.order_error) return "order unknown";
  const guid = d.sc_product_guid?.toLowerCase();
  const slot = r.connected.find((s) => !!guid && s.sc_product_guid?.toLowerCase() === guid);
  if (!slot) return "not in the game's device list";
  if (slot.stored_instance === null) return `js${slot.effective_instance} · not saved in the bindings`;
  if (slot.stored_instance !== slot.effective_instance) return `js${slot.effective_instance} · saved as js${slot.stored_instance} (clash)`;
  return `js${slot.effective_instance} · saved`;
}

// Key/value rows of everything known about a device. The keyboard is a
// synthetic device (the mouse is part of it, as in the game) — it has
// nothing but its name, its hardware id and what the capture knows.
function deviceRows(d: DeviceInfo): [string, string][] {
  if (d.kind === "keyboard") {
    return [
      ["kind", d.kind],
      ["sdl name", d.sdl_name],
      ["hardware id", d.hardware_id ?? "—"],
      ["game slot", "kb1"],
      ["io", `${KEY_COUNT} keys · mouse ${MOUSE_INPUTS.join(" ")}`],
    ];
  }
  return [
    ["kind", d.kind === "gamepad" ? `gamepad · slot ${d.gamepad_slot ?? "—"} · ${d.controller_name ?? "—"}` : d.kind],
    ["game slot", d.kind === "gamepad" ? (d.gamepad_slot !== null ? `gp${d.gamepad_slot}` : "none (a further pad)") : gameSlotText(d)],
    ["seen by game", props.isUnseen(d) ? "no (SDL lists it, the game's enumeration does not)" : "yes"],
    ["game name", d.sc_name ?? "— (no HID product string)"],
    ["sdl name", d.sdl_name],
    ["sdl guid", d.sdl_guid],
    ["game product", d.sc_product_guid ?? "—"],
    ["hardware id", d.hardware_id ?? "—"],
    ["sdl", `index ${d.index} · instance ${d.sdl_instance_id} · type ${d.sdl_type} · path ${d.sdl_path ?? "—"}`],
    ["usb", `vid ${hex4(d.sdl_vendor)} · pid ${hex4(d.sdl_product)} · version ${hex4(d.sdl_product_version)} · serial ${d.sdl_serial || "—"} · power ${d.power_level}`],
    [
      "io",
      `${d.num_buttons} buttons · ${d.num_axes} axes · ${d.num_hats} hats · ${d.num_balls} balls · rumble ${d.has_rumble ? "yes" : "no"} · led ${d.has_led ? "yes" : "no"}`,
    ],
    ["game axes", axesText(d)],
    ["hid usages", d.hid_usages.length ? compactUsages(d.hid_usages) : "—"],
    ...d.hid_interfaces.map(
      (h, i): [string, string] => [
        `hid #${i}`,
        `if ${h.interface_number} · usage ${h.usage_page}/${h.usage} · ${h.bus_type} · release ${hex4(h.release)}` +
          ` · mfr ${h.manufacturer ?? "—"} · product ${h.product ?? "—"} · serial ${h.serial ?? "—"} · ${h.path}`,
      ],
    ),
    ["hid descriptor", d.hid_descriptor ?? "—"],
  ];
}

// SC side of an event: "js2_button5 · Button 5 (Input 2)", the bare token
// when SC has no label for it, "—" when SC cannot bind the input.
function tokenText(ev: LoggedInput): string {
  if (!ev.token) return "—";
  const label = props.tokenLabel(ev.token);
  return label === ev.token ? ev.token : `${ev.token} · ${label}`;
}

function eventLine(ev: LoggedInput): string {
  const d = deviceOfEvent(ev);
  return `${clock(ev.at)} t${ev.timestamp}  #${d?.index ?? "?"} i${ev.instance_id} ${nameOfEvent(ev)}  ${eventText(ev)}  ${tokenText(ev)}`;
}

// Text dump of the Device List tile.
function listText(): string {
  const lines = [`BindSight device list ${new Date().toISOString()}`, props.systemLine, ""];
  for (const d of props.devices) {
    lines.push(`#${d.index} ${deviceName(d)}`);
    for (const [k, v] of deviceRows(d)) lines.push(`    ${k.padEnd(15)} ${v}`);
  }
  if (!props.devices.length) lines.push("    none");
  return lines.join("\n") + "\n";
}

// Text dump of the Device Events tile, newest first.
function eventsText(): string {
  const lines = [`BindSight device events ${new Date().toISOString()} (newest first)`, props.systemLine, ""];
  for (const ev of props.events) lines.push(eventLine(ev));
  if (!props.events.length) lines.push("    none");
  return lines.join("\n") + "\n";
}

// Save one tile's dump via the save dialog. The text is built after the
// dialog closes, so events that arrived meanwhile are included.
async function saveText(name: string, text: () => string, done: string) {
  try {
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const dest = await save({
      defaultPath: `bindsight-${name}-${stamp}.txt`,
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (!dest) return;
    await invoke("write_text_file", { path: dest, text: text() });
    emit("notify", done, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

function deviceLine(d: DeviceInfo): string {
  if (d.kind === "gamepad" && d.gamepad_slot === null) return "no slot";
  const parts: string[] = [];
  if (d.kind === "keyboard") {
    parts.push(`${KEY_COUNT} keys`, `${MOUSE_INPUTS.length} buttons`);
  } else {
    if (d.num_buttons) parts.push(`${d.num_buttons} btns`);
    if (d.num_axes) parts.push(`${d.num_axes} axes`);
    if (d.num_hats) parts.push(`${d.num_hats} hats`);
  }
  if (!d.hardware_id) parts.push("no game id");
  return parts.join(" · ");
}

// A device that has no image-map yet, shown as a chip.
function noMaps(d: DeviceInfo): boolean {
  return !!d.hardware_id && mapsFor(d).length === 0;
}
</script>

<template>
  <section
    class="devices"
    :class="{ 'device-info': showDeviceInfo, 'no-map': noMap }"
    :style="{ '--left-w': `${leftWidth}px`, '--right-w': `${rightWidth}px` }"
  >
    <!-- left: devices and their image-maps, and the system panel -->
    <aside class="col-left">
      <section class="panel grow">
        <div class="head">
          <Icon name="image" :size="15" />
          <span class="head-title">Image-Maps</span>
        </div>
        <div class="dev-list">
          <template v-for="d in listed" :key="d.index">
            <div
              class="dev"
              :class="{ on: d.sdl_guid === selectedGuid, dim: !d.hardware_id }"
              @click="selectDevice(d)"
            >
              <div class="dev-name">
                <Icon :name="deviceIcon(d)" :size="15" class="dev-kind" />
                <span>{{ deviceName(d) }}</span>
              </div>
              <div class="dev-line">
                {{ deviceLine(d) }}
                <span v-if="noMaps(d)" class="chip small">No image-map</span>
              </div>
            </div>

            <div v-if="d.sdl_guid === selectedGuid && d.hardware_id" class="maps">
              <div
                v-for="s in deviceMaps"
                :key="s.id"
                class="map"
                :class="{ open: s.id === openId }"
                @click="openMap(s.id)"
              >
                <span class="map-name">{{ s.name }}</span>
                <span class="map-mark" :title="s.id === props.chosenMapId(d) ? 'Shown in Monitor' : undefined">
                  <Icon v-if="s.id === props.chosenMapId(d)" name="check" :size="15" />
                </span>
                <span v-if="s.source === 'bundled'" class="ro" title="Read-only">
                  <Icon name="lock" :size="13" />
                </span>
              </div>
              <button type="button" class="map-new" @click="newMap">
                <Icon name="plus" :size="13" />
                <span>New Image-Map</span>
              </button>
            </div>
          </template>

          <!-- image-maps without a connected device, as entries of their own -->
          <template v-if="showUnused">
            <template v-for="g in unusedGroups" :key="g.hw">
              <div class="dev unused" :class="{ on: sameHardware(g.hw, selectedUnusedHw) }" :title="g.hw" @click="selectUnused(g)">
                <div class="dev-name">
                  <Icon :name="g.icon" :size="15" class="dev-kind" />
                  <span>{{ g.name }}</span>
                </div>
                <div class="dev-line">Not connected</div>
              </div>
              <div v-if="sameHardware(g.hw, selectedUnusedHw)" class="maps">
                <div
                  v-for="s in deviceMaps"
                  :key="s.id"
                  class="map"
                  :class="{ open: s.id === openId }"
                  @click="openMap(s.id)"
                >
                  <span class="map-name">{{ s.name }}</span>
                  <span v-if="s.source === 'bundled'" class="ro" title="Read-only">
                    <Icon name="lock" :size="13" />
                  </span>
                </div>
                <button type="button" class="map-new" @click="newMap">
                  <Icon name="plus" :size="13" />
                  <span>New Image-Map</span>
                </button>
              </div>
            </template>
          </template>
        </div>
        <label class="check">
          <input type="checkbox" :checked="showUnused" @change="toggleShowUnused" />
          Show unused image-maps
        </label>
        <div class="foot">
          <button type="button" class="btn outline wide" @click="importMap">
            <Icon name="download" :size="14" />
            Import
          </button>
          <button type="button" class="btn outline wide" :disabled="!map" @click="map && exportMap(map.id, map.name)">
            <Icon name="upload" :size="14" />
            Export
          </button>
        </div>
      </section>

      <section class="panel">
        <div class="head">
          <Icon name="settings" :size="15" />
          <span class="head-title">System</span>
        </div>
        <div class="foot">
          <button type="button" class="btn wide" :class="showDeviceInfo ? 'primary' : 'outline'" @click="toggleDeviceInfo">
            <Icon name="log" :size="14" />
            Device Info
          </button>
        </div>
      </section>
    </aside>

    <!-- what can be done with the open (or not yet created) image-map -->
    <div v-if="!showDeviceInfo && (map || isNew)" class="action-tile">
      <div class="tile-name">
        <Icon :name="editing ? 'edit' : 'image'" :size="14" />
        <input
          v-if="editing && map"
          ref="nameInput"
          :value="map.name"
          class="name"
          spellcheck="false"
          placeholder="Name"
          :maxlength="NAME_MAX"
          @input="map.name = stripNameChars(($event.target as HTMLInputElement).value)"
        />
        <span v-else class="name-text">{{ map ? map.name : selectedName }}</span>
      </div>
      <div class="tile-btns">
      <template v-if="map">
        <button
          type="button"
          class="btn primary small"
          :disabled="isChosen || !device"
          :title="isChosen ? 'Already in use' : !device ? 'Device not connected' : undefined"
          @click="useForDevice"
        >
          <Icon name="check" :size="14" />
          Use for Device
        </button>
        <div class="divider" />
      </template>
      <template v-if="editing">
        <button type="button" class="btn outline small" @click="cancelEdit">
          <Icon name="close" :size="14" />
          Cancel
        </button>
        <button type="button" class="btn primary small" @click="finishEdit">
          <Icon name="save" :size="14" />
          Save
        </button>
        <div class="divider" />
      </template>
      <button v-if="isNew || editing" type="button" class="btn outline small" @click="chooseImage">
        <Icon name="folder" :size="14" />
        Choose Image
      </button>
      <template v-if="map && !editing">
        <button
          type="button"
          class="btn outline small"
          :disabled="locked"
          :title="locked ? 'Read-only' : undefined"
          @click="startEdit"
        >
          <Icon name="edit" :size="14" />
          Edit
        </button>
        <button type="button" class="btn outline small" @click="openSummary && cloneMap(openSummary)">
          <Icon name="clone" :size="14" />
          Clone
        </button>
        <button
          type="button"
          class="btn danger small"
          :disabled="locked"
          :title="locked ? 'Read-only' : undefined"
          @click="openSummary && deleteMap(openSummary)"
        >
          <Icon name="trash" :size="14" />
          Delete
        </button>
      </template>
      </div>
    </div>

    <!-- centre: the canvas -->
    <section class="col-centre" :class="{ split: showDeviceInfo }">
      <template v-if="showDeviceInfo">
        <section class="panel log-tile">
          <div class="head">
            <Icon name="list" :size="15" />
            <span class="head-title">Device List</span>
            <span class="head-count">{{ props.devices.length }}</span>
            <div class="grow" />
            <button
              type="button"
              class="btn primary small"
              @click="saveText('device-list', listText, 'Device list saved')"
            >
              <Icon name="save" :size="13" />
              Save
            </button>
          </div>
          <div class="log mono">
            <div v-for="d in props.devices" :key="d.index" class="log-dev">
              <div class="log-line">
                <span class="log-key">#{{ d.index }}</span>
                <span class="log-name">{{ deviceName(d) }}</span>
              </div>
              <div v-for="[k, v] in deviceRows(d)" :key="k" class="log-kv">
                <span class="log-dim">{{ k }}</span>
                <span class="log-val">{{ v }}</span>
              </div>
            </div>
            <div v-if="!props.devices.length" class="log-line log-dim">None</div>
          </div>
        </section>

        <section class="panel log-tile">
          <div class="head">
            <Icon name="log" :size="15" />
            <span class="head-title">Device Events</span>
            <div class="grow" />
            <button type="button" class="btn outline small" :disabled="!props.events.length" @click="emit('clearLog')">
              <Icon name="trash" :size="13" />
              Clear
            </button>
            <button
              type="button"
              class="btn primary small"
              @click="saveText('device-events', eventsText, 'Device events saved')"
            >
              <Icon name="save" :size="13" />
              Save
            </button>
          </div>
          <div class="log mono">
            <div v-for="ev in props.events" :key="ev.id" class="log-line">
              <span class="log-time">{{ clock(ev.at) }} t{{ ev.timestamp }}</span>
              <span class="log-key">#{{ deviceOfEvent(ev)?.index ?? "?" }} i{{ ev.instance_id }}</span>
              <span class="log-device">{{ nameOfEvent(ev) }}</span>
              <span>{{ eventText(ev) }}</span>
              <span class="log-token">{{ tokenText(ev) }}</span>
            </div>
            <div v-if="!props.events.length" class="log-line log-dim">None</div>
          </div>
        </section>
      </template>
      <template v-else-if="map">
        <div v-if="editing" class="centre-head">
          <template v-for="t in TOOLS" :key="t.tool">
            <div v-if="t.divider" class="divider" />
            <button
              type="button"
              class="tool"
              :class="{ on: tool === t.tool }"
              :disabled="!currentKey"
              :title="currentKey ? t.title : `${t.title} · record an input first`"
              @click="setTool(t.tool)"
            >
              <Icon :name="t.icon" :size="16" />
            </button>
          </template>
          <template v-if="tool === 'text'">
            <input
              v-model="textInput"
              class="text-input"
              type="text"
              :maxlength="TEXT_MAX_LEN"
              placeholder="Text, then click the image"
              spellcheck="false"
            />
            <Dropdown
              v-model="textFamily"
              class="font-pick"
              variant="small"
              :options="fontOptions"
              :placeholder="fontFamilies === null ? 'Loading fonts…' : 'No fonts'"
              title="Font"
            />
            <button type="button" class="tool font" :class="{ on: textBold }" title="Bold" @click="textBold = !textBold">
              B
            </button>
          </template>
          <div class="grow" />
          <button type="button" class="tool" :disabled="!canUndo" title="Undo (Ctrl+Z)" @click="undo()">
            <Icon name="undo" :size="16" />
          </button>
          <button type="button" class="tool" :disabled="!canRedo" title="Redo (Ctrl+Shift+Z)" @click="redo()">
            <Icon name="redo" :size="16" />
          </button>
          <div class="divider" />
          <div class="zoom mono">
            <button type="button" title="Zoom out" @click="zoomStep(-1)">−</button>
            <button type="button" class="zoom-val" title="Reset zoom" @click="zoom = 1">
              {{ Math.round(zoom * 100) }}%
            </button>
            <button type="button" title="Zoom in" @click="zoomStep(1)">+</button>
          </div>
        </div>

        <div class="stage-wrap">
        <div ref="stageBox" class="stage-box" @mousedown.capture="onBoxMouseDown" @wheel="onWheel">
          <div class="stage-centre">
            <v-stage
              v-if="imgEl && W"
              ref="stageRef"
              :config="{ width: W, height: H }"
              @mousedown="onStageMouseDown"
              @mousemove="onStageMouseMove"
              @mouseup="onStageMouseUp"
              @dblclick="commitPolygon"
            >
              <v-layer>
                <v-image :config="{ image: imgEl, width: W, height: H, name: 'bg' }" />
              </v-layer>
              <v-layer>
                <template v-for="a in visibleShapes" :key="a.id">
                  <v-rect
                    v-if="a.geometry.kind === 'rect'"
                    :config="rectCfg(a)"
                    @click="onShapeClick(a)"
                    @mouseenter="onShapeEnter(a)"
                    @mouseleave="onShapeLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-ellipse
                    v-else-if="a.geometry.kind === 'ellipse'"
                    :config="ellipseCfg(a)"
                    @click="onShapeClick(a)"
                    @mouseenter="onShapeEnter(a)"
                    @mouseleave="onShapeLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-line
                    v-else-if="a.geometry.kind === 'polygon'"
                    :config="polyCfg(a)"
                    @click="onShapeClick(a)"
                    @mouseenter="onShapeEnter(a)"
                    @mouseleave="onShapeLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-arc
                    v-else-if="a.geometry.kind === 'arc'"
                    :config="arcCfg(a)"
                    @click="onShapeClick(a)"
                    @mouseenter="onShapeEnter(a)"
                    @mouseleave="onShapeLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-wedge
                    v-else-if="a.geometry.kind === 'wedge'"
                    :config="wedgeCfg(a)"
                    @click="onShapeClick(a)"
                    @mouseenter="onShapeEnter(a)"
                    @mouseleave="onShapeLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-image
                    v-else-if="a.geometry.kind === 'image'"
                    :config="imageCfg(a)"
                    @click="onShapeClick(a)"
                    @mouseenter="onShapeEnter(a)"
                    @mouseleave="onShapeLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-path
                    v-else
                    :config="symbolCfg(a)"
                    @click="onShapeClick(a)"
                    @mouseenter="onShapeEnter(a)"
                    @mouseleave="onShapeLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                </template>

                <v-rect
                  v-if="drawPreview && tool === 'rect'"
                  :config="{ ...drawPreview, stroke: paint.live, dash: [4, 4], listening: false }"
                />
                <v-ellipse
                  v-if="drawPreview && tool === 'ellipse'"
                  :config="{
                    x: drawPreview.x + drawPreview.width / 2,
                    y: drawPreview.y + drawPreview.height / 2,
                    radiusX: drawPreview.width / 2,
                    radiusY: drawPreview.height / 2,
                    stroke: paint.live,
                    dash: [4, 4],
                    listening: false,
                  }"
                />
                <v-circle
                  v-if="drawPreview && (tool === 'arc' || tool === 'wedge')"
                  :config="{
                    x: drawPreview.x + drawPreview.width / 2,
                    y: drawPreview.y + drawPreview.height / 2,
                    radius: Math.min(drawPreview.width, drawPreview.height) / 2,
                    stroke: paint.live,
                    dash: [4, 4],
                    listening: false,
                  }"
                />
                <v-line
                  v-if="polyPreview"
                  :config="{ points: polyPreview, stroke: paint.live, dash: [4, 4], closed: false, listening: false }"
                />

                <v-circle
                  v-for="v in vertexAnchors"
                  :key="`v${v.i}`"
                  :config="{
                    x: v.x,
                    y: v.y,
                    radius: 5,
                    fill: paint.anchor,
                    stroke: paint.live,
                    strokeWidth: 2,
                    draggable: true,
                  }"
                  @dragmove="onVertexDrag(v.i, $event)"
                />

                <v-transformer
                  ref="trRef"
                  :config="{
                    rotateEnabled: true,
                    keepRatio,
                    anchorSize: 8,
                    borderStroke: paint.live,
                    anchorStroke: paint.live,
                    anchorFill: paint.anchor,
                  }"
                />

                <v-label v-if="hover" :config="{ x: hover.x, y: hover.y, listening: false }">
                  <v-tag :config="{ fill: paint.tipBg, cornerRadius: 3 }" />
                  <v-text :config="{ text: hover.text, fill: paint.tipText, fontSize: 12, padding: 4 }" />
                </v-label>
              </v-layer>
            </v-stage>
          </div>
        </div>
        <!-- the selected shape's panel floats over the canvas, top left; it
             sits next to the scrolling box, so it neither moves the canvas
             nor scrolls with it -->
        <div v-if="editing && selectedShape" class="shape-panel">
            <div class="sp-head">
              <Icon :name="shapeIcon(selectedShape)" :size="14" />
              <span class="sp-kind">{{ shapeKind(selectedShape) }}</span>
              <span class="sp-input mono">{{ keyName(selectedShape.input) }}</span>
            </div>
            <template v-if="selectedShape.geometry.kind !== 'image'">
              <div v-for="role in (['stroke', 'fill'] as const)" :key="role" class="sp-row">
                <span class="sp-label">{{ role === "stroke" ? "Outline" : "Fill" }}</span>
                <div class="sp-colour">
                  <div class="swatches">
                    <button
                      v-for="(c, i) in swatches"
                      :key="c"
                      type="button"
                      class="swatch"
                      :class="{ on: colourHex(roleColour(selectedShape, role)) === c }"
                      :style="{ background: c }"
                      :title="i === 0 ? `Default · ${c}` : c"
                      @click="pickSwatch(selectedShape, role, i, c)"
                    />
                  </div>
                  <div class="sp-line">
                    <span class="swatch big" :style="{ background: roleColour(selectedShape, role) }" />
                    <input
                      class="hex mono"
                      :value="colourHex(roleColour(selectedShape, role))"
                      maxlength="7"
                      spellcheck="false"
                      @change="onHexInput(selectedShape, role, $event)"
                    />
                    <input
                      type="range"
                      class="range"
                      min="0"
                      max="100"
                      :value="colourAlpha(roleColour(selectedShape, role))"
                      title="Opacity"
                      @input="onAlphaInput(selectedShape, role, $event)"
                    />
                    <span class="mono sp-val">{{ colourAlpha(roleColour(selectedShape, role)) }}%</span>
                  </div>
                </div>
              </div>
            </template>
            <div v-if="selectedShape.geometry.kind === 'rect'" class="sp-row">
              <span class="sp-label">Corners</span>
              <div class="sp-line">
                <input
                  type="range"
                  class="range"
                  min="0"
                  max="50"
                  :value="Math.round(selectedShape.geometry.radius * 100)"
                  @input="onRangeInput($event, (v) => ((selectedShape!.geometry as RectGeometry).radius = v / 100))"
                />
                <span class="mono sp-val">{{ Math.round(selectedShape.geometry.radius * 100) }}%</span>
              </div>
            </div>
            <template v-if="selectedShape.geometry.kind === 'arc' || selectedShape.geometry.kind === 'wedge'">
              <div class="sp-row">
                <span class="sp-label">Angle</span>
                <div class="sp-line">
                  <input
                    type="range"
                    class="range"
                    min="5"
                    max="360"
                    :value="Math.round(selectedShape.geometry.angle)"
                    @input="onRangeInput($event, (v) => ((selectedShape!.geometry as ArcGeometry | WedgeGeometry).angle = v))"
                  />
                  <span class="mono sp-val">{{ Math.round(selectedShape.geometry.angle) }}°</span>
                </div>
              </div>
              <div v-if="selectedShape.geometry.kind === 'arc'" class="sp-row">
                <span class="sp-label">Inner</span>
                <div class="sp-line">
                  <input
                    type="range"
                    class="range"
                    min="0"
                    max="95"
                    :value="Math.round(selectedShape.geometry.inner * 100)"
                    @input="onRangeInput($event, (v) => ((selectedShape!.geometry as ArcGeometry).inner = v / 100))"
                  />
                  <span class="mono sp-val">{{ Math.round(selectedShape.geometry.inner * 100) }}%</span>
                </div>
              </div>
            </template>
        </div>
        </div>
      </template>
      <template v-else-if="isNew">
        <div class="none">Choose image…</div>
      </template>
      <div v-else-if="noMap" class="none">
        <button type="button" class="btn primary" @click="newMap">
          <Icon name="plus" :size="14" />
          New Image-Map
        </button>
      </div>
      <div v-else class="none">{{ listed.length ? "No image-map" : "No device" }}</div>
    </section>

    <Splitter direction="col" class="col-split-left" @drag="dragLeft" @end="endDragLeft" @reset="leftWidth = LEFT_W.def" />

    <Splitter
      v-if="!showDeviceInfo && !noMap"
      direction="col"
      class="col-split"
      @drag="dragRight"
      @end="endDragRight"
      @reset="rightWidth = RIGHT_W.def"
    />

    <!-- right: live input and shapes -->
    <aside v-if="!showDeviceInfo && !noMap" class="col-right">
      <div v-if="editing" class="input-card">
        <div class="ic-key" :class="{ on: !!currentKey }">
          <Icon name="bolt" :size="22" />
          <span v-if="currentKey" class="key">{{ currentText }}</span>
          <span v-else class="key idle">{{ recording ? "Waiting for input…" : "No input registered" }}</span>
        </div>
        <div class="ic-sub">{{ selectedName }}</div>
        <div class="ic-btns">
          <button
            type="button"
            class="btn small"
            :class="recording ? 'primary' : 'outline'"
            :disabled="!device"
            :title="device ? 'Take the next input of this device' : 'Device not connected'"
            @click="recording = true"
          >
            <Icon name="target" :size="13" />
            {{ recording ? "Recording…" : "Record Input" }}
          </button>
          <span v-if="recording" class="ic-hint">Esc to cancel</span>
        </div>
        <div class="ic-status" :class="inputStatus.kind">
          <Icon :name="inputStatus.icon" :size="13" />
          <span>{{ inputStatus.text }}</span>
        </div>
      </div>

      <div class="shapes" :style="{ '--cols': cols.template.value, '--cols-min': `${cols.minWidth.value}px` }">
        <div class="shapes-head">
          <div class="panel-title">Shapes</div>
          <div class="divider" />
          <button type="button" class="hbtn framed" title="Expand all" :disabled="allExpanded" @click="expandAll">
            <Icon name="unfold" :size="14" />
          </button>
          <button type="button" class="hbtn framed" title="Collapse all" :disabled="!expanded.size" @click="collapseAll">
            <Icon name="fold" :size="14" />
          </button>
          <div class="divider" />
          <button
            type="button"
            class="hbtn framed"
            :title="anyHidden ? 'Show all' : 'Hide all'"
            :disabled="!shapes.length"
            @click="toggleAllHidden"
          >
            <Icon :name="anyHidden ? 'eye-off' : 'eye'" :size="14" />
          </button>
          <div class="grow" />
          <div class="search">
            <Icon name="search" :size="14" />
            <input v-model="filter" class="mono" placeholder="Find…" spellcheck="false" />
            <button v-if="filter" type="button" class="clear" title="Clear" @click="filter = ''">
              <Icon name="close" :size="12" />
            </button>
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
          <template v-for="g in buckets" :key="g.key">
            <div class="row group-row" :class="{ cur: g.key === currentKey }" @click="toggleBucket(g.key)">
              <span class="input-cell">
                <Icon :name="isOpen(g) ? 'chevron-down' : 'chevron-right'" :size="14" class="chevron" />
                <span class="cell-input mono" :title="g.key">{{ g.text }}</span>
              </span>
              <span />
            </div>
            <template v-if="isOpen(g)">
              <div
                v-for="a in g.rows"
                :key="a.id"
                class="row shape-row"
                :class="{ cur: g.key === currentKey, sel: a.id === selectedId, off: hidden.has(a.id) }"
                @click="pickShape(a)"
              >
                <span />
                <span class="shape-cell">
                  <Icon :name="shapeIcon(a)" :size="14" class="shape-icon" />
                  <span class="cell-kind">{{ shapeKind(a) }}</span>
                  <button type="button" class="hbtn" :title="hidden.has(a.id) ? 'Show' : 'Hide'" @click.stop="toggleHidden(a.id)">
                    <Icon :name="hidden.has(a.id) ? 'eye-off' : 'eye'" :size="13" />
                  </button>
                  <template v-if="editing">
                    <button type="button" class="hbtn" title="Duplicate · Ctrl+D" @click.stop="duplicateShape(a)">
                      <Icon name="clone" :size="13" />
                    </button>
                    <button type="button" class="row-del" title="Delete · Del" @click.stop="deleteShape(a.id)">
                      <Icon name="close" :size="13" />
                    </button>
                  </template>
                </span>
              </div>
            </template>
          </template>
          <div v-if="!buckets.length" class="row empty">{{ shapes.length ? "No match" : "No shapes" }}</div>
        </div>
      </div>
    </aside>

    <ConfirmDialog v-if="confirm" :title="confirm.title" :icon="confirm.icon" :buttons="confirm.buttons" @choose="onConfirm" />
  </section>
</template>

<style scoped>
/* The action tile sits above the canvas and the right column; the device list
   spans both rows. Row gap comes from the tile's margin, so the row collapses
   cleanly when there is no tile. */
.devices {
  flex: 1;
  display: grid;
  /* Each splitter is its own 16px column: after the device list, and
     between the right column and the canvas. */
  grid-template-columns: var(--left-w, 300px) 16px var(--right-w, 380px) 16px minmax(0, 1fr);
  grid-template-rows: auto minmax(0, 1fr);
  grid-template-areas:
    "left gap tile tile tile"
    "left gap right split centre";
  padding: 12px 16px 16px;
  min-height: 0;
}

/* Device Info and the no-image-map placeholder take the canvas column and
   the right one. */
.devices.device-info,
.devices.no-map {
  grid-template-columns: var(--left-w, 300px) 16px minmax(0, 1fr);
  grid-template-areas:
    "left gap tile"
    "left gap centre";
}

.col-split {
  grid-area: split;
}

.col-split-left {
  grid-area: gap;
}

.action-tile {
  grid-area: tile;
  margin-bottom: 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 16px 16px;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
}

/* The image-map's name as the tile title; an input while editing. */
/* Same height as text or input, so the tile does not jump between modes. */
.tile-name {
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-chip-sm);
  max-width: 390px;
  min-width: 0;
  color: var(--text);
}

.tile-name .name {
  height: 100%;
  padding: 0 8px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
}

.tile-btns {
  display: flex;
  align-items: center;
  gap: 8px;
}

.action-tile .divider {
  width: 1px;
  height: 20px;
  margin: 0 4px;
  background: var(--border-dim);
}

.grow {
  flex: 1;
}

/* --- shared controls (mirrors SettingsDialog) --- */

.btn {
  height: var(--h-control);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 0 16px;
  border-radius: var(--radius-control);
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  cursor: pointer;
  white-space: nowrap;
}

.btn.small {
  height: 30px;
  padding: 0 12px;
  gap: 6px;
}

.btn.wide {
  flex: 1;
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

/* --- left column --- */

.col-left {
  grid-area: left;
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-height: 0;
}

/* Panels like the Bindings mode's: head with icon + title, body, foot. */
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

.head-count {
  font-size: 12px;
  color: var(--text-2);
}

.foot {
  display: flex;
  gap: 6px;
  padding: 6px 12px 12px;
}

/* Own checkbox look (mirrors SettingsDialog): WebKitGTK would paint GTK's. */
.check {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 8px 14px 14px;
  font-size: 13px;
  color: var(--text-2);
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

.dev-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px;
}

.dev {
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 9px 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  cursor: pointer;
}

.dev.on {
  background: var(--bg-surface-2);
  border-color: var(--accent);
}

.dev.dim {
  opacity: 0.55;
  cursor: default;
}

/* An image-map's device that is not connected. */
.dev.unused {
  border-style: dashed;
  border-color: var(--border);
}

.dev.unused.on {
  border-color: var(--accent);
}

.dev-name {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  font-size: 14px;
}

/* Device kind icon, like the Monitor's device tiles (no status colour here). */
.dev-kind {
  flex-shrink: 0;
  color: var(--text-2);
}

.dev-line {
  font-size: 13px;
  color: var(--text-2);
  display: flex;
  align-items: center;
  gap: 6px;
}

.chip.small {
  padding: 1px 6px;
  font-size: 11px;
}

/* The selected device's image-maps: indented under a guide line, set off from
   the device row above and the next device below. */
.maps {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 4px 0 10px 10px;
  padding-left: 12px;
  border-left: 1px solid var(--border);
}

.map,
.map-new {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 6px;
  font-family: inherit;
  cursor: pointer;
}

.map {
  color: var(--text-2);
  border: 1px solid var(--border);
}

.map.open {
  background: var(--bg-surface-2);
  border-color: var(--accent);
  color: var(--text);
}

.map.open .map-name {
  font-weight: 600;
}

/* The leading slot: the check for the map the Monitor shows, else empty. */
.map-mark {
  display: flex;
  align-items: center;
  width: 15px;
  flex-shrink: 0;
  color: var(--live);
}

/* A bolder check than the icon default. */
.map-mark svg {
  stroke-width: 3;
}

.map-name {
  flex: 1;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ro {
  display: flex;
  align-items: center;
  color: var(--text-3);
}

.map-new {
  border: 1px dashed var(--text-3);
  background: transparent;
  color: var(--text-2);
  font-size: 13px;
}

/* --- centre column --- */

.col-centre {
  grid-area: centre;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-width: 0;
  min-height: 0;
}

/* The centre column's head (tools / log bar): tighter than the panel heads. */
.centre-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-dim);
}

.name {
  flex: 1;
  width: 100%;
  border: none;
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-weight: 600;
  font-size: 14px;
  padding: 0;
  min-width: 0;
  outline: none;
}

.name-text {
  font-weight: 600;
  font-size: 14px;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tool {
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-control);
  border: 1px solid var(--text-3);
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}

.tool.on {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--accent-text);
}

.tool:disabled {
  opacity: 0.4;
  cursor: default;
}

.centre-head .btn.small {
  height: 34px;
}

/* The text tool's input, font pick and bold toggle. */
.text-input {
  width: 220px;
  height: 34px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.tool.font {
  width: auto;
  padding: 0 12px;
  font-size: 13px;
  font-weight: 700;
}

.font-pick {
  flex: none;
  width: 200px;
  max-width: 200px;
}

.centre-head .divider {
  width: 1px;
  height: 24px;
  margin: 0 4px;
  background: var(--border-dim);
}

.zoom {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 34px;
  padding: 0 10px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  font-size: 13px;
  color: var(--text-2);
}

.zoom button {
  border: none;
  background: transparent;
  color: inherit;
  font-family: inherit;
  font-size: 13px;
  padding: 0;
  cursor: pointer;
}

.zoom .zoom-val {
  color: var(--text);
  min-width: 42px;
  text-align: center;
}

.stage-box {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  padding: 16px;
  box-sizing: border-box;
  background-image:
    linear-gradient(color-mix(in srgb, var(--text-2) 6%, transparent) 1px, transparent 1px),
    linear-gradient(90deg, color-mix(in srgb, var(--text-2) 6%, transparent) 1px, transparent 1px);
  background-size: 24px 24px;
}

.stage-box.panning,
.stage-box.panning * {
  cursor: grabbing !important;
}

/* `margin: auto` centres without cutting off the edges when it overflows. */
.stage-centre {
  margin: auto;
  line-height: 0;
}

.none {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 14px;
}

/* The still-empty new image-map, in the place the canvas will take. */
.chip {
  padding: 5px 12px;
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--warn) 14%, transparent);
  color: var(--warn);
  font-size: 13px;
  font-weight: 600;
}

/* --- raw log --- */

/* Device Info: the centre column holds two stacked tiles instead of being one. */
.col-centre.split {
  background: transparent;
  border-radius: 0;
  gap: 16px;
}

.log-tile {
  flex: 1;
  min-height: 0;
}

/* Count next to the title, the buttons pushed right by the spacer. */
.log-tile .head-title {
  flex: none;
}

.log {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 16px;
  font-size: 12px;
}

.log-line {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  padding: 3px 0;
}

.log-key {
  color: var(--live);
  min-width: 5rem;
}

.log-dev {
  margin-bottom: 10px;
}

.log-name {
  font-weight: 600;
}

.log-kv {
  display: grid;
  grid-template-columns: 8rem minmax(0, 1fr);
  gap: 10px;
  padding: 1px 0 1px 1rem;
}

.log-val {
  overflow-wrap: anywhere;
}

.log-time {
  color: var(--text-2);
}

/* The SC side of an event (token · label), set apart from the SDL facts. */
.log-token {
  color: var(--accent);
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

/* --- right column --- */

.col-right {
  grid-area: right;
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
}

.input-card {
  background: var(--bg-surface);
  border: 1px solid color-mix(in srgb, var(--live) 35%, transparent);
  border-radius: var(--radius-panel);
  padding: 16px 20px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* Same look as the Monitor's Last Input card: bolt and label in live colour
   while there is an input, dimmed otherwise. */
.ic-key {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--text-3);
}

.ic-key.on {
  color: var(--live);
}

.ic-key.on :deep(svg) {
  fill: color-mix(in srgb, var(--live) 25%, transparent);
}

.key {
  font-size: 22px;
  font-weight: 600;
  line-height: 1.1;
}

/* Same size as a recorded input, so the card keeps its height. */
.key.idle {
  color: var(--text-3);
}

.ic-sub {
  font-size: 12px;
  color: var(--text-2);
}

.ic-btns {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 4px;
}

.ic-hint {
  font-size: 12px;
  color: var(--text-3);
}

/* Colour-coded state under the Record button. */
.ic-status {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.ic-status.warn {
  color: var(--warn);
}

.ic-status.ok {
  color: var(--ok);
}

.shapes {
  flex: 1;
  min-height: 0;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* Same head as the bindings deck: title, divider, buttons, search. */
.shapes-head {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border-dim);
}

.shapes-head .divider {
  width: 1px;
  height: 20px;
  margin: 0 8px;
  background: var(--border-dim);
}

.search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-chip);
  width: 180px;
  min-width: 0;
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
  font-size: 13px;
  outline: none;
}

.search input::placeholder {
  color: rgba(173, 211, 235, 0.6);
}

/* --- shape panel: floats over the canvas in its top-left corner, sticky so
   it stays there while the canvas scrolls --- */

.stage-wrap {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
}

.shape-panel {
  position: absolute;
  top: 8px;
  left: 8px;
  z-index: 2;
  width: 390px;
  max-width: calc(100% - 16px);
  background: color-mix(in srgb, var(--bg-surface) 92%, transparent);
  border: 1px solid var(--border-dim);
  border-radius: var(--radius-panel);
  padding: 12px 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  font-size: 13px;
}

.sp-head {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--text);
}

.sp-kind {
  font-weight: 600;
  text-transform: capitalize;
}

.sp-input {
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sp-row {
  display: grid;
  grid-template-columns: 60px minmax(0, 1fr);
  gap: 10px;
  align-items: start;
}

.sp-label {
  padding-top: 4px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-2);
}

.sp-colour {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.swatches {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.swatch {
  width: 18px;
  height: 18px;
  border-radius: 3px;
  border: 1px solid var(--border);
  padding: 0;
  cursor: pointer;
}

.swatch.on {
  outline: 2px solid var(--text);
  outline-offset: 1px;
}

.swatch.big {
  width: 24px;
  height: 24px;
  flex-shrink: 0;
  cursor: default;
}

.sp-line {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.hex {
  width: 74px;
  height: 24px;
  padding: 0 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 12px;
  outline: none;
}

.sp-val {
  width: 40px;
  flex-shrink: 0;
  text-align: right;
  color: var(--text-2);
  font-size: 12px;
}

/* Own-styled slider (WebKitGTK would paint GTK's). */
.range {
  flex: 0 0 90px;
  height: 4px;
  appearance: none;
  -webkit-appearance: none;
  background: var(--bg-surface-3);
  border-radius: 2px;
  outline: none;
  cursor: pointer;
}

.range::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: var(--accent);
  border: none;
}

/* --- the table (mirrors the bindings deck) --- */

.table {
  overflow: auto;
  min-height: 0;
  flex: 1;
}

.row {
  display: grid;
  grid-template-columns: var(--cols);
  gap: 12px;
  padding: 8px 14px;
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
  color: var(--text-2);
  border-bottom: 1px solid var(--border-dim);
}

.row.empty {
  color: var(--text-3);
  font-size: 13px;
}

/* A bucket is set off from the one before by a line above its head; its
   shapes hang below it, tied to the head by a thin tree line under the
   chevron. */
.group-row {
  font-size: 13px;
  cursor: pointer;
  user-select: none;
  border-left: 3px solid transparent;
  border-top: 1px solid var(--border-dim);
  color: var(--text-2);
}

.shape-row {
  position: relative;
  font-size: 13px;
  cursor: pointer;
  border-left: 3px solid transparent;
  color: var(--text-2);
}

.shape-row::before {
  content: "";
  position: absolute;
  left: 22px;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--border-dim);
}

.group-row:hover,
.shape-row:hover {
  background: var(--bg-surface-2);
}

.input-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.input-cell .chevron {
  flex-shrink: 0;
  color: var(--text-3);
}

.input-cell .cell-input {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

/* The recorded input: live-coloured edge, text and bolt on its bucket. */
.group-row.cur {
  border-left-color: var(--live);
  background: color-mix(in srgb, var(--live) 8%, transparent);
}

.group-row.cur .cell-input {
  color: var(--live);
}

.shape-row.cur .cell-kind {
  color: var(--live);
}

.shape-row.sel {
  background: var(--bg-surface-2);
}

.shape-cell {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.shape-icon {
  flex-shrink: 0;
}

.cell-kind {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Small neutral icon button (head and rows). */
.hbtn {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}

.hbtn:hover:not(:disabled) {
  background: var(--bg-surface-2);
  color: var(--text);
}

/* Head buttons look like the bindings deck's: chip-sized, white outline. */
.hbtn.framed {
  width: var(--h-chip-sm);
  height: var(--h-chip-sm);
  border: 1px solid rgba(255, 255, 255, 0.5);
  color: var(--text);
}

.hbtn.framed:hover:not(:disabled) {
  border-color: var(--accent);
}

.hbtn:disabled {
  opacity: 0.35;
  cursor: default;
}

.shape-row.off .cell-kind,
.shape-row.off .shape-icon {
  opacity: 0.45;
}

.row-del {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}

.row-del:hover {
  background: var(--bg-surface-2);
  color: var(--err);
}


</style>
