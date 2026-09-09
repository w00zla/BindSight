<script setup lang="ts">
// Hardware profile editor: pick a device, pick/create a profile (around its image),
// press a physical input and draw the areas that belong to it. Konva does the
// canvas work; everything is stored normalized (0..1) in the profile.
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { KonvaEventObject, Node as KonvaNode } from "konva/lib/Node";
import type { Stage } from "konva/lib/Stage";
import type { Ellipse } from "konva/lib/shapes/Ellipse";
import type { Transformer } from "konva/lib/shapes/Transformer";
import type { VueKonvaRef } from "vue-konva";
import type { DeviceInfo, JoyInput } from "../types";
import {
  SYMBOL_PATHS,
  symbolPx,
  inputKey,
  inputLabel,
  sameHardware,
  type HwArea,
  type HwProfile,
  type HwProfileSummary,
  type SymbolKind,
} from "../hwprofile";

const props = defineProps<{ devices: DeviceInfo[] }>();
const emit = defineEmits<{ notify: [message: string, type: "ok" | "error"]; saved: [] }>();

type Tool = "select" | "rect" | "ellipse" | "polygon" | SymbolKind;
const TOOLS: Tool[] = ["select", "rect", "ellipse", "polygon", "arrow", "cw", "ccw"];
// A new symbol is 5% of the image width, square on screen.
const DEFAULT_SYMBOL_W = 0.05;
const MIN_DRAW_PX = 4;

const BLUE = "#396cd8";
const GREY = "rgba(128, 128, 128, 0.75)";

// --- device / profile selection -------------------------------------------

// Only devices SC can identify (a profile is keyed by the SC Product GUID).
const usableDevices = computed(() => props.devices.filter((d) => d.sc_product_guid));
const selectedGuid = ref("");
const device = computed(() => usableDevices.value.find((d) => d.sdl_guid === selectedGuid.value) ?? null);
const hardwareId = computed(() => device.value?.sc_product_guid ?? null);

const summaries = ref<HwProfileSummary[]>([]);
const matching = computed(() => summaries.value.filter((s) => sameHardware(s.hardware_id, hardwareId.value)));

const profileId = ref("");
const profile = ref<HwProfile | null>(null);
// Serialized state as last stored, for the dirty indicator.
const savedJson = ref("");
const dirty = computed(() => !!profile.value && JSON.stringify(profile.value) !== savedJson.value);
const currentSummary = computed(() => summaries.value.find((s) => s.id === profile.value?.id) ?? null);
const isBundled = computed(() => currentSummary.value?.source === "bundled");
const busy = ref(false);

// --- canvas state ----------------------------------------------------------

const currentImage = computed(() => profile.value?.image ?? null);
const imageAreas = computed<HwArea[]>(() => profile.value?.areas ?? []);

const imgEl = ref<HTMLImageElement | null>(null);
const natW = ref(0);
const natH = ref(0);
const wrap = ref<HTMLElement | null>(null);
const boxW = ref(0);
// Stage size in display pixels; normalized coords map straight onto it.
const W = computed(() => (natW.value && boxW.value ? boxW.value : 0));
const H = computed(() => (W.value ? (W.value * natH.value) / natW.value : 0));

const tool = ref<Tool>("select");
const selectedId = ref<string | null>(null);
const selectedArea = computed(() => imageAreas.value.find((a) => a.id === selectedId.value) ?? null);
const hover = ref<{ x: number; y: number; text: string } | null>(null);

const currentInput = ref<string | null>(null);
const inputLocked = ref(false);
const currentInputCount = computed(() =>
  currentInput.value ? (profile.value?.areas ?? []).filter((a) => a.input === currentInput.value).length : 0,
);

// Drag-drawn rect/ellipse in progress, in stage pixels.
const draft = ref<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
// Polygon under construction, flat [x0, y0, ...] in stage pixels.
const draftPoly = ref<number[]>([]);
const pointer = ref<{ x: number; y: number } | null>(null);

const stageRef = ref<VueKonvaRef<Stage> | null>(null);
const trRef = ref<VueKonvaRef<Transformer> | null>(null);

let areaSeq = 0;
function newId(): string {
  areaSeq += 1;
  return `a${Date.now().toString(36)}${areaSeq}`;
}

// --- loading ---------------------------------------------------------------

async function loadSummaries() {
  try {
    summaries.value = await invoke<HwProfileSummary[]>("list_hw_profiles");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function loadProfile(id: string) {
  try {
    const p = await invoke<HwProfile>("get_hw_profile", { id });
    profile.value = p;
    savedJson.value = JSON.stringify(p);
    selectedId.value = null;
  } catch (e) {
    profile.value = null;
    emit("notify", String(e), "error");
  }
}

// Data URLs of profile images, keyed by `<profile id>/<file>`.
const imgCache = new Map<string, string>();
async function imageUrl(id: string, file: string): Promise<string> {
  const key = `${id}/${file}`;
  const hit = imgCache.get(key);
  if (hit) return hit;
  const url = await invoke<string>("read_hw_profile_image", { id, file });
  imgCache.set(key, url);
  return url;
}

async function loadCanvasImage() {
  imgEl.value = null;
  natW.value = 0;
  natH.value = 0;
  const p = profile.value;
  const im = currentImage.value;
  if (!p || !im) return;
  try {
    const url = await imageUrl(p.id, im.file);
    const el = new Image();
    el.onload = () => {
      natW.value = el.naturalWidth;
      natH.value = el.naturalHeight;
      imgEl.value = el;
    };
    el.onerror = () => emit("notify", `Cannot decode ${im.file}`, "error");
    el.src = url;
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

watch(
  usableDevices,
  (list) => {
    if (!list.some((d) => d.sdl_guid === selectedGuid.value)) selectedGuid.value = list[0]?.sdl_guid ?? "";
  },
  { immediate: true },
);

// The canvas only exists once a profile is open; follow the element.
let ro: ResizeObserver | null = null;
watch(wrap, (el) => {
  ro?.disconnect();
  ro = null;
  if (!el) return;
  boxW.value = el.clientWidth;
  ro = new ResizeObserver(() => {
    boxW.value = el.clientWidth;
  });
  ro.observe(el);
});

watch([hardwareId, summaries], () => {
  if (!matching.value.some((s) => s.id === profileId.value)) profileId.value = matching.value[0]?.id ?? "";
});

watch(profileId, (id) => {
  if (id) loadProfile(id);
  else {
    profile.value = null;
    savedJson.value = "";
  }
});

watch([() => currentImage.value?.file, () => profile.value?.id], () => {
  selectedId.value = null;
  cancelDraw();
  loadCanvasImage();
});

// --- profile actions -------------------------------------------------------

// Pick an image file, or null when the dialog was cancelled.
async function pickImage(): Promise<string | null> {
  const src = await open({
    multiple: false,
    filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp"] }],
  });
  return src ?? null;
}

// "New" opens an inline form: a name and the image the areas will be drawn
// on. Picking the image creates the profile right away.
const draftNew = ref<{ name: string } | null>(null);

function startNew() {
  const d = device.value;
  if (!d?.sc_product_guid) return;
  draftNew.value = { name: d.sc_name ?? d.sdl_name };
}

async function createProfile() {
  const d = device.value;
  const draft = draftNew.value;
  if (!d?.sc_product_guid || !draft) return;
  busy.value = true;
  try {
    const imagePath = await pickImage();
    if (!imagePath) return;
    const p = await invoke<HwProfile>("create_hw_profile", {
      name: draft.name.trim() || (d.sc_name ?? d.sdl_name),
      hardwareId: d.sc_product_guid,
      hardwareName: d.sc_name ?? "",
      imagePath,
    });
    draftNew.value = null;
    await loadSummaries();
    profileId.value = p.id;
    emit("notify", "Image-map created", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function saveProfile() {
  if (!profile.value) return;
  busy.value = true;
  try {
    const p = await invoke<HwProfile>("save_hw_profile", { profile: profile.value });
    profile.value = p;
    savedJson.value = JSON.stringify(p);
    await loadSummaries();
    emit("saved");
    emit("notify", "Image-map saved", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function deleteProfile() {
  const p = profile.value;
  if (!p || currentSummary.value?.source !== "user") return;
  busy.value = true;
  try {
    await invoke("delete_hw_profile", { id: p.id });
    profileId.value = "";
    await loadSummaries();
    profileId.value = matching.value[0]?.id ?? "";
    emit("saved");
    emit("notify", "Image-map deleted", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function exportProfile() {
  const p = profile.value;
  if (!p) return;
  try {
    const dest = await save({
      defaultPath: `${p.name || "image-map"}.zip`,
      filters: [{ name: "Image-map", extensions: ["zip"] }],
    });
    if (!dest) return;
    await invoke("export_hw_profile", { id: p.id, destPath: dest });
    emit("notify", "Image-map exported", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function importProfile() {
  try {
    const src = await open({ multiple: false, filters: [{ name: "Image-map", extensions: ["zip"] }] });
    if (!src) return;
    const s = await invoke<HwProfileSummary>("import_hw_profile", { sourcePath: src });
    imgCache.clear();
    await loadSummaries();
    if (sameHardware(s.hardware_id, hardwareId.value)) profileId.value = s.id;
    emit("saved");
    emit("notify", `Imported ${s.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// --- image -----------------------------------------------------------------

// Replace the profile's image. The areas stay (a re-shot of the same view
// keeps them useful); the old file is removed once the new one is in.
async function replaceImage() {
  const p = profile.value;
  if (!p) return;
  try {
    const src = await pickImage();
    if (!src) return;
    const old = p.image.file;
    const img = await invoke<{ file: string; label: string }>("add_hw_profile_image", {
      id: p.id,
      sourcePath: src,
    });
    p.image = img;
    if (old !== img.file) {
      await invoke("remove_hw_profile_image", { id: p.id, file: old });
      imgCache.delete(`${p.id}/${old}`);
    }
    // The file is on disk already — keep profile.json in step with it.
    await saveProfile();
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// --- live input capture ----------------------------------------------------

let unlisten: UnlistenFn[] = [];

onMounted(async () => {
  unlisten.push(
    await listen<JoyInput>("joy-input", (e) => {
      if (inputLocked.value) return;
      if (e.payload.guid !== selectedGuid.value) return;
      const key = inputKey(e.payload);
      if (key) currentInput.value = key;
    }),
  );
  window.addEventListener("keydown", onKeyDown);
  await loadSummaries();
});

onUnmounted(() => {
  unlisten.forEach((fn) => fn());
  unlisten = [];
  window.removeEventListener("keydown", onKeyDown);
  ro?.disconnect();
  ro = null;
});

function onKeyDown(e: KeyboardEvent) {
  const t = e.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
  if (e.key === "Escape") {
    cancelDraw();
    selectedId.value = null;
  } else if (e.key === "Enter") {
    commitPolygon();
  } else if (e.key === "Delete" || e.key === "Backspace") {
    deleteSelected();
  }
}

// --- normalized <-> stage pixels ------------------------------------------

function addArea(shape: HwArea["shape"]) {
  const p = profile.value;
  if (!p || !currentInput.value) return;
  const area: HwArea = { id: newId(), input: currentInput.value, shape };
  p.areas.push(area);
  selectedId.value = area.id;
  tool.value = "select";
}

// Drop every area of the profile (unsaved until Save, like any edit).
function removeAllAreas() {
  const p = profile.value;
  if (!p) return;
  p.areas = [];
  selectedId.value = null;
  cancelDraw();
}

function setTool(t: Tool) {
  cancelDraw();
  tool.value = t;
  if (t !== "select") selectedId.value = null;
}

function cancelDraw() {
  draft.value = null;
  draftPoly.value = [];
}

function deleteSelected() {
  const p = profile.value;
  if (!p || !selectedId.value) return;
  p.areas = p.areas.filter((a) => a.id !== selectedId.value);
  selectedId.value = null;
}

function stagePointer(): { x: number; y: number } | null {
  const stage = stageRef.value?.getStage();
  return stage?.getPointerPosition() ?? null;
}

function onStageMouseDown(e: KonvaEventObject<MouseEvent>) {
  if (!W.value) return;
  const pos = stagePointer();
  if (!pos) return;
  if (tool.value === "select") {
    // A click on the bare image clears the selection.
    if (e.target === e.target.getStage() || e.target.name() === "bg") selectedId.value = null;
    return;
  }
  if (!currentInput.value) {
    emit("notify", "Press an input first", "error");
    return;
  }
  if (tool.value === "rect" || tool.value === "ellipse") {
    draft.value = { x0: pos.x, y0: pos.y, x1: pos.x, y1: pos.y };
  } else if (tool.value === "polygon") {
    draftPoly.value = [...draftPoly.value, pos.x, pos.y];
  } else {
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

function colours(a: HwArea) {
  const isCurrent = a.input === currentInput.value;
  const selected = a.id === selectedId.value;
  return {
    stroke: isCurrent ? BLUE : GREY,
    strokeWidth: selected ? 3 : isCurrent ? 2 : 1,
    fill: isCurrent ? "rgba(57, 108, 216, 0.25)" : "rgba(128, 128, 128, 0.10)",
    opacity: isCurrent ? 1 : 0.6,
    dash: selected ? [6, 3] : undefined,
  };
}

const canSelect = computed(() => tool.value === "select");

function rectCfg(a: HwArea) {
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
    draggable: canSelect.value,
    ...colours(a),
  };
}

function ellipseCfg(a: HwArea) {
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
    draggable: canSelect.value,
    ...colours(a),
  };
}

function polyCfg(a: HwArea) {
  if (a.shape.kind !== "polygon") return {};
  return {
    id: a.id,
    x: 0,
    y: 0,
    points: a.shape.points.flatMap(([x, y]) => [x * W.value, y * H.value]),
    closed: true,
    draggable: canSelect.value,
    ...colours(a),
  };
}

function symbolCfg(a: HwArea) {
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
    draggable: canSelect.value,
    ...colours(a),
    strokeScaleEnabled: false,
  };
}

// --- shape edits -----------------------------------------------------------

function onAreaClick(a: HwArea) {
  if (!canSelect.value) return;
  selectedId.value = a.id;
}

function onAreaEnter(a: HwArea) {
  const pos = stagePointer();
  hover.value = pos ? { x: pos.x + 10, y: pos.y + 10, text: inputLabel(a.input) } : null;
}

function onAreaLeave() {
  hover.value = null;
}

function onDragEnd(a: HwArea, e: KonvaEventObject<DragEvent>) {
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

function onTransformEnd(a: HwArea, e: KonvaEventObject<Event>) {
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
  if (!a || a.shape.kind !== "polygon" || !canSelect.value) return [];
  return a.shape.points.map(([x, y], i) => ({ i, x: x * W.value, y: y * H.value }));
});

function onVertexDrag(i: number, e: KonvaEventObject<DragEvent>) {
  const a = selectedArea.value;
  if (!a || a.shape.kind !== "polygon") return;
  a.shape.points[i] = [e.target.x() / W.value, e.target.y() / H.value];
}

// Attach/detach the transformer whenever the selection or the shapes change.
watch(
  [selectedId, canSelect, imageAreas, W],
  async () => {
    await nextTick();
    const tr = trRef.value?.getNode();
    const stage = stageRef.value?.getStage();
    if (!tr || !stage) return;
    const a = selectedArea.value;
    if (!a || a.shape.kind === "polygon" || !canSelect.value) {
      tr.nodes([]);
      return;
    }
    const node = stage.findOne<KonvaNode>(`#${a.id}`);
    tr.nodes(node ? [node] : []);
  },
  { deep: true },
);

// Inputs with areas, with their counts.
const imageInputs = computed(() => {
  const m = new Map<string, number>();
  for (const a of imageAreas.value) m.set(a.input, (m.get(a.input) ?? 0) + 1);
  return [...m.entries()].sort((x, y) => x[0].localeCompare(y[0]));
});

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
</script>

<template>
  <section class="editor">
    <div class="panel-title">Image-maps</div>
    <div class="row">
      <select v-model="selectedGuid" class="pick">
        <option v-if="!usableDevices.length" value="">No device</option>
        <option v-for="d in usableDevices" :key="d.sdl_guid" :value="d.sdl_guid">
          {{ d.sc_name ?? d.sdl_name }}
        </option>
      </select>
      <select v-model="profileId" class="pick">
        <option value="">— no image-map —</option>
        <option v-for="s in matching" :key="s.id" :value="s.id">
          {{ s.name }}
        </option>
      </select>
      <span v-if="isBundled" class="tag">bundled</span>
      <button :disabled="!device || busy || !!draftNew" @click="startNew">New</button>
      <button :disabled="currentSummary?.source !== 'user' || busy" @click="deleteProfile">Delete</button>
      <button :disabled="!profile" @click="exportProfile">Export…</button>
      <button @click="importProfile">Import…</button>
    </div>

    <div v-if="draftNew" class="newbox">
      <div class="row">
        <span class="cur">New image-map for {{ device?.sc_name ?? device?.sdl_name }}</span>
      </div>
      <div class="row">
        <input v-model="draftNew.name" class="txt" placeholder="Name" />
      </div>
      <div class="row">
        <button :disabled="busy" @click="createProfile">Choose image…</button>
        <span class="hint">Photo or drawing of the device (png/jpg/webp); the input areas are drawn on it. Picking it creates the profile.</span>
      </div>
      <div class="row">
        <button :disabled="busy" @click="draftNew = null">Cancel</button>
      </div>
    </div>

    <template v-if="profile && !draftNew">
      <div class="row">
        <input v-model="profile.name" class="txt" placeholder="Name" />
        <button :disabled="!dirty || busy" @click="saveProfile">Save</button>
        <span v-if="dirty" class="tag warn">unsaved</span>
      </div>

      <div class="row tabs">
        <button @click="replaceImage">Replace image…</button>
      </div>

      <div class="row inputbox">
        <span class="cur">{{ currentInput ? inputLabel(currentInput) : "Press an input…" }}</span>
        <span v-if="currentInput" class="tag">{{ currentInputCount }} area(s)</span>
        <label class="lock"><input v-model="inputLocked" type="checkbox" /> Lock</label>
      </div>

      <div class="row tools">
        <button v-for="t in TOOLS" :key="t" :class="{ on: tool === t }" @click="setTool(t)">{{ t }}</button>
        <button :disabled="!selectedId" @click="deleteSelected">Delete area</button>
        <button :disabled="!imageAreas.length" @click="removeAllAreas">Remove all areas</button>
      </div>

      <div ref="wrap" class="canvas">
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
            <template v-for="a in imageAreas" :key="a.id">
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
              :config="{ ...drawPreview, stroke: BLUE, dash: [4, 4], listening: false }"
            />
            <v-ellipse
              v-if="drawPreview && tool === 'ellipse'"
              :config="{
                x: drawPreview.x + drawPreview.width / 2,
                y: drawPreview.y + drawPreview.height / 2,
                radiusX: drawPreview.width / 2,
                radiusY: drawPreview.height / 2,
                stroke: BLUE,
                dash: [4, 4],
                listening: false,
              }"
            />
            <v-line
              v-if="polyPreview"
              :config="{ points: polyPreview, stroke: BLUE, dash: [4, 4], closed: false, listening: false }"
            />

            <v-circle
              v-for="v in vertexAnchors"
              :key="`v${v.i}`"
              :config="{ x: v.x, y: v.y, radius: 5, fill: '#fff', stroke: BLUE, strokeWidth: 2, draggable: true }"
              @dragmove="onVertexDrag(v.i, $event)"
            />

            <v-transformer
              ref="trRef"
              :config="{ rotateEnabled: true, keepRatio: false, anchorSize: 8, borderStroke: BLUE, anchorStroke: BLUE }"
            />

            <v-label v-if="hover" :config="{ x: hover.x, y: hover.y, listening: false }">
              <v-tag :config="{ fill: 'rgba(0,0,0,0.75)', cornerRadius: 3 }" />
              <v-text :config="{ text: hover.text, fill: '#fff', fontSize: 12, padding: 4 }" />
            </v-label>
          </v-layer>
        </v-stage>
        <p v-else class="empty">Loading image…</p>
      </div>

      <ul v-if="imageInputs.length" class="inputs">
        <li v-for="[key, count] in imageInputs" :key="key">
          <button :class="{ on: key === currentInput }" @click="currentInput = key">
            {{ inputLabel(key) }} <span class="n">{{ count }}</span>
          </button>
        </li>
      </ul>
    </template>

    <p v-else-if="!draftNew" class="empty">
      {{ usableDevices.length ? "No image-map selected." : "No device with an SC product GUID." }}
    </p>
  </section>
</template>

<style scoped>
.editor {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
  margin-top: 1rem;
}

.row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.pick,
.txt {
  padding: 0.4em 0.6em;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.4);
  background: transparent;
  color: inherit;
  font-family: inherit;
  font-size: 0.9rem;
}

.txt {
  flex: 1;
  min-width: 8rem;
}

.txt.short {
  flex: 0 0 9rem;
  min-width: 0;
}

.file {
  font-size: 0.8rem;
  opacity: 0.6;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.tag {
  font-size: 0.75rem;
  font-weight: 600;
  color: #7f8c8d;
  background: rgba(127, 140, 141, 0.16);
  border-radius: 4px;
  padding: 0.05rem 0.35rem;
  white-space: nowrap;
}

.tag.warn {
  color: #b9770e;
  background: rgba(230, 126, 34, 0.14);
}

.inputbox {
  border: 1px solid rgba(128, 128, 128, 0.3);
  border-radius: 10px;
  background: rgba(128, 128, 128, 0.06);
  padding: 0.5rem 0.8rem;
}

.cur {
  font-weight: 600;
}

.lock {
  margin-left: auto;
  font-size: 0.85rem;
  opacity: 0.8;
  display: flex;
  align-items: center;
  gap: 0.3rem;
}

.tabs button,
.tools button,
.inputs button {
  padding: 0.25em 0.7em;
  font-size: 0.8rem;
  font-weight: 500;
  box-shadow: none;
  border: 1px solid rgba(128, 128, 128, 0.4);
  background: transparent;
  color: inherit;
}

.tabs button.on,
.tools button.on,
.inputs button.on {
  border-color: #396cd8;
  background: rgba(57, 108, 216, 0.14);
}

.canvas {
  border: 1px solid rgba(128, 128, 128, 0.3);
  border-radius: 10px;
  overflow: hidden;
  line-height: 0;
  min-height: 2rem;
}

.inputs {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 0.35rem;
}

.inputs .n {
  opacity: 0.55;
  margin-left: 0.25rem;
}

.empty {
  opacity: 0.7;
}

.newbox {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding: 0.75rem 0.9rem;
  border: 1px solid rgba(128, 128, 128, 0.35);
  border-radius: 10px;
  background: rgba(128, 128, 128, 0.06);
}

.hint {
  font-size: 0.8rem;
  opacity: 0.7;
}
</style>
