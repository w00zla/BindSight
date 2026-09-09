<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import ImageMapEditor from "./components/ImageMapEditor.vue";
import TopBar from "./components/TopBar.vue";
import DeviceTile from "./components/DeviceTile.vue";
import StatusPanel from "./components/StatusPanel.vue";
import ImageStage from "./components/ImageStage.vue";
import LiveCard from "./components/LiveCard.vue";
import BindingsDeck from "./components/BindingsDeck.vue";
import ToolsView from "./components/ToolsView.vue";
import Toasts from "./components/Toasts.vue";
import Splitter from "./components/Splitter.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import type {
  ActionMap,
  BoundAction,
  ClashReport,
  Config,
  CurrentInput,
  DeviceInfo,
  Environment,
  ImageMapView,
  JoyInput,
  LoadStatus,
  LoggedInput,
  Mode,
  ResolvedBinding,
  ScStatus,
  SlotStatus,
} from "./types";
import { sameHardware, type HighlightClass, type ImageMap, type ImageMapSummary } from "./imagemap";

const MAX_EVENTS = 500;

const devices = ref<DeviceInfo[]>([]);
const events = ref<LoggedInput[]>([]);
const actionMaps = ref<ActionMap[]>([]);
// SC environments (Settings) and the one being read (top-bar chip).
const environments = ref<Record<string, Environment>>({});
const activeEnv = ref("LIVE");
const bindings = ref<ResolvedBinding[]>([]);
const tokens = ref<Record<string, string>>({});
const currentInput = ref<CurrentInput | null>(null);
const clash = ref<ClashReport | null>(null);
// The install's version and game-data load state (updated via `scdata-changed`).
const scStatus = ref<ScStatus | null>(null);
// Set while a base-path change is being loaded, so its result gets a toast.
let awaitingPathLoad = false;
// SC Product GUIDs the user marked "SC doesn't see this device" (persisted per OS).
const ignoredDevices = ref<string[]>([]);
const error = ref<string | null>(null);
// The live actionmaps.xml is parsed (from the last LoadStatus).
const profileLoaded = ref(false);
const loading = ref(false);
const showSettings = ref(false);

// --- image-maps --------------------------------------------------------

// How long an axis stays highlighted after its last event (axes never rest).
const AXIS_PULSE_MS = 400;

const mode = ref<Mode>("live");
const mapSummaries = ref<ImageMapSummary[]>([]);
// Map id -> full image-map, and `<map id>/<file>` -> image data URL.
const loadedMaps = ref<Record<string, ImageMap>>({});
const mapImages = ref<Record<string, string>>({});
// Lowercase hardware id -> chosen map id (from config.json).
const mapChoices = ref<Record<string, string>>({});
// SDL GUID -> input key -> highlight class, for the currently active inputs.
const activeInputs = ref<Record<string, Record<string, HighlightClass>>>({});
const axisTimers = new Map<string, number>();

function mapsFor(guid: string | null): ImageMapSummary[] {
  return mapSummaries.value.filter((s) => sameHardware(s.hardware_id, guid));
}

// The user's pick for this device, else the single/first matching map.
function chosenMapId(guid: string | null): string | null {
  const list = mapsFor(guid);
  if (!list.length) return null;
  const pick = guid ? mapChoices.value[guid.toLowerCase()] : undefined;
  return list.find((s) => s.id === pick)?.id ?? list[0].id;
}

function imgSrc(id: string, file: string): string {
  return mapImages.value[`${id}/${file}`] ?? "";
}

const mapViews = computed<ImageMapView[]>(() =>
  devices.value.flatMap((d) => {
    const id = chosenMapId(d.sc_product_guid);
    const p = id ? loadedMaps.value[id] : null;
    return p ? [{ device: d, map: p, options: mapsFor(d.sc_product_guid) }] : [];
  }),
);

// Resizable layout: stage height and live-card width, remembered locally.
const LAYOUT_KEY = "bindsight.layout";
const STAGE_H = { def: 440, min: 200, max: 900 };
const LIVE_W = { def: 400, min: 280, max: 800 };
const stageHeight = ref(STAGE_H.def);
const liveWidth = ref(LIVE_W.def);
try {
  const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}") as { stageHeight?: number; liveWidth?: number };
  if (typeof saved.stageHeight === "number") stageHeight.value = saved.stageHeight;
  if (typeof saved.liveWidth === "number") liveWidth.value = saved.liveWidth;
} catch {
  /* defaults */
}
let layoutStart: { stageHeight: number; liveWidth: number } | null = null;
const clamp = (v: number, r: { min: number; max: number }) => Math.min(Math.max(v, r.min), r.max);
function dragStage(delta: number) {
  layoutStart ??= { stageHeight: stageHeight.value, liveWidth: liveWidth.value };
  stageHeight.value = clamp(layoutStart.stageHeight + delta, STAGE_H);
}
function dragLive(delta: number) {
  layoutStart ??= { stageHeight: stageHeight.value, liveWidth: liveWidth.value };
  liveWidth.value = clamp(layoutStart.liveWidth + delta, LIVE_W);
}
function saveLayout() {
  layoutStart = null;
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ stageHeight: stageHeight.value, liveWidth: liveWidth.value }));
  } catch {
    /* ignore */
  }
}
function resetStage() {
  stageHeight.value = STAGE_H.def;
  saveLayout();
}
function resetLive() {
  liveWidth.value = LIVE_W.def;
  saveLayout();
}

// Connected, not excluded devices without an image-map (placeholder tiles).
const stagePlaceholders = computed<DeviceInfo[]>(() =>
  devices.value.filter(
    (d) => !isIgnored(d.sc_product_guid) && !mapViews.value.some((v) => v.device.index === d.index),
  ),
);

async function loadMapImage(id: string, file: string) {
  const key = `${id}/${file}`;
  if (mapImages.value[key]) return;
  try {
    mapImages.value[key] = await invoke<string>("read_imagemap_image", { id, file });
  } catch {
    /* a missing image just stays blank */
  }
}

async function loadChosenMaps() {
  for (const d of devices.value) {
    const id = chosenMapId(d.sc_product_guid);
    if (!id || loadedMaps.value[id]) continue;
    try {
      const p = await invoke<ImageMap>("get_imagemap", { id });
      loadedMaps.value[id] = p;
      await loadMapImage(p.id, p.image.file);
    } catch {
      /* skip a map that will not load */
    }
  }
}

async function reloadMaps() {
  try {
    mapSummaries.value = await invoke<ImageMapSummary[]>("list_imagemaps");
  } catch (e) {
    error.value = String(e);
    return;
  }
  await loadChosenMaps();
}

// A save in the editor can change the image and areas — drop the caches.
async function onMapsSaved() {
  loadedMaps.value = {};
  mapImages.value = {};
  await reloadMaps();
}

// The editor guards its unsaved changes; leaving Devices can be refused.
const editor = ref<InstanceType<typeof ImageMapEditor> | null>(null);

async function setMode(m: Mode) {
  if (mode.value === "devices" && m !== "devices") {
    if ((await editor.value?.requestLeave()) === false) return;
  }
  mode.value = m;
  // Inputs released while the editor was open were never seen here.
  activeInputs.value = {};
  if (m === "live") await reloadMaps();
}

async function setMapChoice(guid: string | null, id: string) {
  if (!guid) return;
  try {
    const cfg = await invoke<{ imagemap_choices: Record<string, string> }>("set_imagemap_choice", {
      hardwareId: guid,
      imagemapId: id || null,
    });
    mapChoices.value = cfg.imagemap_choices;
    await loadChosenMaps();
  } catch (e) {
    notify(String(e), "error");
  }
}

function setActive(guid: string, key: string, cls: HighlightClass) {
  const cur = activeInputs.value[guid] ?? {};
  activeInputs.value = { ...activeInputs.value, [guid]: { ...cur, [key]: cls } };
}

function clearActive(guid: string, drop: (key: string) => boolean) {
  const cur = activeInputs.value[guid];
  if (!cur) return;
  const next: Record<string, HighlightClass> = {};
  for (const [k, v] of Object.entries(cur)) if (!drop(k)) next[k] = v;
  activeInputs.value = { ...activeInputs.value, [guid]: next };
}

// A binding clicked in the list, kept lit on the image-map image until clicked
// again or another one is picked.
const pinned = ref<{ guid: string; key: string } | null>(null);

function activeFor(guid: string): Map<string, HighlightClass> {
  const m = new Map<string, HighlightClass>(Object.entries(activeInputs.value[guid] ?? {}));
  if (pinned.value?.guid === guid) m.set(pinned.value.key, "bound");
  return m;
}

// Image-map input key for an SC token on a device. Undoes the +1 offset of
// button/hat numbering; axes go through the device's HID-derived axis names
// (`js2_rotz` -> the SDL index whose name is `rotz`).
function inputKeyForToken(token: string, d: DeviceInfo): string | null {
  const b = token.match(/^js\d+_button(\d+)$/);
  if (b) return `button:${Number(b[1]) - 1}`;
  const h = token.match(/^js\d+_hat(\d+)_(up|down|left|right)$/);
  if (h) return `hat:${Number(h[1]) - 1}:${h[2]}`;
  const a = token.match(/^js\d+_([a-z0-9]+)$/);
  if (a) {
    const i = d.axes.indexOf(a[1]);
    if (i >= 0) return `axis:${i}`;
  }
  return null;
}

// Does the map shown for this device (by SDL GUID) have an area for the
// input? `null` when the device has no image-map.
function inMap(sdlGuid: string, key: string): boolean | null {
  const d = devices.value.find((dev) => dev.sdl_guid === sdlGuid);
  const id = chosenMapId(d?.sc_product_guid ?? null);
  const p = id ? loadedMaps.value[id] : null;
  if (!p) return null;
  return p.areas.some((a) => a.input === key);
}

// Can a binding in the list be lit on an image-map image? Needs a connected
// device with an image-map and a mappable token.
function pinTarget(b: ResolvedBinding): { guid: string; key: string } | null {
  const r = resolvePin(b);
  return "reason" in r ? null : r;
}

// Where a binding would light up, or why it cannot.
function resolvePin(b: ResolvedBinding): { guid: string; key: string } | { reason: string } {
  const d = devices.value.find((dev) => !!b.device_guid && sameHardware(dev.sc_product_guid, b.device_guid));
  if (!d) return { reason: `${b.device ?? "Device"} not connected` };
  const key = inputKeyForToken(b.token, d);
  if (!key) return { reason: `${tokenLabel(b.token)}: no SDL axis for it (${d.axes_error ?? "not in HID descriptor"})` };
  const has = inMap(d.sdl_guid, key);
  if (has === null) return { reason: `${d.sc_name ?? d.sdl_name} has no image-map` };
  if (!has) return { reason: `No area for ${tokenLabel(b.token)}` };
  return { guid: d.sdl_guid, key };
}

// The device has an image-map, but no area for this binding's input.
function missingInMap(b: ResolvedBinding): boolean {
  const t = pinTarget(b);
  return !!t && inMap(t.guid, t.key) === false;
}

function isPinned(b: ResolvedBinding): boolean {
  const t = pinTarget(b);
  return !!t && pinned.value?.guid === t.guid && pinned.value.key === t.key;
}

function togglePin(b: ResolvedBinding) {
  const r = resolvePin(b);
  if ("reason" in r) {
    notify(r.reason, "error");
    return;
  }
  pinned.value = isPinned(b) ? null : r;
}

// Buttons stay lit while held, hats until centered, axes pulse.
function trackActive(p: JoyInput) {
  if (p.kind === "button") {
    if (p.pressed) setActive(p.guid, `button:${p.index}`, "none");
    else clearActive(p.guid, (k) => k === `button:${p.index}`);
    return;
  }
  if (p.kind === "hat") {
    clearActive(p.guid, (k) => k.startsWith(`hat:${p.index}:`));
    if (p.direction !== "centered") setActive(p.guid, `hat:${p.index}:${p.direction}`, "none");
    return;
  }
  const key = `axis:${p.index}`;
  setActive(p.guid, key, "none");
  const tk = `${p.guid}#${key}`;
  const prev = axisTimers.get(tk);
  if (prev) clearTimeout(prev);
  axisTimers.set(
    tk,
    window.setTimeout(() => {
      clearActive(p.guid, (k) => k === key);
      axisTimers.delete(tk);
    }, AXIS_PULSE_MS),
  );
}

// SC's own display label for a full token, e.g. "js2_button1" -> "Button 1
// (Input 2)". Falls back to the raw token if SC has no label for it.
function tokenLabel(token: string | null): string {
  if (!token) return "—";
  return tokens.value[token] ?? token;
}

// Localized actionmap (category) label, e.g. "spaceship_movement" -> its label.
function actionmapLabel(name: string): string {
  return actionMaps.value.find((m) => m.name === name)?.label ?? name;
}

// Is the device recorded for this binding currently connected (matched by GUID)?
function isConnected(b: ResolvedBinding): boolean {
  return !!b.device_guid && devices.value.some((d) => d.sc_product_guid === b.device_guid);
}

// SC instance number from a token, e.g. "js2_button9" -> "2".
function instanceOf(token: string): string {
  return token.match(/^js(\d+)_/)?.[1] ?? "?";
}

// Number of bindings assigned to a device.
function bindingCountFor(guid: string | null): number {
  if (!guid) return 0;
  return bindings.value.filter((b) => b.device_guid === guid).length;
}

// Connected-slot status by GUID, from the clash report.
const slotByGuid = computed<Map<string, SlotStatus>>(() => {
  const m = new Map<string, SlotStatus>();
  for (const s of clash.value?.connected ?? []) {
    if (s.sc_product_guid) m.set(s.sc_product_guid, s);
  }
  return m;
});

function slotFor(guid: string | null): SlotStatus | null {
  return guid ? slotByGuid.value.get(guid) ?? null : null;
}

// GUIDs SC did not list at its last start (device-order log source only).
const unseenGuids = computed<Set<string>>(
  () => new Set((clash.value?.unseen ?? []).map((u) => u.sc_product_guid).filter((g): g is string => !!g)),
);

function isUnseen(guid: string | null): boolean {
  return !!guid && unseenGuids.value.has(guid);
}

function isIgnored(guid: string | null): boolean {
  return !!guid && ignoredDevices.value.some((g) => g.toLowerCase() === guid.toLowerCase());
}
// Settings dialog Save: apply the exclusions and the environments; the
// backend reloads when the active environment changed.
async function applySettings(s: { environments: Record<string, Environment>; ignored: string[] }) {
  showSettings.value = false;
  try {
    ignoredDevices.value = await invoke<string[]>("set_ignored_devices", { guids: s.ignored });
    const reloading = await invoke<boolean>("set_environments", { environments: s.environments });
    environments.value = s.environments;
    if (reloading) {
      await awaitScLoad();
    } else {
      await loadClash();
    }
  } catch (e) {
    notify(String(e), "error");
  }
}

// Top-bar chip: switch the environment the app reads.
async function switchEnv(slug: string) {
  try {
    const changed = await invoke<boolean>("set_active_env", { slug });
    activeEnv.value = slug;
    if (changed) await awaitScLoad();
  } catch (e) {
    notify(String(e), "error");
  }
}

// Instances whose bindings still land on the right device (the device SC now
// assigns that jsN is the one saved under it). Any other instance is misdirected.
const healthyInstances = computed<Set<number>>(() => {
  const s = new Set<number>();
  for (const slot of clash.value?.connected ?? []) {
    if (slot.stored_instance !== null && slot.stored_instance === slot.effective_instance) {
      s.add(slot.effective_instance);
    }
  }
  return s;
});

// Is this binding's slot misdirected by the current device-order clash?
function bindingClash(token: string): boolean {
  if (!clash.value?.has_clash) return false;
  const n = Number(instanceOf(token));
  return Number.isFinite(n) && !healthyInstances.value.has(n);
}

async function loadClash() {
  try {
    clash.value = await invoke<ClashReport>("get_clash_report");
  } catch {
    clash.value = null;
  }
}

// Whether the clash panel has anything to show (a clash, or no usable order source).

// Put the pp_resortdevices commands on the clipboard, one per line.
async function copyResortCommands() {
  const text = (clash.value?.resort_commands ?? []).join("\n");
  try {
    await navigator.clipboard.writeText(text);
    notify("Commands copied", "ok");
  } catch (e) {
    notify(`Copy failed: ${String(e)}`, "error");
  }
}

// Rewrite actionmaps.xml with the resort (SC must be closed) and reload.
async function applyResort() {
  try {
    const s = await invoke<LoadStatus>("apply_resort");
    takeStatus(s);
    await loadClash();
    if (s.loaded) {
      notify("actionmaps.xml resorted (backup kept next to it)", "ok");
    } else {
      notify(s.error ?? "Reload failed", "error");
    }
  } catch (e) {
    notify(String(e), "error");
  }
}

// Resolve a live button/hat input to its token and bound action(s) and show it.
// Axes stream events; resolve one per axis at most every AXIS_RESOLVE_MS.
const AXIS_RESOLVE_MS = 150;
const lastAxisResolve = new Map<string, number>();
function axisDue(guid: string, index: number): boolean {
  const k = `${guid}#${index}`;
  const now = Date.now();
  if (now - (lastAxisResolve.get(k) ?? 0) < AXIS_RESOLVE_MS) return false;
  lastAxisResolve.set(k, now);
  return true;
}

async function showBinding(
  guid: string,
  kind: "button" | "hat" | "axis",
  index: number,
  direction: string | null,
) {
  try {
    const res = await invoke<{ token: string | null; actions: BoundAction[] }>("resolve_input", {
      guid,
      kind,
      index,
      direction,
    });
    const key = kind === "button" ? `button:${index}` : kind === "axis" ? `axis:${index}` : `hat:${index}:${direction}`;
    currentInput.value = {
      device: nameFor(guid),
      sc_guid: devices.value.find((d) => d.sdl_guid === guid)?.sc_product_guid ?? null,
      token: res.token,
      sdl: sdlInputName(kind, index, direction),
      actions: res.actions,
      in_imagemap: inMap(guid, key),
    };
    if (res.actions.length && currentInput.value.in_imagemap === false) {
      // One toast per input while it is on screen — hammering a button
      // must not stack them.
      const tk = `${guid}#${key}`;
      const now = Date.now();
      if (lastMissingToast.key !== tk || now - lastMissingToast.at > TOAST_MS) {
        lastMissingToast = { key: tk, at: now };
        notify(`No area for ${res.token ? tokenLabel(res.token) : currentInput.value.sdl}`, "error");
      }
    }
    // Upgrade the image-map highlight to blue when SC has a binding — but only
    // while the input is still held (the resolve is async).
    if (activeInputs.value[guid]?.[key] !== undefined) {
      setActive(guid, key, res.actions.length ? "bound" : "none");
    }
  } catch {
    /* ignore transient resolve errors */
  }
}

interface Toast {
  id: number;
  message: string;
  type: "ok" | "error";
}

// How long a toast stays on screen.
const TOAST_MS = 4000;
const toasts = ref<Toast[]>([]);
// The last "not in image-map" toast for a live input, to avoid stacking.
let lastMissingToast = { key: "", at: 0 };
let toastSeq = 0;

// Show a transient toast that dismisses itself after a few seconds.
function notify(message: string, type: "ok" | "error" = "ok") {
  const id = ++toastSeq;
  toasts.value.push({ id, message, type });
  setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }, TOAST_MS);
}

// Take a LoadStatus over: bindings, whether the profile parsed, and its error
// for the Status panel.
function takeStatus(s: LoadStatus) {
  bindings.value = s.bindings;
  profileLoaded.value = s.loaded;
  error.value = s.loaded ? null : s.error;
}

// A backup was written back over actionmaps.xml — same follow-up as a reload.
async function onRestored(s: LoadStatus) {
  takeStatus(s);
  await loadClash();
  if (s.loaded) {
    notify("Restored", "ok");
  } else {
    notify(s.error ?? "Load failed", "error");
  }
}

// The backend is reloading the install's game data and profile in the
// background; show the progress and toast the outcome via `scdata-changed`.
async function awaitScLoad() {
  awaitingPathLoad = true;
  try {
    scStatus.value = await invoke<ScStatus>("get_sc_status");
  } catch (e) {
    awaitingPathLoad = false;
    notify(String(e), "error");
  }
}

// The install's game data (re)loaded: pick up actions, tokens, bindings.
async function onScDataChanged(s: LoadStatus) {
  scStatus.value = s.sc;
  actionMaps.value = await invoke<ActionMap[]>("get_actions");
  tokens.value = await invoke<Record<string, string>>("get_tokens");
  takeStatus(s);
  await loadClash();
  if (!awaitingPathLoad) return;
  awaitingPathLoad = false;
  if (s.sc.error) {
    notify(s.sc.error, "error");
  } else if (s.loaded) {
    notify(`Loaded ${s.bindings.length} joystick binding(s)`, "ok");
  } else {
    notify(s.error ?? "Load failed", "error");
  }
}

// Refresh = everything from scratch: devices, actionmaps.xml, Game.log, then
// the clash report on top of those.
async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    devices.value = await invoke<DeviceInfo[]>("list_joysticks");
    const s = await invoke<LoadStatus>("reload");
    takeStatus(s);
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
  await loadClash();
}

// Hot-plug (and startup): re-list the devices and redo the clash report on
// top of them; actionmaps.xml and Game.log are not re-read (SDL raises one
// devices-changed per device at startup, and the install did not change).
async function refreshDevices() {
  try {
    devices.value = await invoke<DeviceInfo[]>("list_joysticks");
  } catch (e) {
    error.value = String(e);
  }
  await loadClash();
}

function nameFor(guid: string): string {
  const d = devices.value.find((dev) => dev.sdl_guid === guid);
  return d?.sc_name ?? guid;
}

// SDL-side name of a live input, e.g. "button 5", "axis 2" or "hat 0 up".
function sdlInputName(kind: "button" | "hat" | "axis", index: number, direction: string | null): string {
  if (kind === "hat") return `hat ${index} ${direction ?? ""}`.trim();
  return `${kind} ${index}`;
}

// Live card colour: yellow when SC doesn't see the device (unseen or excluded
// — any SC token is meaningless then), blue when the input has SC bindings,
// grey otherwise.
function liveState(): "unseen" | "bound" | "none" {
  const c = currentInput.value;
  if (!c) return "none";
  if (isIgnored(c.sc_guid) || isUnseen(c.sc_guid)) return "unseen";
  return c.actions.length ? "bound" : "none";
}


let unlisten: UnlistenFn[] = [];

onMounted(async () => {
  unlisten.push(
    await listen<JoyInput>("joy-input", (e) => {
      const p = e.payload;
      // The raw log collects in every mode (shown in Devices).
      events.value.unshift({ ...p, at: Date.now() });
      if (events.value.length > MAX_EVENTS) events.value.pop();
      // The editor owns the input while an image-map is being edited.
      if (mode.value !== "live") return;
      trackActive(p);

      if (p.kind === "button" && p.pressed) {
        showBinding(p.guid, "button", p.index, null);
      } else if (p.kind === "hat" && p.direction !== "centered") {
        showBinding(p.guid, "hat", p.index, p.direction);
      } else if (p.kind === "axis" && axisDue(p.guid, p.index)) {
        showBinding(p.guid, "axis", p.index, null);
      }
    }),
  );
  unlisten.push(
    await listen("devices-changed", async () => {
      await refreshDevices();
      await reloadMaps();
    }),
  );
  // Registered before the initial fetch below, so a load finishing in
  // between is not missed.
  unlisten.push(await listen<LoadStatus>("scdata-changed", (e) => onScDataChanged(e.payload)));
  unlisten.push(
    await listen<ScStatus>("scdata-progress", (e) => {
      scStatus.value = e.payload;
    }),
  );
  // The backend reloads actionmaps.xml itself once the game data is in.
  await refreshDevices();

  try {
    scStatus.value = await invoke<ScStatus>("get_sc_status");
    actionMaps.value = await invoke<ActionMap[]>("get_actions");
    tokens.value = await invoke<Record<string, string>>("get_tokens");
    const cfg = await invoke<Config>("get_config");
    environments.value = cfg.environments;
    activeEnv.value = cfg.active_env;
    ignoredDevices.value = cfg.ignored_devices;
    mapChoices.value = cfg.imagemap_choices ?? {};
    bindings.value = await invoke<ResolvedBinding[]>("get_bindings");
  } catch (e) {
    error.value = String(e);
  }
  await reloadMaps();
});

onUnmounted(() => {
  unlisten.forEach((fn) => fn());
  unlisten = [];
  axisTimers.forEach((t) => clearTimeout(t));
  axisTimers.clear();
});
</script>

<template>
  <main class="app">
    <TopBar
      :mode="mode"
      :activeEnv="activeEnv"
      :scVersion="scStatus?.version?.label ?? ''"
      :loading="loading"
      @update:mode="setMode"
      @refresh="refresh"
      @settings="showSettings = !showSettings"
      @update:env="switchEnv"
    />

    <SettingsDialog
      v-if="showSettings"
      :environments="environments"
      :devices="devices"
      :ignored="ignoredDevices"
      @close="showSettings = false"
      @save="applySettings"
      @notify="notify"
    />

    <div v-if="mode === 'live'" class="content">
      <div class="top-row">
      <div class="devices-panel">
        <div class="panel-title">Connected devices</div>
        <div class="rail">
        <DeviceTile
          v-for="d in devices"
          :key="d.index"
          :device="d"
          :slot="slotFor(d.sc_product_guid)"
          :ignored="isIgnored(d.sc_product_guid)"
          :unseen="isUnseen(d.sc_product_guid)"
          :bindingCount="bindingCountFor(d.sc_product_guid)"
        />
        <div v-if="!devices.length" class="tile-none">None</div>
        </div>
      </div>
      <StatusPanel
        :report="clash"
        :loadError="error"
        :sc="scStatus"
        @apply="applyResort"
        @copy="copyResortCommands"
      />
      </div>

      <ImageStage
        :views="mapViews"
        :placeholders="stagePlaceholders"
        :height="stageHeight"
        :imgSrc="imgSrc"
        :activeFor="activeFor"
        @choose="setMapChoice"
      />
      <Splitter direction="row" @drag="dragStage" @end="saveLayout" @reset="resetStage" />

      <div class="deck-row" :style="{ gridTemplateColumns: `${liveWidth}px 16px minmax(0, 1fr)` }">
        <LiveCard
          :input="currentInput"
          :state="liveState()"
          :excluded="isIgnored(currentInput?.sc_guid ?? null)"
          :tokenLabel="tokenLabel"
          :categoryLabel="actionmapLabel"
        />
        <Splitter direction="col" @drag="dragLive" @end="saveLayout" @reset="resetLive" />
        <BindingsDeck
          :bindings="bindings"
          :currentToken="currentInput?.token ?? null"
          :tokenLabel="tokenLabel"
          :categoryLabel="actionmapLabel"
          :instanceOf="instanceOf"
          :isClash="bindingClash"
          :isConnected="isConnected"
          :isMissing="missingInMap"
          :isPinned="isPinned"
          @pin="togglePin"
        />
      </div>
    </div>

    <ToolsView
      v-else-if="mode === 'tools'"
      :bindings="bindings"
      :actionMaps="actionMaps"
      :hasCurrent="profileLoaded"
      @notify="notify"
      @restored="onRestored"
    />

    <ImageMapEditor
      v-else-if="mode === 'devices'"
      ref="editor"
      :devices="devices"
      :events="events"
      @notify="notify"
      @saved="onMapsSaved"
      @clear-log="events = []"
    />

    <Toasts :toasts="toasts" />
  </main>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  background: var(--bg-base);
}

/* No column gap: the row splitter between stage and deck carries the 16px. */
.content {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 12px 16px 16px;
  min-height: 0;
}

.top-row {
  display: flex;
  align-items: stretch;
  gap: 16px;
  margin-bottom: 16px;
}

.devices-panel {
  flex: 1;
  min-width: 0;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  padding: 12px 16px 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.rail {
  display: flex;
  align-items: flex-start;
  flex-wrap: wrap;
  gap: 12px;
}

/* Empty rail: one dimmed tile in the device tile's footprint. */
.tile-none {
  width: 340px;
  height: 84px;
  box-sizing: border-box;
  border-radius: var(--radius-panel);
  border: 1px dashed var(--border);
  background: var(--bg-surface-2);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 15px;
  font-weight: 600;
}

.deck-row {
  flex: 1;
  display: grid;
  grid-template-columns: 400px 16px minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr);
  min-height: 0;
}
</style>
