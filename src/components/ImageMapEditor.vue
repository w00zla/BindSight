<script setup lang="ts">
// Devices mode: pick a device, pick one of its image-maps, press a physical
// input and draw the areas that belong to it. Konva does the canvas work;
// everything is stored normalized (0..1) in the image-map.
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { KonvaEventObject, Node as KonvaNode } from "konva/lib/Node";
import type { Stage } from "konva/lib/Stage";
import type { Ellipse } from "konva/lib/shapes/Ellipse";
import type { Transformer } from "konva/lib/shapes/Transformer";
import type { VueKonvaRef } from "vue-konva";
import Icon, { type IconName } from "./Icon.vue";
import ConfirmDialog, { type ConfirmButton } from "./ConfirmDialog.vue";
import type { DeviceInfo, JoyInput, LoggedInput } from "../types";
import { deviceName } from "../devices";
import {
  SYMBOL_PATHS,
  symbolPx,
  inputKey,
  sameHardware,
  type Area,
  type ImageMap,
  type ImageMapSummary,
  type Shape,
  type SymbolKind,
} from "../imagemap";

// `keyInput`: the last key captured in the webview — the backend never sees
// keys, so App hands them over instead of an event.
const props = defineProps<{ devices: DeviceInfo[]; events: LoggedInput[]; keyInput: JoyInput | null }>();
const emit = defineEmits<{ notify: [message: string, type: "ok" | "error"]; saved: []; clearLog: [] }>();

// No active tool == select/move mode.
type Tool = "rect" | "ellipse" | "polygon" | SymbolKind;
const TOOLS: { tool: Tool; icon: IconName; title: string }[] = [
  { tool: "rect", icon: "shape-rect", title: "Rectangle" },
  { tool: "ellipse", icon: "shape-ellipse", title: "Ellipse" },
  { tool: "polygon", icon: "shape-polygon", title: "Polygon" },
  { tool: "arrow", icon: "shape-arrow", title: "Arrow" },
  { tool: "cw", icon: "shape-cw", title: "Clockwise" },
  { tool: "ccw", icon: "shape-ccw", title: "Counter-clockwise" },
];

// A new symbol is 5% of the image width, square on screen.
const DEFAULT_SYMBOL_W = 0.05;
const MIN_DRAW_PX = 4;
const ZOOMS = [0.5, 0.75, 1, 1.5, 2, 3];

// --- colours ---------------------------------------------------------------

// Konva needs literal colours; the tokens are the only place they are defined.
function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function withAlpha(colour: string, alpha: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(colour);
  if (!m) return colour;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

// Filled once at mount — the stage only exists after an image has loaded.
const paint = ref({
  live: "",
  liveFill: "",
  dim: "",
  dimFill: "",
  anchor: "",
  tipBg: "",
  tipText: "",
});

function readPaint() {
  const live = cssVar("--live");
  const dim = cssVar("--text-2");
  paint.value = {
    live,
    liveFill: withAlpha(live, 0.25),
    dim,
    dimFill: withAlpha(dim, 0.1),
    anchor: cssVar("--text"),
    tipBg: withAlpha(cssVar("--bg-base"), 0.85),
    tipText: cssVar("--text"),
  };
}

// --- device / image-map selection -----------------------------------------

const selectedGuid = ref("");
const device = computed(() => props.devices.find((d) => d.sdl_guid === selectedGuid.value) ?? null);


const selectedName = computed(() => (device.value ? deviceName(device.value) : "—"));

const summaries = ref<ImageMapSummary[]>([]);

function mapsFor(d: DeviceInfo): ImageMapSummary[] {
  return summaries.value.filter((s) => sameHardware(s.hardware_id, d.hardware_id));
}

const deviceMaps = computed(() => (device.value ? mapsFor(device.value) : []));

const openId = ref("");
const map = ref<ImageMap | null>(null);
// Serialized state as last loaded/saved, for the dirty check.
const savedJson = ref("");
const dirty = computed(() => !!map.value && JSON.stringify(map.value) !== savedJson.value);
const openSummary = computed(() => summaries.value.find((s) => s.id === openId.value) ?? null);
// Bundled image-maps are read-only, and so is everything without a map.
const locked = computed(() => !map.value || openSummary.value?.source === "bundled");

// --- canvas state ----------------------------------------------------------

const areas = computed<Area[]>(() => map.value?.areas ?? []);

const imgEl = ref<HTMLImageElement | null>(null);
const natW = ref(0);
const natH = ref(0);
const aspect = computed(() => (natW.value && natH.value ? natW.value / natH.value : 1));

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
const canDrag = computed(() => selectMode.value && !locked.value);
const selectedId = ref<string | null>(null);
const selectedArea = computed(() => areas.value.find((a) => a.id === selectedId.value) ?? null);
const hover = ref<{ x: number; y: number; text: string } | null>(null);

const currentKey = ref<string | null>(null);
const currentCount = computed(() =>
  currentKey.value ? areas.value.filter((a) => a.input === currentKey.value).length : 0,
);
const currentCountText = computed(() => `${currentCount.value} area${currentCount.value === 1 ? "" : "s"}`);

const filter = ref("");
// Natural order, so button:2 comes before button:10.
const listedAreas = computed(() => {
  const f = filter.value.trim().toLowerCase();
  return areas.value
    .filter((a) => !f || a.input.toLowerCase().includes(f))
    .slice()
    .sort((x, y) => x.input.localeCompare(y.input, undefined, { numeric: true }));
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

// True when it is fine to drop what is on screen: not dirty, or the user chose
// Discard, or the save went through.
async function requestLeave(): Promise<boolean> {
  if (!dirty.value) return true;
  const choice = await ask("Unsaved changes", [
    { label: "Discard", kind: "danger", value: "discard" },
    { label: "Save", kind: "primary", value: "save" },
    { label: "Keep editing", kind: "outline", value: "keep" },
  ]);
  if (choice === "keep") return false;
  if (choice === "save") return await saveMap();
  return true;
}

// `selectedGuid` and `confirmOpen` tell App whether to capture keys.
const confirmOpen = computed(() => confirm.value !== null);
defineExpose({ requestLeave, selectedGuid, confirmOpen });

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
}

function closeMap() {
  map.value = null;
  openId.value = "";
  savedJson.value = "";
  selectedId.value = null;
  tool.value = null;
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

// The canvas only exists once an image-map is open; follow the element.
let ro: ResizeObserver | null = null;
watch(stageBox, (el) => {
  ro?.disconnect();
  ro = null;
  if (!el) return;
  boxW.value = el.clientWidth;
  boxH.value = el.clientHeight;
  ro = new ResizeObserver(() => {
    boxW.value = el.clientWidth;
    boxH.value = el.clientHeight;
  });
  ro.observe(el);
});

watch([() => map.value?.id, () => map.value?.image.file], () => {
  cancelDraw();
  loadCanvasImage();
});

// Keep the device selection on something that exists.
watch(
  () => props.devices,
  (list) => {
    if (list.some((d) => d.sdl_guid === selectedGuid.value)) return;
    selectedGuid.value = list.find((d) => d.hardware_id)?.sdl_guid ?? "";
    currentKey.value = null;
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
  showLog.value = false;
  if (!d.hardware_id || d.sdl_guid === selectedGuid.value) return;
  if (!(await requestLeave())) return;
  selectedGuid.value = d.sdl_guid;
  currentKey.value = null;
  closeMap();
  await openFirst();
}

async function openMap(id: string) {
  showLog.value = false;
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

async function newMap() {
  const d = device.value;
  if (!d?.hardware_id) return;
  if (!(await requestLeave())) return;
  try {
    const imagePath = await pickImage();
    if (!imagePath) return;
    const m = await invoke<ImageMap>("create_imagemap", {
      name: deviceName(d),
      hardwareId: d.hardware_id,
      hardwareName: d.sc_name ?? "",
      imagePath,
    });
    await loadSummaries();
    await loadMap(m.id);
    focusName();
    emit("notify", "Image-map created", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function cloneMap(s: ImageMapSummary) {
  if (!(await requestLeave())) return;
  try {
    const m = await invoke<ImageMap>("clone_imagemap", { id: s.id, name: `${s.name} copy` });
    await loadSummaries();
    await loadMap(m.id);
    focusName();
    emit("saved");
    emit("notify", "Image-map cloned", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function deleteMap(s: ImageMapSummary) {
  const choice = await ask("Delete image-map?", [
    { label: "Delete", kind: "danger", value: "delete" },
    { label: "Cancel", kind: "outline", value: "cancel" },
  ]);
  if (choice !== "delete") return;
  try {
    await invoke("delete_imagemap", { id: s.id });
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

async function discard() {
  if (!openId.value) return;
  await loadMap(openId.value);
}

async function exportMap() {
  const m = map.value;
  if (!m) return;
  try {
    const dest = await save({
      defaultPath: `${m.name || "image-map"}.zip`,
      filters: [{ name: "Image-map", extensions: ["zip"] }],
    });
    if (!dest) return;
    await invoke("export_imagemap", { id: m.id, destPath: dest });
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
    const d = props.devices.find((dev) => sameHardware(dev.hardware_id, s.hardware_id));
    if (d) {
      selectedGuid.value = d.sdl_guid;
      await loadMap(s.id);
    }
    emit("saved");
    emit("notify", `Imported ${s.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// Replace the image-map's image. The areas stay (a re-shot of the same view
// keeps them useful); the old file is removed once the new one is in.
async function replaceImage() {
  const m = map.value;
  if (!m || locked.value) return;
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

function takeInput(ev: JoyInput) {
  if (ev.guid !== selectedGuid.value) return;
  const key = inputKey(ev);
  if (key) currentKey.value = key;
}

// Keys arrive as a prop (App captures them), joystick and pad events directly.
watch(
  () => props.keyInput,
  (ev) => {
    if (ev) takeInput(ev);
  },
);

onMounted(async () => {
  readPaint();
  unlisten.push(await listen<JoyInput>("joy-input", (e) => takeInput(e.payload)));
  await loadSummaries();
  if (!openId.value) await openFirst();
});

onUnmounted(() => {
  unlisten.forEach((fn) => fn());
  unlisten = [];
  ro?.disconnect();
  ro = null;
});

// --- areas -----------------------------------------------------------------

function addArea(shape: Shape) {
  const m = map.value;
  if (!m || !currentKey.value) return;
  const area: Area = { id: newId(), input: currentKey.value, shape };
  m.areas.push(area);
  selectedId.value = area.id;
  tool.value = null;
}

// "Add area": a default shape of the active tool's kind at the image centre.
function addDefaultArea() {
  if (!map.value || !currentKey.value || locked.value) return;
  const kind = tool.value ?? "rect";
  if (kind === "rect") {
    addArea({ kind: "rect", x: 0.45, y: 0.47, w: 0.1, h: 0.06, rotation: 0 });
  } else if (kind === "ellipse") {
    addArea({ kind: "ellipse", cx: 0.5, cy: 0.5, rx: 0.05, ry: 0.03, rotation: 0 });
  } else if (kind === "polygon") {
    const hx = 0.04;
    const hy = hx * aspect.value;
    addArea({
      kind: "polygon",
      points: [
        [0.5, 0.5 - hy],
        [0.5 + hx, 0.5],
        [0.5, 0.5 + hy],
        [0.5 - hx, 0.5],
      ],
    });
  } else {
    addArea({
      kind: "symbol",
      symbol: kind,
      x: 0.5,
      y: 0.5,
      w: DEFAULT_SYMBOL_W,
      h: DEFAULT_SYMBOL_W * aspect.value,
      rotation: 0,
    });
  }
}

function setTool(t: Tool) {
  if (locked.value) return;
  cancelDraw();
  // Clicking the active tool turns it off — no tool == select/move.
  tool.value = tool.value === t ? null : t;
  if (tool.value) selectedId.value = null;
}

function cancelDraw() {
  draft.value = null;
  draftPoly.value = [];
}

function deleteSelected() {
  const m = map.value;
  if (!m || !selectedId.value || locked.value) return;
  m.areas = m.areas.filter((a) => a.id !== selectedId.value);
  selectedId.value = null;
}

// A row in the list, or a shape on the canvas: both select the area and make
// its input the current one.
function pickArea(a: Area) {
  selectedId.value = a.id;
  currentKey.value = a.input;
}

function areaIcon(a: Area): IconName {
  return (a.shape.kind === "symbol" ? `shape-${a.shape.symbol}` : `shape-${a.shape.kind}`) as IconName;
}

function areaKind(a: Area): string {
  return a.shape.kind === "symbol" ? a.shape.symbol : a.shape.kind;
}

// --- zoom ------------------------------------------------------------------

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

function onStageMouseDown(e: KonvaEventObject<MouseEvent>) {
  if (!W.value) return;
  const pos = stagePointer();
  if (!pos) return;
  if (selectMode.value) {
    // A click on the bare image clears the selection.
    if (e.target === e.target.getStage() || e.target.name() === "bg") selectedId.value = null;
    return;
  }
  if (locked.value) return;
  if (!currentKey.value) {
    emit("notify", "Press an input first", "error");
    return;
  }
  if (tool.value === "rect" || tool.value === "ellipse") {
    draft.value = { x0: pos.x, y0: pos.y, x1: pos.x, y1: pos.y };
  } else if (tool.value === "polygon") {
    draftPoly.value = [...draftPoly.value, pos.x, pos.y];
  } else if (tool.value) {
    addArea({
      kind: "symbol",
      symbol: tool.value,
      x: pos.x / W.value,
      y: pos.y / H.value,
      w: DEFAULT_SYMBOL_W,
      h: (DEFAULT_SYMBOL_W * W.value) / H.value,
      rotation: 0,
    });
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
  if (tool.value === "rect") {
    addArea({ kind: "rect", x: x / W.value, y: y / H.value, w: w / W.value, h: h / H.value, rotation: 0 });
  } else if (tool.value === "ellipse") {
    addArea({
      kind: "ellipse",
      cx: (x + w / 2) / W.value,
      cy: (y + h / 2) / H.value,
      rx: w / 2 / W.value,
      ry: h / 2 / H.value,
      rotation: 0,
    });
  }
}

function commitPolygon() {
  const pts = draftPoly.value;
  if (pts.length < 6 || !W.value) return;
  const points: [number, number][] = [];
  for (let i = 0; i < pts.length; i += 2) {
    // A double-click lands two mousedowns on the same spot — drop the repeat.
    const last = points[points.length - 1];
    if (last && Math.abs(last[0] * W.value - pts[i]) < 2 && Math.abs(last[1] * H.value - pts[i + 1]) < 2) continue;
    points.push([pts[i] / W.value, pts[i + 1] / H.value]);
  }
  draftPoly.value = [];
  if (points.length < 3) return;
  addArea({ kind: "polygon", points });
}

// --- shape configs ---------------------------------------------------------

function colours(a: Area) {
  const isCurrent = a.input === currentKey.value;
  const selected = a.id === selectedId.value;
  return {
    stroke: isCurrent ? paint.value.live : paint.value.dim,
    strokeWidth: selected ? 3 : isCurrent ? 2 : 1,
    fill: isCurrent ? paint.value.liveFill : paint.value.dimFill,
    opacity: isCurrent ? 1 : 0.6,
    dash: selected ? [6, 3] : undefined,
  };
}

function rectCfg(a: Area) {
  if (a.shape.kind !== "rect") return {};
  const s = a.shape;
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
    rotation: s.rotation,
    draggable: canDrag.value,
    ...colours(a),
  };
}

function ellipseCfg(a: Area) {
  if (a.shape.kind !== "ellipse") return {};
  const s = a.shape;
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

function polyCfg(a: Area) {
  if (a.shape.kind !== "polygon") return {};
  return {
    id: a.id,
    x: 0,
    y: 0,
    points: a.shape.points.flatMap(([x, y]) => [x * W.value, y * H.value]),
    closed: true,
    draggable: canDrag.value,
    ...colours(a),
  };
}

function symbolCfg(a: Area) {
  if (a.shape.kind !== "symbol") return {};
  const s = a.shape;
  // 100 path units == w * image width by h * image height.
  const px = symbolPx(s, W.value, H.value);
  return {
    id: a.id,
    data: SYMBOL_PATHS[s.symbol],
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

// --- shape edits -----------------------------------------------------------

function onAreaClick(a: Area) {
  if (!selectMode.value) return;
  pickArea(a);
}

function onAreaEnter(a: Area) {
  const pos = stagePointer();
  hover.value = pos ? { x: pos.x + 10, y: pos.y + 10, text: a.input } : null;
}

function onAreaLeave() {
  hover.value = null;
}

function onDragEnd(a: Area, e: KonvaEventObject<DragEvent>) {
  const node = e.target;
  const s = a.shape;
  if (s.kind === "rect") {
    s.x = (node.x() - (s.w * W.value) / 2) / W.value;
    s.y = (node.y() - (s.h * H.value) / 2) / H.value;
  } else if (s.kind === "ellipse") {
    s.cx = node.x() / W.value;
    s.cy = node.y() / H.value;
  } else if (s.kind === "symbol") {
    s.x = node.x() / W.value;
    s.y = node.y() / H.value;
  } else {
    const dx = node.x() / W.value;
    const dy = node.y() / H.value;
    s.points = s.points.map(([x, y]) => [x + dx, y + dy] as [number, number]);
    node.position({ x: 0, y: 0 });
  }
}

function onTransformEnd(a: Area, e: KonvaEventObject<Event>) {
  const node = e.target;
  const s = a.shape;
  const sx = node.scaleX();
  const sy = node.scaleY();
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
  } else if (s.kind === "symbol") {
    s.w = (sx * 100) / W.value;
    s.h = (sy * 100) / H.value;
    s.x = node.x() / W.value;
    s.y = node.y() / H.value;
    s.rotation = node.rotation();
  }
}

// Polygon vertex anchors (select mode, polygon selected).
const vertexAnchors = computed(() => {
  const a = selectedArea.value;
  if (!a || a.shape.kind !== "polygon" || !canDrag.value) return [];
  return a.shape.points.map(([x, y], i) => ({ i, x: x * W.value, y: y * H.value }));
});

function onVertexDrag(i: number, e: KonvaEventObject<DragEvent>) {
  const a = selectedArea.value;
  if (!a || a.shape.kind !== "polygon") return;
  a.shape.points[i] = [e.target.x() / W.value, e.target.y() / H.value];
}

// Attach/detach the transformer whenever the selection or the shapes change.
watch(
  [selectedId, canDrag, areas, W],
  async () => {
    await nextTick();
    const tr = trRef.value?.getNode();
    const stage = stageRef.value?.getStage();
    if (!tr || !stage) return;
    const a = selectedArea.value;
    if (!a || a.shape.kind === "polygon" || !canDrag.value) {
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

// "28 btn · 8 axes · 2 hats", plus the image count for a device that is not
// the selected one (its image-maps are listed below the card).
// --- raw log (device dump + events, replaces the canvas while shown) --------

const showLog = ref(false);

// Last 8 hex chars of an SDL GUID: enough to tell devices apart in the log.
function shortGuid(guid: string): string {
  return guid.slice(-8);
}

function nameOfGuid(guid: string): string {
  const d = props.devices.find((dev) => dev.sdl_guid === guid);
  return d ? deviceName(d) : guid;
}

function deviceOfGuid(guid: string): DeviceInfo | undefined {
  return props.devices.find((dev) => dev.sdl_guid === guid);
}

// Event as one line: the SC axis name comes from the device's derived axes.
function eventText(ev: JoyInput): string {
  switch (ev.kind) {
    case "button":
      return `button ${ev.index} ${ev.pressed ? "down" : "up"}`;
    case "axis": {
      const sc = deviceOfGuid(ev.guid)?.axes[ev.index];
      return `axis ${ev.index}${sc ? ` (${sc})` : ""} = ${ev.value} (${(ev.value / 32767).toFixed(3)})`;
    }
    case "padbutton":
      return `pad ${ev.name} ${ev.pressed ? "down" : "up"}`;
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
  return d.axes.length ? d.axes.join(" ") : (d.axes_error ?? "—");
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

// Key/value rows of everything known about a device. The keyboard is a
// synthetic device — it has nothing but its name and its hardware id.
function deviceRows(d: DeviceInfo): [string, string][] {
  if (d.kind === "keyboard") {
    return [
      ["kind", d.kind],
      ["sdl name", d.sdl_name],
      ["hardware id", d.hardware_id ?? "—"],
    ];
  }
  return [
    ["kind", d.kind === "gamepad" ? `gamepad · slot ${d.gamepad_slot ?? "—"} · ${d.controller_name ?? "—"}` : d.kind],
    ["sdl name", d.sdl_name],
    ["sdl guid", d.sdl_guid],
    ["sc product", d.sc_product_guid ?? "—"],
    ["hardware id", d.hardware_id ?? "—"],
    ["sdl", `index ${d.index} · instance ${d.sdl_instance_id} · type ${d.sdl_type} · path ${d.sdl_path ?? "—"}`],
    ["usb", `vid ${hex4(d.sdl_vendor)} · pid ${hex4(d.sdl_product)} · version ${hex4(d.sdl_product_version)} · power ${d.power_level}`],
    [
      "io",
      `${d.num_buttons} buttons · ${d.num_axes} axes · ${d.num_hats} hats · ${d.num_balls} balls · rumble ${d.has_rumble ? "yes" : "no"} · led ${d.has_led ? "yes" : "no"}`,
    ],
    ["sc axes", axesText(d)],
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

function eventLine(ev: LoggedInput): string {
  const d = deviceOfGuid(ev.guid);
  return `${clock(ev.at)}  sdl+${ev.timestamp}ms  #${d?.index ?? "?"} ${shortGuid(ev.guid)} ${nameOfGuid(ev.guid)}  ${eventText(ev)}`;
}

function logText(): string {
  const lines = [`BindSight device log ${new Date().toISOString()}`, "", "Devices"];
  for (const d of props.devices) {
    lines.push(`#${d.index} ${deviceName(d)}`);
    for (const [k, v] of deviceRows(d)) lines.push(`    ${k.padEnd(15)} ${v}`);
  }
  if (!props.devices.length) lines.push("    none");
  lines.push("", "Events (newest first)");
  for (const ev of props.events) lines.push(eventLine(ev));
  if (!props.events.length) lines.push("    none");
  return lines.join("\n") + "\n";
}

async function saveLog() {
  try {
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const dest = await save({
      defaultPath: `bindsight-devices-${stamp}.txt`,
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (!dest) return;
    await invoke("write_text_file", { path: dest, text: logText() });
    emit("notify", "Log saved", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

function deviceLine(d: DeviceInfo): string {
  if (d.kind === "gamepad" && d.gamepad_slot === null) return "no slot";
  const parts: string[] = [];
  if (d.kind !== "keyboard") {
    if (d.num_buttons) parts.push(`${d.num_buttons} btn`);
    if (d.num_axes) parts.push(`${d.num_axes} ${d.num_axes === 1 ? "axis" : "axes"}`);
    if (d.num_hats) parts.push(`${d.num_hats} ${d.num_hats === 1 ? "hat" : "hats"}`);
  }
  if (!d.hardware_id) {
    parts.push("no SC id");
  } else if (d.sdl_guid !== selectedGuid.value) {
    const n = mapsFor(d).length;
    parts.push(n === 0 ? "no image" : `${n} ${n === 1 ? "image" : "images"}`);
  }
  return parts.join(" · ");
}
</script>

<template>
  <section class="devices" :class="{ log: showLog }">
    <!-- left: devices and their image-maps, and the system panel -->
    <aside class="col-left">
      <section class="panel grow">
        <div class="head">
          <Icon name="image" :size="15" />
          <span class="head-title">Image-maps</span>
          <span class="head-count">{{ summaries.length }}</span>
        </div>
        <div class="dev-list">
          <template v-for="d in props.devices" :key="d.index">
            <div
              class="dev"
              :class="{ on: d.sdl_guid === selectedGuid, dim: !d.hardware_id }"
              @click="selectDevice(d)"
            >
              <div class="dev-name">{{ deviceName(d) }}</div>
              <div class="dev-line">{{ deviceLine(d) }}</div>
            </div>

            <div v-if="d.sdl_guid === selectedGuid && d.hardware_id" class="maps">
              <div
                v-for="s in deviceMaps"
                :key="s.id"
                class="map"
                :class="{ open: s.id === openId }"
                @click="openMap(s.id)"
              >
                <Icon name="image" :size="13" />
                <span class="map-name">{{ s.name }}</span>
                <button type="button" class="icon-btn" title="Clone" @click.stop="cloneMap(s)">
                  <Icon name="clone" :size="13" />
                </button>
                <span v-if="s.source === 'bundled'" class="ro" title="Read-only">
                  <Icon name="lock" :size="13" />
                </span>
                <button v-else type="button" class="icon-btn" title="Delete image-map" @click.stop="deleteMap(s)">
                  <Icon name="trash" :size="13" />
                </button>
              </div>
              <button type="button" class="map-new" @click="newMap">
                <Icon name="plus" :size="13" />
                <span>New image-map</span>
              </button>
            </div>
          </template>
        </div>
        <div class="foot">
          <button type="button" class="btn outline wide" @click="importMap">
            <Icon name="download" :size="14" />
            Import
          </button>
          <button type="button" class="btn outline wide" :disabled="!map" @click="exportMap">
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
          <button type="button" class="btn wide" :class="showLog ? 'primary' : 'outline'" @click="showLog = !showLog">
            <Icon name="log" :size="14" />
            Device log
          </button>
        </div>
      </section>
    </aside>

    <!-- centre: the canvas -->
    <section class="col-centre">
      <template v-if="showLog">
        <div class="head">
          <span class="log-title">Log</span>
          <div class="grow" />
          <button type="button" class="btn outline small" :disabled="!props.events.length" @click="emit('clearLog')">
            <Icon name="trash" :size="13" />
            Clear
          </button>
          <button type="button" class="btn primary small" @click="saveLog">
            <Icon name="upload" :size="13" />
            Save
          </button>
        </div>
        <div class="log mono">
          <div class="log-section">Devices</div>
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
          <div class="log-section log-events">Events</div>
          <div v-for="(ev, i) in props.events" :key="i" class="log-line">
            <span class="log-time">{{ clock(ev.at) }}</span>
            <span class="log-dim">sdl+{{ ev.timestamp }}ms</span>
            <span class="log-key">#{{ deviceOfGuid(ev.guid)?.index ?? "?" }} {{ shortGuid(ev.guid) }}</span>
            <span class="log-device">{{ nameOfGuid(ev.guid) }}</span>
            <span>{{ eventText(ev) }}</span>
          </div>
          <div v-if="!props.events.length" class="log-line log-dim">None</div>
        </div>
      </template>
      <template v-else-if="map">
        <div class="head">
          <div class="head-name">
            <Icon name="image" :size="14" />
            <input
              ref="nameInput"
              v-model="map.name"
              class="name"
              :readonly="locked"
              spellcheck="false"
              placeholder="Name"
            />
          </div>
          <button
            v-for="t in TOOLS"
            :key="t.tool"
            type="button"
            class="tool"
            :class="{ on: tool === t.tool }"
            :title="t.title"
            :disabled="locked"
            @click="setTool(t.tool)"
          >
            <Icon :name="t.icon" :size="16" />
          </button>
          <div class="grow" />
          <button type="button" class="btn outline small" :disabled="locked" @click="replaceImage">
            <Icon name="image-plus" :size="14" />
            Replace image
          </button>
          <div class="zoom mono">
            <button type="button" title="Zoom out" @click="zoomStep(-1)">−</button>
            <button type="button" class="zoom-val" title="Reset zoom" @click="zoom = 1">
              {{ Math.round(zoom * 100) }}%
            </button>
            <button type="button" title="Zoom in" @click="zoomStep(1)">+</button>
          </div>
        </div>

        <div ref="stageBox" class="stage-box">
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
                <template v-for="a in areas" :key="a.id">
                  <v-rect
                    v-if="a.shape.kind === 'rect'"
                    :config="rectCfg(a)"
                    @click="onAreaClick(a)"
                    @mouseenter="onAreaEnter(a)"
                    @mouseleave="onAreaLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-ellipse
                    v-else-if="a.shape.kind === 'ellipse'"
                    :config="ellipseCfg(a)"
                    @click="onAreaClick(a)"
                    @mouseenter="onAreaEnter(a)"
                    @mouseleave="onAreaLeave"
                    @dragend="onDragEnd(a, $event)"
                    @transformend="onTransformEnd(a, $event)"
                  />
                  <v-line
                    v-else-if="a.shape.kind === 'polygon'"
                    :config="polyCfg(a)"
                    @click="onAreaClick(a)"
                    @mouseenter="onAreaEnter(a)"
                    @mouseleave="onAreaLeave"
                    @dragend="onDragEnd(a, $event)"
                  />
                  <v-path
                    v-else
                    :config="symbolCfg(a)"
                    @click="onAreaClick(a)"
                    @mouseenter="onAreaEnter(a)"
                    @mouseleave="onAreaLeave"
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
                    keepRatio: false,
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
      </template>
      <div v-else class="none">{{ props.devices.length ? "No image-map" : "No device" }}</div>
    </section>

    <!-- right: live input and areas -->
    <aside v-if="!showLog" class="col-right">
      <div class="input-card">
        <div class="ic-key">
          <Icon name="bolt" :size="22" />
          <span class="mono key" :class="{ idle: !currentKey }">{{ currentKey ?? "—" }}</span>
        </div>
        <div class="ic-sub">{{ selectedName }} · {{ currentCountText }}</div>
        <div class="ic-btns">
          <button
            type="button"
            class="btn primary small"
            :disabled="!currentKey || locked"
            @click="addDefaultArea"
          >
            <Icon name="plus" :size="13" />
            Add area
          </button>
          <button type="button" class="btn danger small" :disabled="!selectedId || locked" @click="deleteSelected">
            <Icon name="trash" :size="13" />
            Delete
          </button>
        </div>
      </div>

      <div class="areas">
        <div class="tabs">
          <div class="tab">Areas <span>{{ areas.length }}</span></div>
          <div class="grow" />
          <div class="filter">
            <Icon name="search" :size="13" />
            <input v-model="filter" class="filter-in mono" placeholder="Filter…" spellcheck="false" />
          </div>
        </div>
        <div class="rows">
          <div
            v-for="a in listedAreas"
            :key="a.id"
            class="arow"
            :class="{ cur: a.input === currentKey, sel: a.id === selectedId }"
            @click="pickArea(a)"
          >
            <Icon :name="areaIcon(a)" :size="14" />
            <span class="mono akey">{{ a.input }}</span>
            <span class="akind">{{ areaKind(a) }}</span>
          </div>
        </div>
      </div>

      <div class="right-foot">
        <button type="button" class="btn outline" :disabled="!dirty" @click="discard">Discard</button>
        <button type="button" class="btn primary grow" :disabled="!dirty" @click="saveMap">
          <Icon name="check" :size="14" />
          Save
        </button>
      </div>
    </aside>

    <ConfirmDialog v-if="confirm" :title="confirm.title" :buttons="confirm.buttons" @choose="onConfirm" />
  </section>
</template>

<style scoped>
.devices {
  flex: 1;
  display: grid;
  grid-template-columns: 300px minmax(0, 1fr) 380px;
  gap: 16px;
  padding: 12px 16px 16px;
  min-height: 0;
}

/* The log takes the canvas column and the right one. */
.devices.log {
  grid-template-columns: 300px minmax(0, 1fr);
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

/* --- left column --- */

.col-left {
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
  border: 1px solid transparent;
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

.dev-name {
  font-weight: 600;
  font-size: 14px;
}

.dev-line {
  font-size: 13px;
  color: var(--text-2);
}

.maps {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding-left: 14px;
}

.map,
.map-new {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  font-family: inherit;
  cursor: pointer;
}

.map {
  color: var(--text-2);
  border: 1px solid transparent;
}

.map.open {
  background: var(--bg-surface-2);
  color: var(--text);
}

.map.open .map-name {
  font-weight: 600;
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
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-width: 0;
  min-height: 0;
}

.head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-dim);
}

.head-name {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-right: 10px;
  margin-right: 4px;
  border-right: 1px solid var(--border-dim);
  max-width: 260px;
}

.name {
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

.head .btn.small {
  height: 34px;
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
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  background-image:
    linear-gradient(color-mix(in srgb, var(--text-2) 6%, transparent) 1px, transparent 1px),
    linear-gradient(90deg, color-mix(in srgb, var(--text-2) 6%, transparent) 1px, transparent 1px);
  background-size: 24px 24px;
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

/* --- raw log --- */

.log-title {
  font-weight: 600;
  font-size: 14px;
}

.log {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 16px;
  font-size: 12px;
}

.log-section {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--text-2);
  margin-bottom: 4px;
}

.log-events {
  margin-top: 12px;
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

.ic-key {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--live);
}

.ic-key :deep(svg) {
  fill: color-mix(in srgb, var(--live) 25%, transparent);
}

.key {
  font-size: 26px;
  line-height: 1.1;
}

.key.idle {
  color: var(--text-3);
}

.ic-sub {
  font-size: 12px;
  color: var(--text-2);
}

.ic-btns {
  display: flex;
  gap: 6px;
  margin-top: 4px;
}

.areas {
  flex: 1;
  min-height: 0;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.tabs {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 10px 12px 0;
  border-bottom: 1px solid var(--border-dim);
}

.tab {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 14px;
  border-radius: var(--radius-control) var(--radius-control) 0 0;
  background: var(--bg-surface-2);
  font-weight: 600;
  font-size: 13px;
}

.tab span {
  color: var(--text-2);
}

.filter {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 30px;
  width: 150px;
  padding: 0 10px;
  margin-bottom: 6px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text-2);
  box-sizing: border-box;
}

.filter-in {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.rows {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  font-size: 13px;
}

.arow {
  display: grid;
  grid-template-columns: 24px minmax(0, 1fr) 70px;
  gap: 10px;
  align-items: center;
  padding: 8px 14px;
  color: var(--text-2);
  cursor: pointer;
}

.arow.cur {
  color: var(--live);
}

.arow.sel {
  background: color-mix(in srgb, var(--live) 8%, transparent);
}

.akey {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.akind {
  color: var(--text-2);
}

.right-foot {
  display: flex;
  gap: 8px;
}
</style>
