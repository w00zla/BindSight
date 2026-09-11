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
import WindowEdges from "./components/WindowEdges.vue";
import { deviceKey, deviceName } from "./devices";
import Splitter from "./components/Splitter.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import StartupTile from "./components/StartupTile.vue";
import { setDebugLogging } from "./logging";
import type {
  DeviceKind,
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
import {
  inputKey,
  inputKeyForToken,
  inputKeysForToken,
  sameHardware,
  type HighlightClass,
  type ImageMap,
  type ImageMapSummary,
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
const keyInput = ref<JoyInput | null>(null);
const clash = ref<ClashReport | null>(null);
// The install's version and game-data load state (updated via `scdata-changed`).
const scStatus = ref<ScStatus | null>(null);
// The first game-data load after start is still running: the startup tile
// covers every mode until it ends, whatever its outcome.
const starting = ref(true);
// The startup tile stays up at least this long, so it does not just flash by.
const STARTUP_MIN_MS = 3000;
const startedAt = Date.now();

// End the startup tile, but not before it has been up STARTUP_MIN_MS.
function endStartup() {
  const left = STARTUP_MIN_MS - (Date.now() - startedAt);
  if (left > 0) setTimeout(() => (starting.value = false), left);
  else starting.value = false;
}
// Set while a base-path change is being loaded, so its result gets a toast.
let awaitingPathLoad = false;
// SC Product GUIDs the user marked "SC doesn't see this device" (persisted per OS).
const ignoredDevices = ref<string[]>([]);
const error = ref<string | null>(null);
// The live actionmaps.xml is parsed (from the last LoadStatus).
const profileLoaded = ref(false);
const loading = ref(false);
const showSettings = ref(false);
// Back up actionmaps.xml before BindSight overwrites it (Settings).
const autoBackup = ref(true);
// Write DEBUG records to bindsight.log (Settings).
const debugLogging = ref(false);

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

// Devices the stage shows at all: a further pad has no slot, so it has no
// bindings and no tile, and the user can hide any device by hand.
function onStage(d: DeviceInfo): boolean {
  if (d.kind === "gamepad" && d.gamepad_slot === null) return false;
  if (isIgnored(d.sc_product_guid)) return false;
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

// Resizable layout: stage height and live-card width, remembered locally.
const LAYOUT_KEY = "bindsight.layout";
const STAGE_H = { def: 440, min: 120 };
const LIVE_W = { def: 400, min: 280 };
// The deck never gets narrower / lower than this; stage and live card take
// the rest.
const DECK_MIN = 320;
const DECK_MIN_H = 160;
const stageHeight = ref(STAGE_H.def);
const liveWidth = ref(LIVE_W.def);
try {
  const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}") as { stageHeight?: number; liveWidth?: number };
  if (typeof saved.stageHeight === "number") stageHeight.value = saved.stageHeight;
  if (typeof saved.liveWidth === "number") liveWidth.value = saved.liveWidth;
} catch {
  /* defaults */
}
// Stage height / live width when a drag began, plus the deck's height then:
// stage and deck share the column, so the stage may grow by what the deck
// has above DECK_MIN_H.
let layoutStart: { stageHeight: number; liveWidth: number; deckHeight: number } | null = null;
const clamp = (v: number, r: { min: number; max: number }) => Math.min(Math.max(v, r.min), r.max);
const deckRow = ref<HTMLElement | null>(null);
function startLayout() {
  layoutStart ??= { stageHeight: stageHeight.value, liveWidth: liveWidth.value, deckHeight: deckRow.value?.clientHeight ?? Infinity };
  return layoutStart;
}
function dragStage(delta: number) {
  const start = startLayout();
  const max = start.stageHeight + start.deckHeight - DECK_MIN_H;
  stageHeight.value = clamp(start.stageHeight + delta, { min: STAGE_H.min, max: Math.max(max, STAGE_H.min) });
}
// The live card grows until the deck is down to DECK_MIN in the current row.
function dragLive(delta: number) {
  const start = startLayout();
  const max = (deckRow.value?.clientWidth ?? Infinity) - 16 - DECK_MIN;
  liveWidth.value = clamp(start.liveWidth + delta, { min: LIVE_W.min, max: Math.max(max, LIVE_W.min) });
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
  orderedDevices.value.filter(
    (d) => onStage(d) && !mapViews.value.some((v) => v.device.index === d.index),
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
    const id = chosenMapId(d);
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

// The editor and the Bindings mode guard their unsaved changes; leaving
// Devices or Bindings can be refused.
const editor = ref<InstanceType<typeof ImageMapEditor> | null>(null);
const tools = ref<InstanceType<typeof ToolsView> | null>(null);

async function setMode(m: Mode) {
  if (mode.value === "devices" && m !== "devices") {
    if ((await editor.value?.requestLeave()) === false) return;
  }
  if (mode.value === "tools" && m !== "tools") {
    if ((await tools.value?.requestLeave()) === false) return;
  }
  mode.value = m;
  // Inputs released while the editor was open were never seen here.
  activeInputs.value = {};
  if (m === "live") await reloadMaps();
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

// A binding clicked in the list, selected until clicked again or another
// one is picked; it is kept lit on the image-map image when it can be.
const pinned = ref<ResolvedBinding | null>(null);
const pinnedTarget = computed(() => (pinned.value ? pinTarget(pinned.value) : null));

function activeFor(guid: string): Map<string, HighlightClass> {
  const m = new Map<string, HighlightClass>(Object.entries(activeInputs.value[guid] ?? {}));
  const t = pinnedTarget.value;
  if (t?.guid === guid) for (const k of t.keys) m.set(k, "bound");
  return m;
}

// Does the map shown for this device (by SDL GUID) have an area for the
// input? `null` when the device has no image-map.
function inMap(sdlGuid: string, key: string): boolean | null {
  const id = chosenMapId(deviceOf(sdlGuid));
  const p = id ? loadedMaps.value[id] : null;
  if (!p) return null;
  return p.areas.some((a) => a.input === key);
}

function deviceOf(sdlGuid: string): DeviceInfo | undefined {
  return devices.value.find((d) => d.sdl_guid === sdlGuid);
}

// The connected device a binding belongs to: joysticks by GUID, keyboard and
// gamepad by kind (SC has exactly one of each).
function deviceForBinding(b: ResolvedBinding): DeviceInfo | undefined {
  if (b.device_kind === "keyboard") return devices.value.find((d) => d.kind === "keyboard");
  if (b.device_kind === "gamepad") return devices.value.find((d) => d.kind === "gamepad" && d.gamepad_slot !== null);
  return devices.value.find((d) => !!b.device_guid && sameHardware(d.hardware_id, b.device_guid));
}

// Where a pinned binding lights up: the device and its input's key, plus
// every key of the map that gets lit (a combo's modifiers too, when the
// map has areas for them).
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
  if (!key) return { reason: `${tokenLabel(b.token)}: no SDL axis for it (${d.axes_error ?? "not in HID descriptor"})` };
  const has = inMap(d.sdl_guid, key);
  if (has === null) return { reason: `${deviceName(d)} has no image-map` };
  if (!has) return { reason: `No area for ${tokenLabel(b.token)}` };
  const keys = inputKeysForToken(b.token, d).filter((k) => inMap(d.sdl_guid, k));
  return { guid: d.sdl_guid, key, keys };
}

// The device has an image-map, but no area for this binding's input.
function missingInMap(b: ResolvedBinding): boolean {
  const t = pinTarget(b);
  return !!t && inMap(t.guid, t.key) === false;
}

function isPinned(b: ResolvedBinding): boolean {
  const p = pinned.value;
  return !!p && p.token === b.token && p.actionmap === b.actionmap && p.action === b.action;
}

// Select the row either way; say why nothing lights up when it cannot (a
// missing area is already tagged on the row itself).
function togglePin(b: ResolvedBinding) {
  if (isPinned(b)) {
    pinned.value = null;
    return;
  }
  pinned.value = b;
  const r = resolvePin(b);
  if ("reason" in r && !missingInMap(b)) notify(r.reason, "error");
}

// Buttons and keys stay lit while held, hats until centered, axes pulse.
function trackActive(p: JoyInput) {
  if (p.kind === "button" || p.kind === "padbutton" || p.kind === "key") {
    const key = p.kind === "button" ? `button:${p.index}` : `${p.kind === "key" ? "key" : "pad"}:${p.name}`;
    if (p.pressed) setActive(p.guid, key, "none");
    else clearActive(p.guid, (k) => k === key);
    return;
  }
  if (p.kind === "hat") {
    clearActive(p.guid, (k) => k.startsWith(`hat:${p.index}:`));
    if (p.direction !== "centered") setActive(p.guid, `hat:${p.index}:${p.direction}`, "none");
    return;
  }
  const key = p.kind === "axis" ? `axis:${p.index}` : `pad:${p.name}`;
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

// Currently held key/pad-button names per device GUID, most recent first: the
// modifier candidates for the next press (`kb1_lalt+x`).
const heldNames = new Map<string, string[]>();

function trackHeld(p: JoyInput) {
  if (p.kind !== "key" && p.kind !== "padbutton") return;
  const rest = (heldNames.get(p.guid) ?? []).filter((n) => n !== p.name);
  heldNames.set(p.guid, p.pressed ? [p.name, ...rest] : rest);
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
  return bindings.value.filter((b) => sameHardware(b.device_guid, d.sc_product_guid)).length;
}

// SC did not list this device at its last start. The keyboard is always there;
// a slotted pad rides on the log's xinput line.
function deviceUnseen(d: DeviceInfo): boolean {
  if (d.kind === "keyboard") return false;
  if (d.kind === "gamepad") return d.gamepad_slot !== null && clash.value?.gamepad_seen === false;
  return isUnseen(d.sc_product_guid);
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

// Display order everywhere a device list is shown: the keyboard, the slotted
// pad, the joysticks SC sees, then everything SC does not (unseen, excluded,
// pads without a slot) — SDL order within a group.
function deviceRank(d: DeviceInfo): number {
  if (d.kind === "keyboard") return 0;
  if (d.kind === "gamepad" && d.gamepad_slot === null) return 3;
  if (deviceUnseen(d) || isIgnored(d.sc_product_guid)) return 3;
  return d.kind === "gamepad" ? 1 : 2;
}
const orderedDevices = computed<DeviceInfo[]>(() =>
  [...devices.value].sort((a, b) => deviceRank(a) - deviceRank(b) || a.index - b.index),
);
// Settings dialog Save: apply the exclusions and the environments; the
// backend reloads when the active environment changed.
async function applySettings(s: {
  environments: Record<string, Environment>;
  ignored: string[];
  autoBackup: boolean;
  debugLogging: boolean;
}) {
  showSettings.value = false;
  try {
    ignoredDevices.value = await invoke<string[]>("set_ignored_devices", { guids: s.ignored });
    await invoke("set_auto_backup", { enabled: s.autoBackup });
    autoBackup.value = s.autoBackup;
    await invoke("set_debug_logging", { enabled: s.debugLogging });
    debugLogging.value = s.debugLogging;
    setDebugLogging(s.debugLogging);
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
      notify("Bindings resorted", "ok");
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
  try {
    if (p.kind === "button" || p.kind === "axis" || p.kind === "hat") {
      const res = await invoke<InputResolution>("resolve_input", {
        guid: p.guid,
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
  const d = deviceOf(p.guid);
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
      notify(`No area for ${res.token ? tokenLabel(res.token) : currentInput.value.sdl}`, "error");
    }
  }
  // Upgrade the image-map highlight to blue when SC has a binding — but only
  // while the input is still held (the resolve is async).
  if (activeInputs.value[p.guid]?.[key] !== undefined) {
    setActive(p.guid, key, res.actions.length ? "bound" : "none");
  }
}

// The full SC token a press stands for when it is meant as a new binding:
// `eventToken` plus the other held keys / pad buttons folded in as modifiers
// (`kb1_lalt+x`), the way SC stores a combo.
function rebindToken(p: JoyInput): string | null {
  const token = eventToken(p);
  if (!token || (p.kind !== "key" && p.kind !== "padbutton")) return token;
  const others = (heldNames.get(p.guid) ?? []).filter((n) => n !== p.name);
  if (!others.length) return token;
  const prefix = p.kind === "key" ? "kb1" : "gp1";
  return `${prefix}_${[...others].reverse().join("+")}+${p.name}`;
}

// The SC token an input event stands for, with the jsN from actionmaps.xml
// (like the Last Input card); null when SC cannot bind it (device not in the
// profile, pad without a slot, unknown axis name, hat diagonal or centre).
function eventToken(p: JoyInput): string | null {
  const d = deviceOf(p.guid);
  const js = () => slotFor(d?.sc_product_guid ?? null)?.stored_instance ?? null;
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
  events.value.unshift({ ...p, at: Date.now(), token: eventToken(p) });
  if (events.value.length > MAX_EVENTS) events.value.pop();
  trackHeld(p);
  if (p.kind === "key") keyInput.value = p;
  if (mode.value !== "live") return;
  trackActive(p);
  // A pad without a slot is not gp1 — SC has no bindings for it.
  if ((p.kind === "padbutton" || p.kind === "padaxis") && deviceOf(p.guid)?.gamepad_slot === null) return;
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
      if (axisDue(p.guid, `pad:${p.name}`)) showBinding(p);
      break;
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

// Rebinds were written into actionmaps.xml — same follow-up as a reload.
async function onSaved(s: LoadStatus) {
  takeStatus(s);
  await loadClash();
  if (s.loaded) {
    notify("Saved", "ok");
  } else {
    notify(s.error ?? "Load failed", "error");
  }
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
  loading.value = true;
  error.value = null;
  try {
    devices.value = await invoke<DeviceInfo[]>("list_devices");
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
    devices.value = await invoke<DeviceInfo[]>("list_devices");
  } catch (e) {
    error.value = String(e);
  }
  await loadClash();
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

// Live card colour: yellow when SC doesn't see the device (unseen or excluded
// — any SC token is meaningless then), blue when the input has SC bindings,
// grey otherwise. The keyboard is always there for SC.
function liveState(): "unseen" | "bound" | "none" {
  const c = currentInput.value;
  if (!c) return "none";
  if (c.kind === "joystick" && (isIgnored(c.sc_guid) || isUnseen(c.sc_guid))) return "unseen";
  if (c.kind === "gamepad" && (isIgnored(c.sc_guid) || clash.value?.gamepad_seen === false)) return "unseen";
  return c.actions.length ? "bound" : "none";
}


let unlisten: UnlistenFn[] = [];
let stopKeyboard: (() => void) | null = null;
let stopMouse: (() => void) | null = null;

// Keys are captured in every mode (the webview never gets to act on them;
// text fields and open dialogs are skipped inside the capture), so a rebind
// flow can rely on it anywhere.
function keyboardActive(): boolean {
  return true;
}

// A pad button held while the window loses focus would stay a modifier.
function clearHeld() {
  heldNames.clear();
}

onMounted(async () => {
  stopKeyboard = startKeyboardCapture(onInput, keyboardActive);
  // The mouse only while a Record button armed it (see keyboard.ts).
  stopMouse = startMouseCapture(onInput);
  try {
    keyboardLayout.value = await invoke<string | null>("keyboard_layout");
  } catch {
    keyboardLayout.value = null;
  }
  window.addEventListener("blur", clearHeld);
  unlisten.push(await listen<JoyInput>("joy-input", (e) => onInput(e.payload)));
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
    // The first load may have ended before the scdata-changed listener was up.
    if (!scStatus.value.loading) endStartup();
    actionMaps.value = await invoke<ActionMap[]>("get_actions");
    tokens.value = await invoke<Record<string, string>>("get_tokens");
    const cfg = await invoke<Config>("get_config");
    environments.value = cfg.environments;
    activeEnv.value = cfg.active_env;
    ignoredDevices.value = cfg.ignored_devices;
    autoBackup.value = cfg.auto_backup;
    debugLogging.value = cfg.debug_logging;
    setDebugLogging(cfg.debug_logging);
    mapChoices.value = cfg.imagemap_choices ?? {};
    // The outcome of a load that ended before the listener was up: bindings,
    // whether the profile parsed, and its error.
    takeStatus(await invoke<LoadStatus>("get_load_status"));
  } catch (e) {
    error.value = String(e);
    // Never leave the startup tile up: the other features work regardless.
    endStartup();
  }
  await reloadMaps();
});

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
      :devices="orderedDevices"
      :ignored="ignoredDevices"
      :autoBackup="autoBackup"
      :debugLogging="debugLogging"
      @close="showSettings = false"
      @save="applySettings"
      @notify="notify"
    />

    <div v-if="starting" class="content">
      <StartupTile :sc="scStatus" />
    </div>

    <div v-else-if="mode === 'live'" class="content">
      <div class="top-row">
      <div class="devices-panel">
        <div class="panel-title">Connected Devices</div>
        <div class="rail">
        <DeviceTile
          v-for="d in orderedDevices"
          :key="d.index"
          :device="d"
          :slot="slotFor(d.sc_product_guid)"
          :ignored="isIgnored(d.sc_product_guid)"
          :unseen="deviceUnseen(d)"
          :bindingCount="bindingCountFor(d)"
          :hidden="isStageHidden(d)"
          @toggleMap="toggleStageHidden(d)"
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
        :style="{ gridTemplateColumns: `minmax(${LIVE_W.min}px, ${liveWidth}px) 16px minmax(${DECK_MIN}px, 1fr)` }"
      >
        <LiveCard
          :input="currentInput"
          :state="liveState()"
          :excluded="isIgnored(currentInput?.sc_guid ?? null)"
          :tokenLabel="tokenLabel"
          :categoryLabel="actionmapLabel"
        />
        <Splitter direction="col" @drag="dragLive" @end="saveLayout" @reset="resetLive" />
        <BindingsDeck
          :bindings="connectedBindings"
          :currentToken="currentInput?.token ?? null"
          :liveOn="currentHeld"
          :tokenLabel="tokenLabel"
          :categoryLabel="actionmapLabel"
          :deviceLabel="deviceLabel"
          :isClash="bindingClash"
          :isMissing="missingInMap"
          :isPinned="isPinned"
          @pin="togglePin"
        />
      </div>
    </div>

    <ToolsView
      v-else-if="mode === 'tools'"
      ref="tools"
      :bindings="bindings"
      :actionMaps="actionMaps"
      :hasCurrent="profileLoaded"
      :keyInput="keyInput"
      :tokenLabel="tokenLabel"
      :inputToken="rebindToken"
      @notify="notify"
      @restored="onRestored"
      @saved="onSaved"
    />

    <ImageMapEditor
      v-else-if="mode === 'devices'"
      ref="editor"
      :devices="orderedDevices"
      :events="events"
      :keyInput="keyInput"
      :chosenMapId="chosenMapId"
      :tokenLabel="tokenLabel"
      @choose="setMapChoice"
      @notify="notify"
      @saved="onMapsSaved"
      @clear-log="events = []"
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
  /* Overridden inline: the live card's width, the deck keeps DECK_MIN. */
  grid-template-columns: minmax(280px, 400px) 16px minmax(320px, 1fr);
  grid-template-rows: minmax(0, 1fr);
  min-height: 160px;
}
</style>
