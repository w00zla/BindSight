<script setup lang="ts">
import { ref, shallowRef, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import ImageMapEditor from "./components/ImageMapEditor.vue";
import TopBar from "./components/TopBar.vue";
import DeviceTile from "./components/DeviceTile.vue";
import ScrollRail from "./components/ScrollRail.vue";
import StatusPanel from "./components/StatusPanel.vue";
import ImageStage from "./components/ImageStage.vue";
import LastInputCard from "./components/LastInputCard.vue";
import BindingsDeck from "./components/BindingsDeck.vue";
import BindingsView from "./components/BindingsView.vue";
import Toasts from "./components/Toasts.vue";
import WindowEdges from "./components/WindowEdges.vue";
import { deviceKey, deviceName } from "./devices";
import Splitter from "./components/Splitter.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import StartupTile from "./components/StartupTile.vue";
import AppFooter from "./components/AppFooter.vue";
import { setDebugLogging } from "./logging";
import VersionDialog from "./components/VersionDialog.vue";
import type { Updater } from "./update";
import type { LogOnlyJoystick } from "./types";
import type {
  DeviceKind,
  ActionMap,
  BoundAction,
  ClashReport,
  HidOnlyDevice,
  WineKey,
  DiDevice,
  Config,
  CurrentInput,
  DeviceInfo,
  DevicesChanged,
  Environment,
  ImageMapView,
  JoyInput,
  LiveState,
  LoadStatus,
  LoggedInput,
  Mode,
  OverlayPosition,
  ResolvedBinding,
  ScStatus,
  SlotStatus,
  SystemInfo,
  ToastType,
  UpdateChannel,
} from "./types";
import {
  inputKey,
  inputKeyForToken,
  inputKeysForToken,
  sameHardware,
  shapeImageFiles,
  type ImageMap,
  type ImageMapSummary,
  type OverlayTarget,
} from "./imagemap";
import { startKeyboardCapture, startMouseCapture } from "./keyboard";

const MAX_EVENTS = 500;

// The image-map shown while the user has not chosen one. Hard-coded rules
// for the bundled keyboard and gamepad maps, nothing in the image-maps
// themselves: pads match wildcard patterns against the controller name,
// keyboards the OS keyboard layout (xkb code). First fitting rule wins,
// else the default per kind, else the first map for the hardware id.
// Joysticks match by hardware id only.
const MAP_US = "4b7a2c1e-0001-4000-8000-000000000001";
const MAP_DE = "4b7a2c1e-0001-4000-8000-000000000002";
const MAP_XBOX = "4b7a2c1e-0001-4000-8000-000000000003";
const MAP_PS = "4b7a2c1e-0001-4000-8000-000000000004";

const BUNDLED_RULES: { id: string; layouts?: string[]; names?: string[] }[] = [
  { id: MAP_US, layouts: ["us", "gb", "au", "ca"] },
  { id: MAP_DE, layouts: ["de", "at", "ch"] },
  { id: MAP_XBOX, names: ["*xbox*", "*x-box*", "*microsoft*", "*8bitdo*"] },
  { id: MAP_PS, names: ["*playstation*", "*dualshock*", "*dualsense*", "*sony*", "*ps3*", "*ps4*", "*ps5*"] },
];

const DEFAULT_MAPS: Partial<Record<DeviceKind, string>> = { keyboard: MAP_US, gamepad: MAP_XBOX };

// OS keyboard layout (xkb code such as `de`), null when unknown.
const keyboardLayout = ref<string | null>(null);
// One line of environment facts for the Device Info dumps.
const systemInfo = ref<SystemInfo | null>(null);
// The in-app updater (update.ts; installs only where `SystemInfo.updater`,
// elsewhere it links the project page); its footer mark is a component it
// brings along.
const updater = shallowRef<Updater | null>(null);
// The App Update dialog (footer click; opens itself when the startup check
// finds an update).
const showVersion = ref(false);
const systemLine = computed(() => {
  const s = systemInfo.value;
  const env = s ? `BindSight ${s.app_version} · ${s.os} ${s.arch} · tauri ${s.tauri} · webview ${s.webview} · SDL ${s.sdl}` : "BindSight";
  return `${env} · keyboard layout ${keyboardLayout.value ?? "unknown"}`;
});

// Case-insensitive glob: `*` any run, `?` one char.
function wildcard(pattern: string, text: string): boolean {
  const re = new RegExp(`^${pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")}$`, "i");
  return re.test(text);
}

function ruleFits(r: (typeof BUNDLED_RULES)[number], d: DeviceInfo): boolean {
  if (d.kind === "gamepad") return !!r.names?.some((p) => wildcard(p, deviceName(d)));
  if (d.kind === "keyboard") {
    const l = keyboardLayout.value?.toLowerCase();
    return !!l && !!r.layouts?.some((x) => x === l);
  }
  return false;
}

function defaultMapId(d: DeviceInfo, list: ImageMapSummary[]): string | undefined {
  const hit = BUNDLED_RULES.find((r) => list.some((s) => s.id === r.id) && ruleFits(r, d));
  return hit?.id ?? DEFAULT_MAPS[d.kind];
}

const devices = ref<DeviceInfo[]>([]);
const events = ref<LoggedInput[]>([]);
const actionMaps = ref<ActionMap[]>([]);
// SC environments (Settings) and the one being read (top-bar chip).
const environments = ref<Record<string, Environment>>({});
const activeEnv = ref("LIVE");
const bindings = ref<ResolvedBinding[]>([]);
const tokens = ref<Record<string, string>>({});
const currentInput = ref<CurrentInput | null>(null);
// The physical input behind `currentInput`, to tell whether it is still held
// (buttons / keys while down, hats until centred, axes for a pulse — the
// image stage's `activeInputs` rules).
const currentKey = ref<{ guid: string; key: string } | null>(null);
const currentHeld = computed(() => {
  const c = currentKey.value;
  return !!c && activeInputs.value[c.guid]?.[c.key] !== undefined;
});
// The last captured key event, handed to the editor (only the webview sees keys).
const clash = ref<ClashReport | null>(null);
// The HID interfaces Wine registers (Linux): the Device List's "wine" row.
const wineKeys = ref<WineKey[]>([]);
// DirectInput's game controllers (Windows): the Device List's "dinput" row.
const dinputDevices = ref<DiDevice[]>([]);
// Joystick-class HID devices SDL does not list: the Device List's tail.
const hidOnly = ref<HidOnlyDevice[]>([]);
// The install's version and game-data load state (updated via `scdata-changed`).
const scStatus = ref<ScStatus | null>(null);
// The first game-data load after start is still running: the startup tile
// covers every mode until it ends, whatever its outcome.
const starting = ref(true);
// The startup tile stays up at least this long, so it does not just flash by.
const STARTUP_MIN_MS = 3000;
const startedAt = Date.now();

// The startup image-map load is done (the tile waits for it, so the maps do
// not pop in after the stage is already showing); a game-data load that
// finishes earlier is remembered and ends the tile once the maps are in.
let mapsReady = false;
let startupPending = false;

// End the startup tile, but not before it has been up STARTUP_MIN_MS and the
// image-maps are loaded.
function endStartup() {
  if (!mapsReady) {
    startupPending = true;
    return;
  }
  const left = STARTUP_MIN_MS - (Date.now() - startedAt);
  if (left > 0) setTimeout(() => (starting.value = false), left);
  else starting.value = false;
}
// Set while a base-path change is being loaded, so its result gets a toast.
let awaitingPathLoad = false;
// SC Product GUIDs the user marked "SC doesn't see this device" (persisted per OS).
const error = ref<string | null>(null);
// The live actionmaps.xml is parsed (from the last LoadStatus).
const currentLoaded = ref(false);
const loading = ref(false);
const showSettings = ref(false);
// Back up actionmaps.xml before BindSight overwrites it (Settings).
const autoBackup = ref(true);
// Write DEBUG records to bindsight.log (Settings).
const debugLogging = ref(false);
// Check for an update at startup (Settings); the dialog's manual check is
// always available.
const updateCheck = ref(true);
// Which release feed the updater reads (Settings).
const updateChannel = ref<UpdateChannel>("stable");
// The input-preview overlay's size in px and where it appears (Settings).
const overlaySize = ref(340);
const overlayPosition = ref<OverlayPosition>("mouse-offset");

// --- image-maps --------------------------------------------------------

// How long an axis stays highlighted after its last event (axes never rest).
const AXIS_PULSE_MS = 400;

const mode = ref<Mode>("monitor");
const mapSummaries = ref<ImageMapSummary[]>([]);
// Map id -> full image-map, and `<map id>/<file>` -> image data URL.
const loadedMaps = ref<Record<string, ImageMap>>({});
const mapImages = ref<Record<string, string>>({});
// Lowercase hardware id -> chosen map id (from config.json).
const mapChoices = ref<Record<string, string>>({});
// SDL GUID -> the input keys currently active (held, or pulsing for an axis).
const activeInputs = ref<Record<string, Record<string, true>>>({});
const axisTimers = new Map<string, number>();

function mapsFor(hardwareId: string | null): ImageMapSummary[] {
  return mapSummaries.value.filter((s) => sameHardware(s.hardware_id, hardwareId));
}

// The user's pick for this device, else the bundled default for a pad, else
// the single/first matching map.
function chosenMapId(d: DeviceInfo | null | undefined): string | null {
  const hardwareId = d?.hardware_id ?? null;
  const list = mapsFor(hardwareId);
  if (!list.length) return null;
  const pick = hardwareId ? mapChoices.value[hardwareId.toLowerCase()] : undefined;
  const chosen = list.find((s) => s.id === pick);
  if (chosen) return chosen.id;
  const fallback = d ? defaultMapId(d, list) : undefined;
  return list.find((s) => s.id === fallback)?.id ?? list[0].id;
}

function imgSrc(id: string, file: string): string {
  return mapImages.value[`${id}/${file}`] ?? "";
}

// Devices the stage shows at all: a further pad has no slot and a joystick
// the game does not see has no jsN, so neither has bindings or a tile; the
// user can hide any other device by hand.
function onStage(d: DeviceInfo): boolean {
  if (d.kind === "gamepad" && d.gamepad_slot === null) return false;
  if (deviceUnseen(d)) return false;
  return !isStageHidden(d);
}

const mapViews = computed<ImageMapView[]>(() =>
  orderedDevices.value.flatMap((d) => {
    if (!onStage(d)) return [];
    const id = chosenMapId(d);
    const p = id ? loadedMaps.value[id] : null;
    return p ? [{ device: d, map: p }] : [];
  }),
);

// Devices the user took off the stage, by lowercase hardware id.
const HIDDEN_KEY = "bindsight.stage.hidden";
const stageHidden = ref<string[]>([]);
try {
  const parsed = JSON.parse(localStorage.getItem(HIDDEN_KEY) ?? "[]") as unknown;
  if (Array.isArray(parsed)) stageHidden.value = parsed.filter((x): x is string => typeof x === "string");
} catch {
  /* nothing hidden */
}

function isStageHidden(d: DeviceInfo): boolean {
  return !!d.hardware_id && stageHidden.value.includes(d.hardware_id.toLowerCase());
}

function toggleStageHidden(d: DeviceInfo) {
  if (!d.hardware_id) return;
  const id = d.hardware_id.toLowerCase();
  stageHidden.value = stageHidden.value.includes(id)
    ? stageHidden.value.filter((x) => x !== id)
    : [...stageHidden.value, id];
  try {
    localStorage.setItem(HIDDEN_KEY, JSON.stringify(stageHidden.value));
  } catch {
    /* the choice simply does not persist */
  }
}

// Resizable layout: stage height and input-card width, remembered locally.
const LAYOUT_KEY = "bindsight.layout";
const STAGE_H = { def: 440, min: 120 };
const INPUT_CARD_W = { def: 400, min: 280 };
// The deck never gets narrower / lower than this; stage and input card take
// the rest.
const DECK_MIN = 320;
const DECK_MIN_H = 160;
const stageHeight = ref(STAGE_H.def);
const inputCardWidth = ref(INPUT_CARD_W.def);
try {
  // `liveWidth` is the persisted property name from before the input card
  // was renamed; kept as-is so existing localStorage records still apply.
  const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}") as { stageHeight?: number; liveWidth?: number };
  if (typeof saved.stageHeight === "number") stageHeight.value = saved.stageHeight;
  if (typeof saved.liveWidth === "number") inputCardWidth.value = saved.liveWidth;
} catch {
  /* defaults */
}
// Stage height / input-card width when a drag began, plus the deck's height
// then: stage and deck share the column, so the stage may grow by what the
// deck has above DECK_MIN_H.
let layoutStart: { stageHeight: number; inputCardWidth: number; deckHeight: number } | null = null;
const clamp = (v: number, r: { min: number; max: number }) => Math.min(Math.max(v, r.min), r.max);
const deckRow = ref<HTMLElement | null>(null);
function startLayout() {
  layoutStart ??= { stageHeight: stageHeight.value, inputCardWidth: inputCardWidth.value, deckHeight: deckRow.value?.clientHeight ?? Infinity };
  return layoutStart;
}
function dragStage(delta: number) {
  const start = startLayout();
  const max = start.stageHeight + start.deckHeight - DECK_MIN_H;
  stageHeight.value = clamp(start.stageHeight + delta, { min: STAGE_H.min, max: Math.max(max, STAGE_H.min) });
}
// The input card grows until the deck is down to DECK_MIN in the current row.
function dragInputCard(delta: number) {
  const start = startLayout();
  const max = (deckRow.value?.clientWidth ?? Infinity) - 16 - DECK_MIN;
  inputCardWidth.value = clamp(start.inputCardWidth + delta, { min: INPUT_CARD_W.min, max: Math.max(max, INPUT_CARD_W.min) });
}
function saveLayout() {
  layoutStart = null;
  try {
    // `liveWidth` kept as the persisted property name on purpose (see above).
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ stageHeight: stageHeight.value, liveWidth: inputCardWidth.value }));
  } catch {
    /* ignore */
  }
}
function resetStage() {
  stageHeight.value = STAGE_H.def;
  saveLayout();
}
function resetInputCard() {
  inputCardWidth.value = INPUT_CARD_W.def;
  saveLayout();
}

// Connected devices on the stage without an image-map (placeholder tiles).
const stagePlaceholders = computed<DeviceInfo[]>(() =>
  orderedDevices.value.filter(
    (d) => onStage(d) && !mapViews.value.some((v) => v.device.index === d.index),
  ),
);

// Map ids and `<map id>/<file>` keys whose load is in flight: two callers at
// once (the startup step and a devices-changed right behind it) must not
// fetch the same map twice.
const mapLoading = new Set<string>();

async function loadMapImage(id: string, file: string) {
  const key = `${id}/${file}`;
  if (mapImages.value[key] || mapLoading.has(key)) return;
  mapLoading.add(key);
  try {
    mapImages.value[key] = await invoke<string>("read_imagemap_image", { id, file });
  } catch {
    /* a missing image just stays blank */
  } finally {
    mapLoading.delete(key);
  }
}

async function loadChosenMaps() {
  for (const d of devices.value) {
    const id = chosenMapId(d);
    if (!id || loadedMaps.value[id] || mapLoading.has(id)) continue;
    mapLoading.add(id);
    try {
      const p = await invoke<ImageMap>("get_imagemap", { id });
      loadedMaps.value[id] = p;
      await loadMapImage(p.id, p.image.file);
      for (const f of shapeImageFiles(p)) await loadMapImage(p.id, f);
    } catch {
      /* skip a map that will not load */
    } finally {
      mapLoading.delete(id);
    }
  }
}

async function reloadMaps() {
  try {
    mapSummaries.value = await invoke<ImageMapSummary[]>("list_imagemaps");
  } catch (e) {
    console.error("image-map list failed", e);
    error.value = String(e);
    return;
  }
  await loadChosenMaps();
}

// A save in the editor can change the images and shapes — drop the caches.
async function onMapsSaved() {
  loadedMaps.value = {};
  mapImages.value = {};
  await reloadMaps();
}

// The editor and the Bindings mode guard their unsaved changes; leaving
// Devices or Bindings can be refused.
const editor = ref<InstanceType<typeof ImageMapEditor> | null>(null);
const bindingsView = ref<InstanceType<typeof BindingsView> | null>(null);

// Pending rebinds settled (saved, discarded, or none)? Anything that re-reads
// or swaps the bindings file underneath them asks first.
async function bindingsSettled(): Promise<boolean> {
  return (await bindingsView.value?.requestLeave()) !== false;
}

// Both modes' unsaved changes settled — before the window goes away.
async function allSettled(): Promise<boolean> {
  return (await editor.value?.requestLeave()) !== false && (await bindingsSettled());
}

async function setMode(m: Mode) {
  // The mode buttons do nothing while the startup tile is up (no disabled
  // look on purpose).
  if (starting.value) return;
  if (mode.value === "devices" && m !== "devices") {
    if ((await editor.value?.requestLeave()) === false) return;
  }
  if (mode.value === "bindings" && m !== "bindings") {
    if (!(await bindingsSettled())) return;
  }
  mode.value = m;
  // Inputs released while the editor was open were never seen here.
  activeInputs.value = {};
  if (m === "monitor") await reloadMaps();
  if (m === "devices") await loadDeviceInfo();
}

async function setMapChoice(hardwareId: string | null, id: string) {
  if (!hardwareId) return;
  try {
    const cfg = await invoke<{ imagemap_choices: Record<string, string> }>("set_imagemap_choice", {
      hardwareId,
      imagemapId: id || null,
    });
    mapChoices.value = cfg.imagemap_choices;
    await loadChosenMaps();
  } catch (e) {
    notify(String(e), "error");
  }
}

function setActive(guid: string, key: string) {
  const cur = activeInputs.value[guid] ?? {};
  activeInputs.value = { ...activeInputs.value, [guid]: { ...cur, [key]: true } };
}

function clearActive(guid: string, drop: (key: string) => boolean) {
  const cur = activeInputs.value[guid];
  if (!cur) return;
  const next: Record<string, true> = {};
  for (const k of Object.keys(cur)) if (!drop(k)) next[k] = true;
  activeInputs.value = { ...activeInputs.value, [guid]: next };
}

// A binding clicked in the list stays selected — its row and its input on
// the image-map image — until the next live input takes over; the Last
// Input card shows it meanwhile, as if its input had arrived.
const flash = ref<{ b: ResolvedBinding; target: PinTarget | null } | null>(null);

// The lit inputs of a device: everything active plus a selected binding's.
function activeFor(guid: string): Set<string> {
  const s = new Set(Object.keys(activeInputs.value[guid] ?? {}));
  const t = flash.value?.target;
  if (t?.guid === guid) for (const k of t.keys) s.add(k);
  return s;
}

// Does the map shown for this device (by SDL GUID) have an area for the
// input? `null` when the device has no image-map.
function inMap(sdlGuid: string, key: string): boolean | null {
  const id = chosenMapId(deviceOf(sdlGuid));
  const p = id ? loadedMaps.value[id] : null;
  if (!p) return null;
  return p.shapes.some((a) => a.input === key);
}

function deviceOf(sdlGuid: string): DeviceInfo | undefined {
  return devices.value.find((d) => d.sdl_guid === sdlGuid);
}

// The device an event came from: SDL GUID plus instance id — two sticks of
// the same type share the GUID, the instance id tells them apart (the
// keyboard is GUID "keyboard", instance 0).
function deviceOfEvent(p: JoyInput): DeviceInfo | undefined {
  return devices.value.find((d) => d.sdl_guid === p.guid && d.sdl_instance_id === p.instance_id);
}

// The connected device a binding belongs to: joysticks by GUID, keyboard and
// gamepad by kind (SC has exactly one of each).
// A joystick binding sits on the device SC ranks at its jsN (the game's own
// order, never the saved <options> slot); without that order, on none.
function deviceForBinding(b: ResolvedBinding): DeviceInfo | undefined {
  const d = deviceForKind(b);
  return d && !deviceUnseen(d) ? d : undefined;
}

function deviceForKind(b: ResolvedBinding): DeviceInfo | undefined {
  if (b.device_kind === "keyboard") return devices.value.find((d) => d.kind === "keyboard");
  if (b.device_kind === "gamepad") return devices.value.find((d) => d.kind === "gamepad" && d.gamepad_slot !== null);
  return joystickOnSlot(b.instance);
}

// The image-map preview target for a device slot (Bindings tab overlay): the
// resolved device, its chosen map, and the image resolvers — or null when the
// slot has no connected device or no (loaded) map. Mirrors `deviceForKind`
// but keyed by kind + instance, as the Bindings List columns are.
function overlayFor(kind: DeviceKind, instance: number): OverlayTarget | null {
  let device: DeviceInfo | undefined;
  if (kind === "keyboard") device = devices.value.find((d) => d.kind === "keyboard");
  else if (kind === "gamepad") device = devices.value.find((d) => d.kind === "gamepad" && d.gamepad_slot !== null);
  else {
    device = joystickOnSlot(instance);
  }
  if (!device) return null;
  const id = chosenMapId(device);
  const map = id ? loadedMaps.value[id] : null;
  if (!id || !map) return null;
  return { device, map, src: imgSrc(id, map.image.file), imageUrl: (file: string) => imgSrc(id, file) };
}

// Where a flashed binding lights up: the device and its input's key, plus
// every key of the map that gets lit (a combo's modifiers too, when the
// map has shapes for them).
interface PinTarget {
  guid: string;
  key: string;
  keys: string[];
}

// Can a binding in the list be lit on an image-map image? Needs a connected
// device with an image-map and a mappable token.
function pinTarget(b: ResolvedBinding): PinTarget | null {
  const r = resolvePin(b);
  return "reason" in r ? null : r;
}

// Where a binding would light up, or why it cannot.
function resolvePin(b: ResolvedBinding): PinTarget | { reason: string } {
  const d = deviceForBinding(b);
  if (!d) return { reason: `${b.device ?? "Device"} not connected` };
  const key = inputKeyForToken(b.token, d);
  if (!key) return { reason: `${tokenLabel(b.token)}: no such axis on ${deviceName(d)}${d.axes_error ? ` (${d.axes_error})` : ""}` };
  const has = inMap(d.sdl_guid, key);
  if (has === null) return { reason: `${deviceName(d)} has no image-map` };
  if (!has) return { reason: `No shape for ${tokenLabel(b.token)}` };
  const keys = inputKeysForToken(b.token, d).filter((k) => inMap(d.sdl_guid, k));
  return { guid: d.sdl_guid, key, keys };
}

// The device has an image-map, but no area for this binding's input.
function missingInMap(b: ResolvedBinding): boolean {
  const t = pinTarget(b);
  return !!t && inMap(t.guid, t.key) === false;
}

function isFlashed(b: ResolvedBinding): boolean {
  const f = flash.value?.b;
  return !!f && f.token === b.token && f.actionmap === b.actionmap && f.action === b.action;
}

// Flash the row either way; say why nothing lights up on the image when it
// cannot (a missing area is already tagged on the row itself).
function flashBinding(b: ResolvedBinding) {
  const r = resolvePin(b);
  if ("reason" in r && !missingInMap(b)) notify(r.reason, "warn");
  const target = "reason" in r ? null : r;
  flash.value = { b, target };
  const d = deviceForBinding(b);
  const key = d ? inputKeyForToken(b.token, d) : null;
  currentKey.value = target ? { guid: target.guid, key: target.key } : null;
  currentInput.value = {
    device: d ? deviceName(d) : (b.device ?? "—"),
    kind: b.device_kind,
    sc_guid: b.device_guid,
    token: b.token,
    // The image-map key reads like `sdlInputName` does ("button 2").
    sdl: key ? key.split(":").join(" ") : "",
    actions: connectedBindings.value
      .filter((x) => x.token === b.token)
      .map((x) => ({ actionmap: x.actionmap, action: x.action, label: x.label, is_default: x.is_default, device_kind: x.device_kind })),
    in_imagemap: target ? true : d && key ? inMap(d.sdl_guid, key) : null,
  };
}

// Buttons and keys stay lit while held, hats until centered, axes pulse.
function trackActive(p: JoyInput) {
  if (p.kind === "button" || p.kind === "padbutton" || p.kind === "key") {
    const key = p.kind === "button" ? `button:${p.index}` : `${p.kind === "key" ? "key" : "pad"}:${p.name}`;
    if (p.pressed) setActive(p.guid, key);
    else clearActive(p.guid, (k) => k === key);
    return;
  }
  if (p.kind === "hat") {
    clearActive(p.guid, (k) => k.startsWith(`hat:${p.index}:`));
    if (p.direction !== "centered") setActive(p.guid, `hat:${p.index}:${p.direction}`);
    return;
  }
  const key = p.kind === "axis" ? `axis:${p.index}` : `pad:${p.name}`;
  setActive(p.guid, key);
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

// The buttons the backend derives from each pad stick axis (`thumbl_left`
// past half travel of `thumblx`); a trigger arrives only as its derived
// button (`triggerl_btn`), never as an axis. None of them is a modifier —
// SC's pad combos (`shoulderl+thumbl_left`) start with a real button — and
// while a stick direction is held its axis stays off the Last Input card,
// or the axis and the button would take turns there.
const PAD_DERIVED: Record<string, string[]> = {
  thumblx: ["thumbl_left", "thumbl_right"],
  thumbly: ["thumbl_up", "thumbl_down"],
  thumbrx: ["thumbr_left", "thumbr_right"],
  thumbry: ["thumbr_up", "thumbr_down"],
};
const DERIVED_NAMES = new Set([...Object.values(PAD_DERIVED).flat(), "triggerl_btn", "triggerr_btn", "triggerl_r_btn"]);

// Currently held key/pad-button names per device GUID, most recent first: the
// modifier candidates for the next press (`kb1_lalt+x`). Derived pad buttons
// are kept apart, see `PAD_DERIVED`.
const heldNames = new Map<string, string[]>();
const heldDerived = new Map<string, Set<string>>();

function trackHeld(p: JoyInput) {
  if (p.kind !== "key" && p.kind !== "padbutton") return;
  if (p.kind === "padbutton" && DERIVED_NAMES.has(p.name)) {
    const set = heldDerived.get(p.guid) ?? new Set<string>();
    if (p.pressed) set.add(p.name);
    else set.delete(p.name);
    heldDerived.set(p.guid, set);
    return;
  }
  const rest = (heldNames.get(p.guid) ?? []).filter((n) => n !== p.name);
  heldNames.set(p.guid, p.pressed ? [p.name, ...rest] : rest);
}

// Is a button derived from this pad axis held right now?
function derivedHeld(guid: string, axis: string): boolean {
  const set = heldDerived.get(guid);
  return !!set && (PAD_DERIVED[axis] ?? []).some((n) => set.has(n));
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

// Is the device recorded for this binding currently connected?
function isConnected(b: ResolvedBinding): boolean {
  return !!deviceForBinding(b);
}

// The Monitor deck lists only the bindings of connected devices.
const connectedBindings = computed<ResolvedBinding[]>(() => bindings.value.filter(isConnected));

// SC instance number from a token, e.g. "js2_button9" -> "2".
function instanceOf(token: string): string {
  return token.match(/^js(\d+)_/)?.[1] ?? "?";
}

// SC's name for the device a binding sits on: js1, js2, kb1, gp1.
function deviceLabel(b: ResolvedBinding): string {
  if (b.device_kind === "keyboard") return "kb1";
  if (b.device_kind === "gamepad") return "gp1";
  return `js${b.instance}`;
}

// Number of bindings assigned to a device.
function bindingCountFor(d: DeviceInfo): number {
  if (d.kind === "keyboard") return bindings.value.filter((b) => b.device_kind === "keyboard").length;
  if (d.kind === "gamepad") {
    return d.gamepad_slot === null ? 0 : bindings.value.filter((b) => b.device_kind === "gamepad").length;
  }
  const n = slotOf(d)?.effective_instance;
  return n === undefined ? 0 : bindings.value.filter((b) => b.device_kind === "joystick" && b.instance === n).length;
}

// A joystick SDL sees that is not in the game's order. The keyboard and the
// pad take no slot and are never "unseen".
function deviceUnseen(d: DeviceInfo): boolean {
  return d.kind === "joystick" && isUnseen(d.sc_product_guid);
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

// The slot of an SDL device, by its instance id: identical devices share
// the GUID, not the slot.
function slotOf(d: DeviceInfo | undefined): SlotStatus | null {
  if (d?.kind !== "joystick") return null;
  return clash.value?.connected.find((s) => s.sdl_instance_id === d.sdl_instance_id) ?? null;
}

// The SDL joystick on a jsN slot.
function joystickOnSlot(instance: number): DeviceInfo | undefined {
  const id = slotByInstance.value.get(instance)?.sdl_instance_id;
  return id == null ? undefined : devices.value.find((d) => d.kind === "joystick" && d.sdl_instance_id === id);
}

// Connected-slot status by the jsN SC assigns.
const slotByInstance = computed<Map<number, SlotStatus>>(
  () => new Map((clash.value?.connected ?? []).map((s) => [s.effective_instance, s])),
);

// The game's joystick order is unknown: no joystick has a jsN, so none of
// its input resolves. The keyboard and the pad are unaffected.
const noOrder = computed(() => !!clash.value?.order_error);

// GUIDs of joysticks SDL sees that are not in the game's order.
const unseenGuids = computed<Set<string>>(
  () => new Set((clash.value?.unseen ?? []).map((u) => u.sc_product_guid).filter((g): g is string => !!g)),
);

function isUnseen(guid: string | null): boolean {
  return !!guid && unseenGuids.value.has(guid);
}

// Display order everywhere a device list is shown: the keyboard, the slotted
// pad, the joysticks SC sees, then everything SC does not (unseen, pads
// without a slot) — SDL order within a group.
function deviceRank(d: DeviceInfo): number {
  if (d.kind === "keyboard") return 0;
  if (d.kind === "gamepad" && d.gamepad_slot === null) return 3;
  if (deviceUnseen(d)) return 3;
  return d.kind === "gamepad" ? 1 : 2;
}
const orderedDevices = computed<DeviceInfo[]>(() =>
  [...devices.value].sort((a, b) => deviceRank(a) - deviceRank(b) || a.index - b.index),
);
// The Monitor's device rail: a joystick the game does not see stays out of
// it (only the Device List says so).
const monitorDevices = computed<DeviceInfo[]>(() =>
  orderedDevices.value.filter((d) => !deviceUnseen(d)),
);
// Joysticks the game lists (its order) that SDL does not: no input can
// reach the app from them (e.g. a device on SDL's joystick blacklist that
// Wine takes through hidraw). The Monitor shows them as tiles without input,
// after the SDL devices, in the game's order.
const logOnlyJoysticks = computed<LogOnlyJoystick[]>(() => {
  const sdl = new Set(
    devices.value
      .filter((d) => d.kind === "joystick")
      .map((d) => d.sc_product_guid?.toLowerCase())
      .filter((g): g is string => !!g),
  );
  return (clash.value?.connected ?? [])
    .filter((s): s is typeof s & { sc_product_guid: string } => !!s.sc_product_guid && !sdl.has(s.sc_product_guid.toLowerCase()))
    .map((s) => ({
      kind: "joystick",
      log_only: true,
      sc_name: s.name,
      sdl_name: s.name ?? "?",
      sc_product_guid: s.sc_product_guid,
      hardware_id: null,
      gamepad_slot: null,
      controller_name: null,
    }));
});
const logOnlyInstances = computed<Set<number>>(
  () => new Set(logOnlyJoysticks.value.map((j) => slotFor(j.sc_product_guid)?.effective_instance).filter((n): n is number => n !== undefined)),
);
function isLogOnly(instance: number): boolean {
  return logOnlyInstances.value.has(instance);
}
// Settings dialog Save: apply the environments and the switches; the
// backend reloads when the active environment changed.
async function applySettings(s: {
  environments: Record<string, Environment>;
  autoBackup: boolean;
  debugLogging: boolean;
  updateCheck: boolean;
  updateChannel: UpdateChannel;
  overlaySize: number;
  overlayPosition: OverlayPosition;
}) {
  // A changed active environment means another bindings file: settle the
  // pending rebinds first.
  const envChanged =
    JSON.stringify(s.environments[activeEnv.value] ?? null) !== JSON.stringify(environments.value[activeEnv.value] ?? null);
  if (envChanged && !(await bindingsSettled())) return;
  showSettings.value = false;
  try {
    await invoke("set_auto_backup", { enabled: s.autoBackup });
    autoBackup.value = s.autoBackup;
    await invoke("set_debug_logging", { enabled: s.debugLogging });
    debugLogging.value = s.debugLogging;
    setDebugLogging(s.debugLogging);
    await invoke("set_update_check", { enabled: s.updateCheck });
    updateCheck.value = s.updateCheck;
    await invoke("set_update_channel", { channel: s.updateChannel });
    updateChannel.value = s.updateChannel;
    await invoke("set_overlay_size", { size: s.overlaySize });
    overlaySize.value = s.overlaySize;
    await invoke("set_overlay_position", { position: s.overlayPosition });
    overlayPosition.value = s.overlayPosition;
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

// Top-bar chip: switch the environment the app reads. Every view is keyed by
// `activeEnv`, so setting it below forces a full remount (fresh profiles,
// backups, filters, pending edits) — after the mounted view's unsaved changes
// are settled.
async function switchEnv(slug: string) {
  if (slug === activeEnv.value || !(await allSettled())) return;
  try {
    const changed = await invoke<boolean>("set_active_env", { slug });
    activeEnv.value = slug;
    notify(`Switched to ${slug}`, "ok");
    if (changed) await awaitScLoad();
  } catch (e) {
    notify(String(e), "error");
  }
}

// The device-order clash, seen from a binding: the joystick its jsN lands on
// (SC's order) is saved under another slot in the file, or not at all. The
// text names that slot for the deck's tooltip; undefined = no clash here.
function bindingClash(token: string): string | undefined {
  if (!clash.value?.has_clash) return undefined;
  const slot = slotByInstance.value.get(Number(instanceOf(token)));
  if (!slot || slot.stored_instance === slot.effective_instance) return undefined;
  return slot.stored_instance === null ? "joystick not saved" : `joystick saved as js${slot.stored_instance}`;
}

// `announce`: the report follows something that happened by itself (a
// hot-plug, a new game log), so a clash appearing or going away is toasted.
// Refreshes and writes stay quiet: the Status panel shows the outcome.
async function loadClash(announce = false) {
  const before = clash.value?.has_clash ?? false;
  try {
    clash.value = await invoke<ClashReport>("get_clash_report");
  } catch (e) {
    console.warn("clash report failed", e);
    clash.value = null;
  }
  const after = clash.value?.has_clash ?? false;
  if (announce && before !== after) {
    if (after) notify("Joystick order clash detected", "warn");
    else notify("Joystick order clash resolved", "ok");
  }
}

// Hot-plug toasts, one per device.
function announceDevices(change: DevicesChanged) {
  for (const d of change.added) notify(`${deviceName(d)} connected`, "ok");
  for (const d of change.removed) notify(`${deviceName(d)} disconnected`, "warn");
}

// The game log was read: at start-up / after an environment change quietly,
// or because the game started and wrote a new one with the joysticks it
// enumerated. Either way redo the report; only a clash appearing or going
// away is toasted (a start that changes nothing is not news).
async function onGameLogChanged(started: boolean) {
  await loadClash(started);
}

// The backend saw the bindings file change (not by this app) and re-read it:
// same follow-up as a reload, with a word about it.
async function onBindingsChanged(s: LoadStatus) {
  takeStatus(s);
  await loadClash();
  if (s.loaded) {
    notify("Game bindings changed, reloaded", "hint");
  } else {
    notify(s.error ?? "Game bindings changed, load failed", "error");
  }
}

// The write went through the backend's one road (`replace_live_file`), which
// backs the file up first unless the user switched that off in Settings.
function withBackup(message: string): string {
  return autoBackup.value ? `${message} (backup created)` : `${message} without backup`;
}

// Put the pp_resortdevices command line from the Fix via console dialog on the
// clipboard.
async function copyResortCommands(command: string) {
  try {
    await navigator.clipboard.writeText(command);
    notify("Command copied", "ok");
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
      notify(withBackup("Bindings resorted"), "ok");
    } else {
      notify(s.error ?? "Reload failed", "error");
    }
  } catch (e) {
    notify(String(e), "error");
  }
}

// Resolve a live input to its token and bound action(s) and show it. Axes
// stream events; resolve one per axis at most every AXIS_RESOLVE_MS.
const AXIS_RESOLVE_MS = 150;
const lastAxisResolve = new Map<string, number>();
function axisDue(guid: string, key: string): boolean {
  const k = `${guid}#${key}`;
  const now = Date.now();
  if (now - (lastAxisResolve.get(k) ?? 0) < AXIS_RESOLVE_MS) return false;
  lastAxisResolve.set(k, now);
  return true;
}

interface InputResolution {
  token: string | null;
  actions: BoundAction[];
}

async function showBinding(p: JoyInput) {
  const key = inputKey(p);
  if (!key) return;
  // A new live input takes over from a selected binding.
  flash.value = null;
  try {
    if (p.kind === "button" || p.kind === "axis" || p.kind === "hat") {
      const res = await invoke<InputResolution>("resolve_input", {
        instanceId: p.instance_id,
        kind: p.kind,
        index: p.index,
        direction: p.kind === "hat" ? p.direction : null,
      });
      applyResolution(p, key, res);
      return;
    }
    // Keyboard and gamepad: SC folds modifiers into the token, so ask for the
    // combos with the other held names first and fall back to the plain token.
    const prefix = p.kind === "key" ? "kb1" : "gp1";
    const others = (heldNames.get(p.guid) ?? []).filter((n) => n !== p.name);
    const candidates = [...others.map((n) => `${prefix}_${n}+${p.name}`), `${prefix}_${p.name}`];
    applyResolution(p, key, await invoke<InputResolution>("resolve_tokens", { candidates }));
  } catch {
    /* ignore transient resolve errors */
  }
}

function applyResolution(p: JoyInput, key: string, res: InputResolution) {
  const d = deviceOfEvent(p);
  currentKey.value = { guid: p.guid, key };
  currentInput.value = {
    device: d ? deviceName(d) : p.guid,
    kind: d?.kind ?? "joystick",
    sc_guid: d?.sc_product_guid ?? null,
    token: res.token,
    sdl: sdlInputName(p),
    actions: res.actions,
    in_imagemap: inMap(p.guid, key),
  };
  if (res.actions.length && currentInput.value.in_imagemap === false) {
    // One toast per input while it is on screen — hammering a button
    // must not stack them.
    const tk = `${p.guid}#${key}`;
    const now = Date.now();
    if (lastMissingToast.key !== tk || now - lastMissingToast.at > TOAST_MS) {
      lastMissingToast = { key: tk, at: now };
      notify(`No shape for ${res.token ? tokenLabel(res.token) : currentInput.value.sdl}`, "warn");
    }
  }
}

// The full SC token a press stands for when it is meant as a new binding:
// `eventToken` plus the other held keys / pad buttons folded in as modifiers
// (`kb1_lalt+x`, `gp1_shoulderl+thumblx` — SC has such axis combos among its
// defaults), the way SC stores a combo.
function rebindToken(p: JoyInput): string | null {
  const token = eventToken(p);
  if (!token || (p.kind !== "key" && p.kind !== "padbutton" && p.kind !== "padaxis")) return token;
  const others = (heldNames.get(p.guid) ?? []).filter((n) => n !== p.name);
  if (!others.length) return token;
  const prefix = p.kind === "key" ? "kb1" : "gp1";
  return `${prefix}_${[...others].reverse().join("+")}+${p.name}`;
}

// The SC token an input event stands for, with the jsN from the game's own
// device order (like the Last Input card); null when SC cannot bind it (no
// order, device not listed, pad without a slot, unknown axis name, hat
// diagonal or centre).
function eventToken(p: JoyInput): string | null {
  const d = deviceOfEvent(p);
  const js = () => slotOf(d)?.effective_instance ?? null;
  switch (p.kind) {
    case "key":
      return `kb1_${p.name}`;
    case "padbutton":
    case "padaxis":
      return d?.gamepad_slot ? `gp1_${p.name}` : null;
    case "button": {
      const n = js();
      return n ? `js${n}_button${p.index + 1}` : null;
    }
    case "axis": {
      const n = js();
      const axis = d?.axes[p.index];
      return n && axis ? `js${n}_${axis}` : null;
    }
    default: {
      const n = js();
      const cardinal = ["up", "right", "down", "left"].includes(p.direction);
      return n && cardinal ? `js${n}_hat${p.index + 1}_${p.direction}` : null;
    }
  }
}

// One path for every live input, whatever made it: the raw log collects in
// every mode, the editor owns the input while an image-map is being edited.
function onInput(p: JoyInput) {
  events.value.unshift({ ...p, id: ++eventSeq, at: Date.now(), token: eventToken(p) });
  if (events.value.length > MAX_EVENTS) events.value.pop();
  trackHeld(p);
  // Keys exist only in the webview (the backend never sees them): handed to
  // the views by direct call, not a prop — a prop would collapse a press and
  // its release arriving in one tick (the wheel) into the release alone.
  if (p.kind === "key") {
    editor.value?.takeInput(p);
    bindingsView.value?.takeInput(p);
  }
  if (mode.value !== "monitor") return;
  trackActive(p);
  // A pad without a slot is not gp1 — SC has no bindings for it. A joystick
  // the game does not see has no jsN: nothing to show, only the Device List
  // (and the app log) tell that SDL lists it and the game does not.
  const source = deviceOfEvent(p);
  if ((p.kind === "padbutton" || p.kind === "padaxis") && source?.gamepad_slot === null) return;
  if (source && deviceUnseen(source)) return;
  switch (p.kind) {
    case "button":
    case "padbutton":
    case "key":
      if (p.pressed) showBinding(p);
      break;
    case "hat":
      if (p.direction !== "centered") showBinding(p);
      break;
    case "axis":
      if (axisDue(p.guid, `axis:${p.index}`)) showBinding(p);
      break;
    case "padaxis":
      if (!derivedHeld(p.guid, p.name) && axisDue(p.guid, `pad:${p.name}`)) showBinding(p);
      break;
  }
}

// "warn" is degraded but running (logged as a warning), "hint" is guidance
// ("Game started"), logged as info.
interface Toast {
  id: number;
  message: string;
  type: ToastType;
}

// How long a toast stays on screen.
const TOAST_MS = 4000;
const toasts = ref<Toast[]>([]);
// The last "not in image-map" toast for a live input, to avoid stacking.
let lastMissingToast = { key: "", at: 0 };
let toastSeq = 0;
let eventSeq = 0;

// Show a transient toast that dismisses itself after a few seconds. An
// error toast is the only trace of most failures, so it goes to the log too.
function notify(message: string, type: ToastType = "ok") {
  if (type === "error") console.error(`toast: ${message}`);
  else if (type === "warn") console.warn(`toast: ${message}`);
  else if (type === "hint") console.info(`hint: ${message}`);
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
  currentLoaded.value = s.loaded;
  error.value = s.loaded ? null : s.error;
}

// Rebinds were written into actionmaps.xml — same follow-up as a reload.
async function onSaved(s: LoadStatus) {
  takeStatus(s);
  await loadClash();
  if (s.loaded) {
    notify(withBackup("Saved"), "ok");
  } else {
    notify(s.error ?? "Load failed", "error");
  }
}

// A backup was written back over actionmaps.xml — same follow-up as a reload.
async function onApplied(s: LoadStatus) {
  takeStatus(s);
  await loadClash();
  if (s.loaded) {
    notify(withBackup("Applied"), "ok");
  } else {
    notify(s.error ?? "Load failed", "error");
  }
}

async function onRestored(s: LoadStatus) {
  takeStatus(s);
  await loadClash();
  if (s.loaded) {
    notify(withBackup("Restored"), "ok");
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
  endStartup();
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
    notify(`Loaded ${s.bindings.length} binding(s)`, "ok");
  } else {
    notify(s.error ?? "Load failed", "error");
  }
}

// Refresh = everything from scratch: devices, actionmaps.xml, Game.log, then
// the clash report on top of those.
async function refresh() {
  if (!(await bindingsSettled())) return;
  loading.value = true;
  error.value = null;
  try {
    devices.value = await invoke<DeviceInfo[]>("list_devices");
    const s = await invoke<LoadStatus>("reload");
    takeStatus(s);
  } catch (e) {
    console.error("refresh failed", e);
    error.value = String(e);
  } finally {
    loading.value = false;
  }
  await loadClash();
}

// What tells two device lists apart: the same devices on the same SDL
// indices with the same slots.
function deviceListKey(list: DeviceInfo[]): string {
  return JSON.stringify(list.map((d) => [d.index, d.sdl_guid, d.kind, d.gamepad_slot ?? null]));
}

// Hot-plug (and startup): re-list the devices and redo the clash report on
// top of them; actionmaps.xml and Game.log are not re-read (the install did
// not change). Returns whether the list changed: SDL raises several
// devices-changed at startup with nothing in them, and the clash report
// (DirectInput on Windows) is too slow to redo for a list that is the same.
async function refreshDevices(announce = false): Promise<boolean> {
  const before = deviceListKey(devices.value);
  try {
    devices.value = await invoke<DeviceInfo[]>("list_devices");
  } catch (e) {
    console.error("device list failed", e);
    error.value = String(e);
  }
  if (deviceListKey(devices.value) === before) return false;
  await loadClash(announce);
  return true;
}

// The Device List's extra facts (Wine's registered interfaces, hidapi's
// joystick-class devices SDL lacks). The hidapi enumeration takes a while on
// Windows, so this runs only for the Devices mode, never at startup.
async function loadDeviceInfo() {
  // Wine's registered interfaces for the Device List; empty off Linux.
  try {
    wineKeys.value = await invoke<WineKey[]>("wine_keys");
  } catch (e) {
    console.warn("wine keys failed", e);
    wineKeys.value = [];
  }
  // DirectInput's enumeration for the Device List; empty off Windows.
  try {
    dinputDevices.value = await invoke<DiDevice[]>("dinput_devices");
  } catch (e) {
    console.warn("dinput devices failed", e);
    dinputDevices.value = [];
  }
  try {
    hidOnly.value = await invoke<HidOnlyDevice[]>("hid_only_devices");
  } catch (e) {
    console.warn("hid-only devices failed", e);
    hidOnly.value = [];
  }
}

// SDL-side name of a live input, e.g. "button 5", "hat 0 up", "key lshift".
function sdlInputName(p: JoyInput): string {
  switch (p.kind) {
    case "hat":
      return `hat ${p.index} ${p.direction}`.trim();
    case "key":
      return `key ${p.name}`;
    case "padbutton":
    case "padaxis":
      return `pad ${p.name}`;
    default:
      return `${p.kind} ${p.index}`;
  }
}

// Live card colour: yellow when there is no device order at all (a joystick
// the game does not see never reaches the card, see `onInput`), blue when
// the input has SC bindings, grey otherwise. The keyboard is always there
// for SC.
function liveState(): LiveState {
  const c = currentInput.value;
  if (!c) return "none";
  if (c.kind === "joystick" && noOrder.value) return "noorder";
  return c.actions.length ? "bound" : "none";
}


let unlisten: UnlistenFn[] = [];
let stopKeyboard: (() => void) | null = null;
let stopMouse: (() => void) | null = null;

// A pad button held while the window loses focus would stay a modifier.
function clearHeld() {
  heldNames.clear();
  heldDerived.clear();
}

onMounted(async () => {
  stopKeyboard = startKeyboardCapture(onInput);
  // The mouse only while a Record button armed it (see keyboard.ts).
  stopMouse = startMouseCapture(onInput);
  try {
    keyboardLayout.value = await invoke<string | null>("keyboard_layout");
  } catch (e) {
    console.warn("keyboard layout unknown", e);
    keyboardLayout.value = null;
  }
  try {
    systemInfo.value = await invoke<SystemInfo>("system_info");
  } catch (e) {
    console.warn("system info unavailable", e);
  }
  // The updater installs only in an install it can replace; the bare
  // executable and deb / rpm only check and link the project page. A dev
  // build (`tauri dev`, no updater either) gets the simulated one, to look
  // at the GUI parts.
  if (systemInfo.value) {
    try {
      const { createUpdater } = await import("./update");
      const self = systemInfo.value.updater;
      updater.value = createUpdater(() => updateChannel.value, import.meta.env.DEV && !self, self || import.meta.env.DEV);
    } catch (e) {
      console.error("updater unavailable", e);
    }
  }
  window.addEventListener("blur", clearHeld);
  // Closing (top-bar button or the window manager) settles unsaved changes
  // first. The backend prevents every close and sends `close-requested`
  // (its own event, see `lib.rs::CloseGuard`); the acknowledgement tells it
  // the webview is alive, else it destroys the window itself after a
  // timeout. `destroy` skips the guard, `close` would run it again.
  const win = getCurrentWindow();
  let closing = false;
  // Every startup step on its own: one that fails is reported and the
  // rest still run, so a broken listener does not take the devices, the
  // game data or the image-maps down with it.
  const step = async (what: string, run: () => Promise<void>) => {
    try {
      await run();
    } catch (e) {
      console.error(`startup: ${what} failed`, e);
      notify(`${what} failed: ${e}`, "error");
    }
  };
  await step("Window close handling", async () => {
    unlisten.push(
      await listen<{ request: number }>("close-requested", async (e) => {
        await invoke("ack_close", { request: e.payload.request });
        // A second request while the dialogs are up (window manager
        // clicked twice) must not stack a second dialog.
        if (closing) return;
        closing = true;
        try {
          if (await allSettled()) await win.destroy();
        } finally {
          closing = false;
        }
      }),
    );
  });
  await step("Input events", async () => {
    unlisten.push(await listen<JoyInput>("joy-input", (e) => onInput(e.payload)));
  });
  await step("Device events", async () => {
    unlisten.push(
      await listen<DevicesChanged>("devices-changed", async (e) => {
        const changed = e.payload.added.length > 0 || e.payload.removed.length > 0;
        announceDevices(e.payload);
        if (!(await refreshDevices(changed))) return;
        await reloadMaps();
        if (mode.value === "devices") await loadDeviceInfo();
      }),
    );
  });
  // The game started and wrote a new log: its device order may have changed.
  await step("Game log events", async () => {
    unlisten.push(await listen<{ started: boolean }>("gamelog-changed", (e) => onGameLogChanged(e.payload.started)));
  });
  // The bindings file changed on disk (the game's console, an editor) and
  // the backend re-read it.
  await step("Bindings file events", async () => {
    unlisten.push(await listen<LoadStatus>("bindings-changed", (e) => onBindingsChanged(e.payload)));
  });
  // Registered before the initial fetch below, so a load finishing in
  // between is not missed.
  await step("Game data events", async () => {
    unlisten.push(await listen<LoadStatus>("scdata-changed", (e) => onScDataChanged(e.payload)));
    unlisten.push(
      await listen<ScStatus>("scdata-progress", (e) => {
        scStatus.value = e.payload;
      }),
    );
  });
  // The backend reloads actionmaps.xml itself once the game data is in.
  await step("Device list", async () => {
    await refreshDevices();
  });
  // The image-maps right behind the devices and the settings they need
  // (the map choices), ahead of the bulkier game data: the startup tile
  // waits for them (`endStartup`).
  await step("Settings", async () => {
    const cfg = await invoke<Config>("get_config");
    environments.value = cfg.environments;
    activeEnv.value = cfg.active_env;
    autoBackup.value = cfg.auto_backup;
    debugLogging.value = cfg.debug_logging;
    setDebugLogging(cfg.debug_logging);
    updateCheck.value = cfg.update_check;
    updateChannel.value = cfg.update_channel;
    overlaySize.value = cfg.overlay_size;
    overlayPosition.value = cfg.overlay_position;
    mapChoices.value = cfg.imagemap_choices ?? {};
  });
  await step("Image-maps", reloadMaps);
  mapsReady = true;
  if (startupPending) endStartup();

  try {
    scStatus.value = await invoke<ScStatus>("get_sc_status");
    // The first load may have ended before the scdata-changed listener was up.
    if (!scStatus.value.loading) endStartup();
    actionMaps.value = await invoke<ActionMap[]>("get_actions");
    tokens.value = await invoke<Record<string, string>>("get_tokens");
    // The outcome of a load that ended before the listener was up: bindings,
    // whether the profile parsed, and its error.
    takeStatus(await invoke<LoadStatus>("get_load_status"));
  } catch (e) {
    console.error("startup: game data load failed", e);
    error.value = String(e);
    // Never leave the startup tile up: the other features work regardless.
    endStartup();
  }
  // The startup check (Settings can switch it off): an update opens the
  // App Update dialog, the user decides.
  if (updater.value && updateCheck.value && (await updater.value.check())) showVersion.value = true;
  // What only the frontend knows: the viewport (the layout wants 1280x720),
  // the pixel ratio, whether localStorage works (-1 = it does not: every
  // remembered width, chip and filter silently falls back to its default).
  console.info(`frontend ready: ${innerWidth}x${innerHeight}@${devicePixelRatio}x, storage ${storageKeyCount()} key(s), keyboard layout ${keyboardLayout.value ?? "unknown"}`);
});

function storageKeyCount(): number {
  try {
    return localStorage.length;
  } catch {
    return -1;
  }
}

onUnmounted(() => {
  unlisten.forEach((fn) => fn());
  unlisten = [];
  stopKeyboard?.();
  stopKeyboard = null;
  stopMouse?.();
  stopMouse = null;
  window.removeEventListener("blur", clearHeld);
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
      :autoBackup="autoBackup"
      :debugLogging="debugLogging"
      :updateCheck="updateCheck"
      :updateChannel="updateChannel"
      :overlaySize="overlaySize"
      :overlayPosition="overlayPosition"
      @close="showSettings = false"
      @save="applySettings"
      @notify="notify"
    />

    <div v-if="starting" class="content">
      <StartupTile :sc="scStatus" :version="systemInfo?.app_version ?? ''" />
    </div>

    <div v-else-if="mode === 'monitor'" :key="activeEnv" class="content">
      <div class="top-row">
      <div class="devices-panel">
        <div class="panel-title">Input Devices</div>
        <ScrollRail>
        <DeviceTile
          v-for="d in monitorDevices"
          :key="d.index"
          :device="d"
          :slot="slotOf(d)"
          :noOrder="d.kind === 'joystick' && noOrder"
          :bindingCount="bindingCountFor(d)"
          :hidden="isStageHidden(d)"
          @toggleMap="toggleStageHidden(d)"
        />
        <DeviceTile
          v-for="j in logOnlyJoysticks"
          :key="'log:' + j.sc_product_guid"
          :device="j"
          :slot="slotFor(j.sc_product_guid)"
          :noOrder="false"
          :bindingCount="0"
          :hidden="false"
        />
        <div v-if="!monitorDevices.length && !logOnlyJoysticks.length" class="tile-none">None</div>
        </ScrollRail>
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
        :sequence="orderedDevices.map(deviceKey)"
        :placeholders="stagePlaceholders"
        :height="stageHeight"
        :imgSrc="imgSrc"
        :activeFor="activeFor"
      />
      <Splitter direction="row" @drag="dragStage" @end="saveLayout" @reset="resetStage" />

      <div
        ref="deckRow"
        class="deck-row"
        :style="{ gridTemplateColumns: `minmax(${INPUT_CARD_W.min}px, ${inputCardWidth}px) 16px minmax(${DECK_MIN}px, 1fr)` }"
      >
        <LastInputCard
          :input="currentInput"
          :state="liveState()"
          :tokenLabel="tokenLabel"
          :categoryLabel="actionmapLabel"
        />
        <Splitter direction="col" @drag="dragInputCard" @end="saveLayout" @reset="resetInputCard" />
        <BindingsDeck
          :bindings="connectedBindings"
          :currentToken="currentInput?.token ?? null"
          :liveOn="currentHeld"
          :tokenLabel="tokenLabel"
          :categoryLabel="actionmapLabel"
          :deviceLabel="deviceLabel"
          :clashOf="bindingClash"
          :noOrder="noOrder"
          :isMissing="missingInMap"
          :isFlashed="isFlashed"
          @flash="flashBinding"
        />
      </div>
    </div>

    <BindingsView
      v-else-if="mode === 'bindings'"
      :key="activeEnv"
      ref="bindingsView"
      :bindings="bindings"
      :actionMaps="actionMaps"
      :clash="clash"
      :isLogOnly="isLogOnly"
      :hasCurrent="currentLoaded"
      :tokenLabel="tokenLabel"
      :inputToken="rebindToken"
      :overlayFor="overlayFor"
      :overlaySize="overlaySize"
      :overlayPosition="overlayPosition"
      @notify="notify"
      @restored="onRestored"
      @applied="onApplied"
      @saved="onSaved"
      @copy="copyResortCommands"
    />

    <ImageMapEditor
      v-else-if="mode === 'devices'"
      :key="activeEnv"
      ref="editor"
      :devices="orderedDevices"
      :events="events"
      :chosenMapId="chosenMapId"
      :tokenLabel="tokenLabel"
      :isUnseen="deviceUnseen"
      :clash="clash"
      :wineKeys="wineKeys"
      :dinputDevices="dinputDevices"
      :hidOnly="hidOnly"
      :systemLine="systemLine"
      @choose="setMapChoice"
      @notify="notify"
      @saved="onMapsSaved"
      @clear-log="events = []"
    />

    <AppFooter :version="systemInfo?.app_version ?? ''" @version="showVersion = true">
      <template v-if="updater?.info" #mark>
        <component :is="updater.mark" @click="showVersion = true" />
      </template>
    </AppFooter>
    <VersionDialog
      v-if="showVersion"
      :version="systemInfo?.app_version ?? ''"
      :updater="updater"
      :channel="updateChannel"
      @close="showVersion = false"
    />
    <Toasts :toasts="toasts" />
    <WindowEdges />
  </main>
</template>

<style scoped>
/* Below this size the content stops shrinking and the window scrolls. */
.app {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 1280px;
  min-height: 720px;
  /* No overflow clipping here: #app is the scroll container, and the sticky
     top bar needs it to be the nearest one. */
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
  /* Overridden inline: the live card's width, the deck keeps DECK_MIN. */
  grid-template-columns: minmax(280px, 400px) 16px minmax(320px, 1fr);
  grid-template-rows: minmax(0, 1fr);
  min-height: 160px;
}
</style>
