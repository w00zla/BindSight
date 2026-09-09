<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import DeviceImage from "./components/DeviceImage.vue";
import ProfileEditor from "./components/ProfileEditor.vue";
import type { DeviceInfo, JoyInput } from "./types";
import { sameHardware, type HighlightClass, type HwProfile, type HwProfileSummary } from "./hwprofile";

interface Action {
  name: string;
  label: string | null;
  description: string | null;
  joystick_default: string | null;
}

interface ActionMap {
  name: string;
  label: string | null;
  actions: Action[];
}

interface ResolvedBinding {
  token: string;
  device: string | null;
  device_guid: string | null;
  actionmap: string;
  action: string;
  label: string | null;
  // A shipped default from defaultProfile.xml (on js1), not a user rebind.
  is_default: boolean;
}

interface LoadStatus {
  base_path: string;
  actionmaps_path: string;
  loaded: boolean;
  error: string | null;
  bindings: ResolvedBinding[];
}

interface BoundAction {
  actionmap: string;
  action: string;
  label: string | null;
  is_default: boolean;
}

// One joystick in SC's order: the jsN SC assigns it vs the jsN its bindings
// were saved under. connected_now: SDL sees it right now (a Game.log slot can
// be unplugged since SC started).
interface SlotStatus {
  effective_instance: number;
  stored_instance: number | null;
  sc_product_guid: string | null;
  name: string | null;
  clash: boolean;
  connected_now: boolean;
}

// A saved slot whose device is not in SC's list — it dangles and shifts the rest.
interface MissingSlot {
  stored_instance: number;
  name: string;
  sc_product_guid: string | null;
}

// An SDL-visible device SC did not list at its last start: hidden by Wine, or
// plugged in after SC started.
interface UnseenDevice {
  name: string | null;
  sc_product_guid: string | null;
}

// Why Game.log could not be used, so the GUI can say exactly what is wrong.
type GameLogError =
  | { kind: "not_found"; path: string; reason: string }
  | { kind: "no_device_lines"; path: string };

// One resort step: the bindings saved under js{from} belong on js{to}.
interface ResortMove {
  from: number;
  to: number;
  name: string | null;
}

// Game.log is the only order source: with log_error set, everything else is
// empty and nothing is said about the order.
interface ClashReport {
  connected: SlotStatus[];
  missing: MissingSlot[];
  unseen: UnseenDevice[];
  log_timestamp: string | null;
  log_error: GameLogError | null;
  has_clash: boolean;
  resort: ResortMove[];
  // In-game equivalent of `resort`: pp_resortdevices swaps, in order.
  resort_commands: string[];
}

const MAX_EVENTS = 50;

const devices = ref<DeviceInfo[]>([]);
const events = ref<JoyInput[]>([]);
const actionMaps = ref<ActionMap[]>([]);
const basePath = ref("");
const bindings = ref<ResolvedBinding[]>([]);
const tokens = ref<Record<string, string>>({});
// `sdl` is the SDL-side input name, shown when SC has no token for the input;
// `sc_guid` lets the tile tell whether SC sees the device at all.
const currentInput = ref<{
  device: string;
  sc_guid: string | null;
  token: string | null;
  sdl: string;
  actions: BoundAction[];
  // Whether the device's hardware profile has an area for this input; `null`
  // when the device has no profile at all.
  in_profile: boolean | null;
} | null>(null);
const clash = ref<ClashReport | null>(null);
// SC Product GUIDs the user marked "SC doesn't see this device" (persisted per OS).
const ignoredDevices = ref<string[]>([]);
const error = ref<string | null>(null);
const loading = ref(false);

// --- hardware profiles -----------------------------------------------------

// How long an axis stays highlighted after its last event (axes never rest).
const AXIS_PULSE_MS = 400;

const mode = ref<"live" | "profiles">("live");
const profileSummaries = ref<HwProfileSummary[]>([]);
// Profile id -> full profile, and `<profile id>/<file>` -> image data URL.
const loadedProfiles = ref<Record<string, HwProfile>>({});
const profileImages = ref<Record<string, string>>({});
// Lowercase hardware id -> chosen profile id (from config.json).
const profileChoices = ref<Record<string, string>>({});
// SDL GUID -> input key -> highlight class, for the currently active inputs.
const activeInputs = ref<Record<string, Record<string, HighlightClass>>>({});
const axisTimers = new Map<string, number>();

function profilesFor(guid: string | null): HwProfileSummary[] {
  return profileSummaries.value.filter((s) => sameHardware(s.hardware_id, guid));
}

// The user's pick for this device, else the single/first matching profile.
function chosenProfileId(guid: string | null): string | null {
  const list = profilesFor(guid);
  if (!list.length) return null;
  const pick = guid ? profileChoices.value[guid.toLowerCase()] : undefined;
  return list.find((s) => s.id === pick)?.id ?? list[0].id;
}

function imgSrc(id: string, file: string): string {
  return profileImages.value[`${id}/${file}`] ?? "";
}

interface ProfileView {
  device: DeviceInfo;
  profile: HwProfile;
  options: HwProfileSummary[];
}

const profileViews = computed<ProfileView[]>(() =>
  devices.value.flatMap((d) => {
    const id = chosenProfileId(d.sc_product_guid);
    const p = id ? loadedProfiles.value[id] : null;
    return p ? [{ device: d, profile: p, options: profilesFor(d.sc_product_guid) }] : [];
  }),
);

async function loadProfileImage(id: string, file: string) {
  const key = `${id}/${file}`;
  if (profileImages.value[key]) return;
  try {
    profileImages.value[key] = await invoke<string>("read_hw_profile_image", { id, file });
  } catch {
    /* a missing image just stays blank */
  }
}

async function loadChosenProfiles() {
  for (const d of devices.value) {
    const id = chosenProfileId(d.sc_product_guid);
    if (!id || loadedProfiles.value[id]) continue;
    try {
      const p = await invoke<HwProfile>("get_hw_profile", { id });
      loadedProfiles.value[id] = p;
      await loadProfileImage(p.id, p.image.file);
    } catch {
      /* skip a profile that will not load */
    }
  }
}

async function reloadProfiles() {
  try {
    profileSummaries.value = await invoke<HwProfileSummary[]>("list_hw_profiles");
  } catch (e) {
    error.value = String(e);
    return;
  }
  await loadChosenProfiles();
}

// A save in the editor can change the image and areas — drop the caches.
async function onProfilesSaved() {
  loadedProfiles.value = {};
  profileImages.value = {};
  await reloadProfiles();
}

async function setMode(m: "live" | "profiles") {
  mode.value = m;
  // Inputs released while the editor was open were never seen here.
  activeInputs.value = {};
  if (m === "live") await reloadProfiles();
}

async function setProfileChoice(guid: string | null, id: string) {
  if (!guid) return;
  try {
    const cfg = await invoke<{ profile_choices: Record<string, string> }>("set_hw_profile_choice", {
      hardwareId: guid,
      profileId: id || null,
    });
    profileChoices.value = cfg.profile_choices;
    await loadChosenProfiles();
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

// A binding clicked in the list, kept lit on the profile image until clicked
// again or another one is picked.
const pinned = ref<{ guid: string; key: string } | null>(null);

function activeFor(guid: string): Map<string, HighlightClass> {
  const m = new Map<string, HighlightClass>(Object.entries(activeInputs.value[guid] ?? {}));
  if (pinned.value?.guid === guid) m.set(pinned.value.key, "bound");
  return m;
}

// Hardware-profile input key for an SC token on a device. Undoes the +1
// offset of button/hat numbering; axes go through the device's HID-derived
// axis names (`js2_rotz` -> the SDL index whose name is `rotz`).
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

// Does the profile shown for this device (by SDL GUID) have an area for the
// input? `null` when the device has no profile.
function inProfile(sdlGuid: string, key: string): boolean | null {
  const d = devices.value.find((dev) => dev.sdl_guid === sdlGuid);
  const id = chosenProfileId(d?.sc_product_guid ?? null);
  const p = id ? loadedProfiles.value[id] : null;
  if (!p) return null;
  return p.areas.some((a) => a.input === key);
}

// Can a binding in the list be lit on a profile image? Needs a connected
// device with a profile and a mappable token.
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
  const has = inProfile(d.sdl_guid, key);
  if (has === null) return { reason: `${d.sc_name ?? d.sdl_name} has no HW profile` };
  if (!has) return { reason: `${tokenLabel(b.token)} not in HW profile` };
  return { guid: d.sdl_guid, key };
}

// The device has a profile, but no area for this binding's input.
function missingInProfile(b: ResolvedBinding): boolean {
  const t = pinTarget(b);
  return !!t && inProfile(t.guid, t.key) === false;
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

// Whether the report rests on a usable Game.log.
const hasLog = computed(() => !!clash.value && !clash.value.log_error && !!clash.value.log_timestamp);

// GUIDs SC did not list at its last start (Game.log source only).
const unseenGuids = computed<Set<string>>(
  () => new Set((clash.value?.unseen ?? []).map((u) => u.sc_product_guid).filter((g): g is string => !!g)),
);

function isUnseen(guid: string | null): boolean {
  return !!guid && unseenGuids.value.has(guid);
}

// Slots in SC's last-start list whose device is unplugged now.
const unpluggedSinceStart = computed<SlotStatus[]>(() => (clash.value?.connected ?? []).filter((s) => !s.connected_now));

// Game.log timestamps are UTC ("...Z"); show them in local time as
// "YYYY-MM-DD HH:MM:SS". Unparseable input is shown as-is.
function fmtTimestamp(ts: string | null): string {
  if (!ts) return "?";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function isIgnored(guid: string | null): boolean {
  return !!guid && ignoredDevices.value.some((g) => g.toLowerCase() === guid.toLowerCase());
}

// Flip "SC doesn't see this device" for a device and persist it. The clash
// report is recomputed with the device treated as unplugged.
async function toggleIgnored(guid: string | null) {
  if (!guid) return;
  const next = isIgnored(guid)
    ? ignoredDevices.value.filter((g) => g.toLowerCase() !== guid.toLowerCase())
    : [...ignoredDevices.value, guid];
  try {
    ignoredDevices.value = await invoke<string[]>("set_ignored_devices", { guids: next });
    await loadClash();
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
    bindings.value = s.bindings;
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
      in_profile: inProfile(guid, key),
    };
    if (res.actions.length && currentInput.value.in_profile === false) {
      // One toast per input while it is on screen — hammering a button
      // must not stack them.
      const tk = `${guid}#${key}`;
      const now = Date.now();
      if (lastMissingToast.key !== tk || now - lastMissingToast.at > TOAST_MS) {
        lastMissingToast = { key: tk, at: now };
        notify(`${res.token ? tokenLabel(res.token) : currentInput.value.sdl} not in HW profile`, "error");
      }
    }
    // Upgrade the profile highlight to blue when SC has a binding — but only
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
// The last "not in profile" toast for a live input, to avoid stacking.
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

async function saveBasePath() {
  try {
    const s = await invoke<LoadStatus>("set_base_path", { path: basePath.value });
    bindings.value = s.bindings;
    await loadClash();
    if (s.loaded) {
      notify(`Loaded ${s.bindings.length} joystick binding(s)`, "ok");
    } else {
      notify(s.error ?? "Load failed", "error");
    }
  } catch (e) {
    notify(String(e), "error");
  }
}

// Show the action's label ("<null>" literally when it is missing). The
// description is kept in the data but not shown for now.
function actionText(a: Action): string {
  return a.label ?? "<null>";
}

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    devices.value = await invoke<DeviceInfo[]>("list_joysticks");
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
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

// Live tile colour: yellow when SC doesn't see the device (unseen or excluded —
// any SC token is meaningless then), blue when the input has SC bindings, grey
// otherwise.
function liveState(): "unseen" | "bound" | "none" {
  const c = currentInput.value;
  if (!c) return "none";
  if (isIgnored(c.sc_guid) || isUnseen(c.sc_guid)) return "unseen";
  return c.actions.length ? "bound" : "none";
}

function describe(ev: JoyInput): string {
  switch (ev.kind) {
    case "button":
      return `button ${ev.index} ${ev.pressed ? "down" : "up"}`;
    case "axis":
      return `axis ${ev.index} = ${ev.value}`;
    case "hat":
      return `hat ${ev.index} → ${ev.direction}`;
  }
}

let unlisten: UnlistenFn[] = [];

onMounted(async () => {
  unlisten.push(
    await listen<JoyInput>("joy-input", (e) => {
      // The editor owns the input while a HW profile is being edited.
      if (mode.value !== "live") return;
      const p = e.payload;
      events.value.unshift(p);
      if (events.value.length > MAX_EVENTS) events.value.pop();
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
      await refresh();
      await reloadProfiles();
    }),
  );
  await refresh();

  try {
    actionMaps.value = await invoke<ActionMap[]>("get_actions");
    tokens.value = await invoke<Record<string, string>>("get_tokens");
    const cfg = await invoke<{
      base_path: string;
      ignored_devices: string[];
      profile_choices: Record<string, string>;
    }>("get_config");
    basePath.value = cfg.base_path;
    ignoredDevices.value = cfg.ignored_devices;
    profileChoices.value = cfg.profile_choices ?? {};
    bindings.value = await invoke<ResolvedBinding[]>("get_bindings");
  } catch (e) {
    error.value = String(e);
  }
  await reloadProfiles();
});

onUnmounted(() => {
  unlisten.forEach((fn) => fn());
  unlisten = [];
  axisTimers.forEach((t) => clearTimeout(t));
  axisTimers.clear();
});
</script>

<template>
  <main class="container">
    <header class="topbar">
      <h1>BindSight</h1>
      <div class="modes">
        <button :class="{ on: mode === 'live' }" @click="setMode('live')">Live</button>
        <button :class="{ on: mode === 'profiles' }" @click="setMode('profiles')">HW profiles</button>
      </div>
      <button @click="refresh" :disabled="loading">
        {{ loading ? "Scanning…" : "Refresh" }}
      </button>
    </header>

    <template v-if="mode === 'live'">
    <section class="config">
      <label class="cfg-row">
        <span>SC base path</span>
        <input v-model="basePath" placeholder="…/StarCitizen/LIVE" />
        <button @click="saveBasePath">Load</button>
      </label>
    </section>

    <p v-if="error" class="error">{{ error }}</p>
    <p v-else-if="!loading && devices.length === 0" class="empty">
      No joysticks detected. Plug in a device and hit Refresh.
    </p>

    <ul class="devices">
      <li v-for="d in devices" :key="d.index" class="device" :class="{ 'device-ignored': isIgnored(d.sc_product_guid) }">
        <div class="device-name">
          {{ d.sc_name ?? "(unknown device)" }}
          <span v-if="isIgnored(d.sc_product_guid)" class="js-ignored">
            excluded
          </span>
          <span v-else-if="slotFor(d.sc_product_guid)?.clash" class="js-clash">
            ⚠ SC → js{{ slotFor(d.sc_product_guid)?.effective_instance }}
            (binds js{{ slotFor(d.sc_product_guid)?.stored_instance }})
          </span>
          <span v-else-if="slotFor(d.sc_product_guid)?.stored_instance" class="js-mapped">
            ✓ js{{ slotFor(d.sc_product_guid)?.effective_instance }}
          </span>
          <span v-else-if="slotFor(d.sc_product_guid)" class="js-new">
            js{{ slotFor(d.sc_product_guid)?.effective_instance }} · not in profile
          </span>
          <span v-else-if="isUnseen(d.sc_product_guid)" class="js-unseen">
            not seen by SC
          </span>
        </div>
        <div class="counts">
          <span>{{ d.num_buttons }} buttons</span>
          <span>{{ d.num_axes }} axes</span>
          <span>{{ d.num_hats }} hats</span>
          <span v-if="bindingCountFor(d.sc_product_guid)" class="bound-count">
            {{ bindingCountFor(d.sc_product_guid) }} bindings
          </span>
          <span v-if="d.axes.length" class="axes-map mono" title="SC axis name per SDL axis index">{{ d.axes.join(" ") }}</span>
          <span v-else-if="d.axes_error" class="axes-error" :title="d.axes_error">axes: {{ d.axes_error }}</span>
          <button
            class="ignore-toggle"
            :disabled="!d.sc_product_guid"
            :title="isIgnored(d.sc_product_guid) ? 'Count this device as visible to SC again' : 'Treat this device as one SC never sees (e.g. hidden by Wine); it then counts as unplugged'"
            @click="toggleIgnored(d.sc_product_guid)"
          >
            {{ isIgnored(d.sc_product_guid) ? "Include" : "Exclude always" }}
          </button>
        </div>
      </li>
    </ul>

    <p v-if="hasLog" class="source-note">
      <code>Game.log</code> {{ fmtTimestamp(clash?.log_timestamp ?? null) }}
      <template v-if="clash?.unseen.length"> · {{ clash?.unseen.length }} not seen by SC</template>
      <template v-if="unpluggedSinceStart.length"> · {{ unpluggedSinceStart.length }} unplugged since</template>
    </p>
    <p v-else-if="clash?.log_error" class="order-note">
      <template v-if="clash.log_error.kind === 'not_found'">
        <code>Game.log</code> not found: <code>{{ clash.log_error.path }}</code>
      </template>
      <template v-else>
        <code>Game.log</code> lists no joysticks: <code>{{ clash.log_error.path }}</code>
      </template>
      · no device order without it
    </p>

    <section v-if="clash?.has_clash" class="clash-banner">
      <div class="clash-title">⚠ Device order clash</div>
      <p class="clash-body">
        SC assigns <code>jsN</code> by connection order, not device identity. The
        current order no longer matches your saved profile, so bindings land on
        the wrong device.
      </p>
      <ul v-if="clash.missing.length" class="clash-missing">
        <li v-for="m in clash.missing" :key="m.stored_instance">
          <strong>js{{ m.stored_instance }}</strong> — {{ m.name }} not in SC's
          device list (slots after it shift down)
        </li>
      </ul>
      <div v-if="clash.resort.length" class="resort">
        <div class="resort-title">Resort</div>
        <ul class="resort-moves">
          <li v-for="m in clash.resort" :key="m.from">
            <strong>js{{ m.from }} → js{{ m.to }}</strong>
            <template v-if="m.name"> {{ m.name }}</template>
            <template v-else> (no saved device)</template>
          </li>
        </ul>
        <div class="resort-row">
          <pre class="resort-cmds">{{ clash.resort_commands.join("\n") }}</pre>
          <button @click="copyResortCommands">Copy</button>
        </div>
        <p class="resort-hint">In-game: paste into the console. Out of game: rewrite <code>actionmaps.xml</code> (SC must be closed; a backup is kept).</p>
        <button class="resort-apply" @click="applyResort">Rewrite actionmaps.xml</button>
      </div>
    </section>

    <section class="current" :class="'state-' + liveState()" v-if="currentInput">
      <div class="cur-head">
        <span class="cur-dev">{{ currentInput.device }}</span>
        <span v-if="liveState() === 'unseen'" class="js-unverified">
          {{ isIgnored(currentInput.sc_guid) ? "excluded" : "not seen by SC" }}
        </span>
        <span class="cur-detail">{{ currentInput.token ? tokenLabel(currentInput.token) : currentInput.sdl }}</span>
      </div>
      <div v-if="currentInput.actions.length" class="cur-actions">
        <div v-for="(a, i) in currentInput.actions" :key="i">
          {{ a.label ?? a.action }}
          <span v-if="a.is_default" class="default-tag">default</span>
          <span class="cur-ctx">({{ actionmapLabel(a.actionmap) }})</span>
        </div>
      </div>
      <div v-else class="cur-none">— not bound —</div>
    </section>

    <section v-if="profileViews.length" class="hwprofiles">
      <h2>HW profiles</h2>
      <div v-for="v in profileViews" :key="v.device.index" class="hwprofile">
        <div class="hp-head">
          <span class="hp-dev">{{ v.device.sc_name ?? v.device.sdl_name }}</span>
          <select
            v-if="v.options.length > 1"
            class="hp-pick"
            :value="v.profile.id"
            @change="setProfileChoice(v.device.sc_product_guid, ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="s in v.options" :key="s.id" :value="s.id">
              {{ s.name }}{{ s.variant ? ` · ${s.variant}` : "" }}
            </option>
          </select>
          <span v-else class="hp-name">{{ v.profile.name }}</span>
        </div>
        <DeviceImage
          v-if="imgSrc(v.profile.id, v.profile.image.file)"
          :profile="v.profile"
          :src="imgSrc(v.profile.id, v.profile.image.file)"
          :active="activeFor(v.device.sdl_guid)"
        />
      </div>
    </section>

    <section class="bindings" v-if="bindings.length">
      <h2>Joystick bindings ({{ bindings.length }})</h2>
      <ul class="binding-list">
        <li
          v-for="(b, i) in bindings"
          :key="i"
          :class="{ 'binding-clash': bindingClash(b.token), 'binding-pinned': isPinned(b) }"
          @click="togglePin(b)"
        >
          <span class="tok">{{ tokenLabel(b.token) }}</span>
          <span class="blabel">
            {{ b.label ?? b.action }}
            <span v-if="b.is_default" class="default-tag">default</span>
            <span v-if="missingInProfile(b)" class="js-unverified">not in HW profile</span>
          </span>
          <span
            class="bctrl"
            :class="bindingClash(b.token) ? 'ctrl-clash' : isConnected(b) ? 'ctrl-ok' : 'ctrl-warn'"
          >
            <template v-if="bindingClash(b.token)">⚠ js{{ instanceOf(b.token) }} misassigned</template>
            <template v-else-if="isConnected(b)">✓ {{ b.device }}</template>
            <template v-else>⚠ {{ b.device ?? "unknown" }} — not connected</template>
          </span>
        </li>
      </ul>
    </section>

    <section class="live">
      <h2>Live input</h2>
      <p v-if="events.length === 0" class="empty">
        Press a button or move an axis…
      </p>
      <ol v-else class="events">
        <li v-for="(ev, i) in events" :key="i">
          <span class="ev-device">{{ nameFor(ev.guid) }}</span>
          <span class="ev-detail mono">{{ describe(ev) }}</span>
        </li>
      </ol>
    </section>

    <section class="actions">
      <h2>Actions</h2>
      <div class="action-scroll">
        <div v-for="map in actionMaps" :key="map.name" class="action-group">
          <div class="group-head">{{ map.label ?? map.name }}</div>
          <ul>
            <li v-for="a in map.actions" :key="a.name">{{ actionText(a) }}</li>
          </ul>
        </div>
      </div>
    </section>
    </template>

    <ProfileEditor v-else :devices="devices" @notify="notify" @saved="onProfilesSaved" />

    <div class="toasts">
      <div v-for="t in toasts" :key="t.id" :class="['toast', t.type]">
        {{ t.message }}
      </div>
    </div>
  </main>
</template>

<style scoped>
.container {
  max-width: 820px;
  margin: 0 auto;
  padding: 1.5rem;
}

.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}

h1 {
  margin: 0;
  font-size: 1.6rem;
}

h2 {
  font-size: 1.1rem;
  margin: 1.5rem 0 0.5rem;
}

.modes {
  display: flex;
  gap: 0.35rem;
  margin-right: auto;
  margin-left: 1rem;
}

.modes button {
  padding: 0.25em 0.8em;
  font-size: 0.85rem;
  box-shadow: none;
  border: 1px solid rgba(128, 128, 128, 0.4);
  background: transparent;
  color: inherit;
}

.modes button.on {
  border-color: #396cd8;
  background: rgba(57, 108, 216, 0.14);
}

.hwprofiles {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.hwprofile {
  border: 1px solid rgba(128, 128, 128, 0.3);
  border-radius: 10px;
  padding: 0.9rem 1rem;
  background: rgba(128, 128, 128, 0.06);
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.hp-head {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}

.hp-dev {
  font-weight: 600;
}

.hp-name {
  font-size: 0.85rem;
  opacity: 0.7;
}

.hp-pick {
  padding: 0.25em 0.5em;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.4);
  background: transparent;
  color: inherit;
  font-family: inherit;
  font-size: 0.85rem;
}

.hint {
  font-size: 0.85rem;
  opacity: 0.7;
  line-height: 1.4;
}

.error {
  color: #c0392b;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  white-space: pre-wrap;
}

.empty {
  opacity: 0.7;
}

.devices {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.device {
  border: 1px solid rgba(128, 128, 128, 0.3);
  border-radius: 10px;
  padding: 0.9rem 1rem;
  background: rgba(128, 128, 128, 0.06);
}

.device-name {
  font-weight: 600;
  margin-bottom: 0.5rem;
}

.js-mapped {
  font-size: 0.75rem;
  font-weight: 600;
  color: #2e7d32;
  background: rgba(46, 125, 50, 0.14);
  border-radius: 4px;
  padding: 0.05rem 0.35rem;
  margin-left: 0.4rem;
  white-space: nowrap;
}

.js-clash {
  font-size: 0.75rem;
  font-weight: 600;
  color: #c0392b;
  background: rgba(192, 57, 43, 0.14);
  border-radius: 4px;
  padding: 0.05rem 0.35rem;
  margin-left: 0.4rem;
  white-space: nowrap;
}

.js-new {
  font-size: 0.75rem;
  font-weight: 600;
  color: #999;
  background: rgba(128, 128, 128, 0.14);
  border-radius: 4px;
  padding: 0.05rem 0.35rem;
  margin-left: 0.4rem;
  white-space: nowrap;
}

.js-unverified {
  font-size: 0.75rem;
  font-weight: 600;
  color: #b9770e;
  background: rgba(230, 126, 34, 0.14);
  border-radius: 4px;
  padding: 0.05rem 0.35rem;
  margin-left: 0.4rem;
  white-space: nowrap;
}

.order-note {
  margin: 0.75rem 0 0;
  font-size: 0.85rem;
  color: #b9770e;
}

.order-note code {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.source-note {
  margin: 0.75rem 0 0;
  font-size: 0.85rem;
  opacity: 0.75;
}

.source-note code {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.js-unseen {
  font-size: 0.75rem;
  font-weight: 600;
  color: #7f8c8d;
  background: rgba(127, 140, 141, 0.16);
  border-radius: 4px;
  padding: 0.05rem 0.35rem;
  margin-left: 0.4rem;
  white-space: nowrap;
}

.js-ignored {
  font-size: 0.75rem;
  font-weight: 600;
  font-style: italic;
  color: #7f8c8d;
  background: rgba(127, 140, 141, 0.16);
  border-radius: 4px;
  padding: 0.05rem 0.35rem;
  margin-left: 0.4rem;
  white-space: nowrap;
}

.device-ignored {
  opacity: 0.55;
}

.ignore-toggle {
  margin-left: auto;
  padding: 0.15em 0.6em;
  font-size: 0.75rem;
  font-weight: 500;
  box-shadow: none;
  border: 1px solid rgba(128, 128, 128, 0.4);
  background: transparent;
  color: inherit;
}

.clash-banner {
  border: 1px solid #c0392b;
  border-radius: 10px;
  padding: 0.8rem 1rem;
  margin: 0.75rem 0;
  background: rgba(192, 57, 43, 0.08);
}

.clash-title {
  font-weight: 700;
  color: #c0392b;
  margin-bottom: 0.3rem;
}

.clash-body {
  margin: 0;
  font-size: 0.85rem;
  opacity: 0.85;
}

.clash-body code {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.clash-missing {
  margin: 0.5rem 0 0;
  padding-left: 1.1rem;
  font-size: 0.85rem;
}

.resort {
  margin-top: 0.75rem;
  padding-top: 0.6rem;
  border-top: 1px solid rgba(192, 57, 43, 0.3);
  font-size: 0.85rem;
}

.resort-title {
  font-weight: 700;
  margin-bottom: 0.3rem;
}

.resort-moves {
  margin: 0 0 0.5rem;
  padding-left: 1.1rem;
}

.resort-row {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
}

.resort-cmds {
  flex: 1;
  margin: 0;
  padding: 0.4rem 0.6rem;
  border-radius: 6px;
  background: rgba(128, 128, 128, 0.14);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 0.8rem;
  overflow-x: auto;
}

.resort-hint {
  margin: 0.5rem 0;
  opacity: 0.8;
}

.resort-hint code {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.binding-clash {
  background: rgba(192, 57, 43, 0.08);
  border-radius: 4px;
}

/* Clickable: lights the input on the device's profile image. */
.binding-list li {
  cursor: pointer;
}

.binding-pinned {
  background: rgba(57, 108, 216, 0.14);
  border-radius: 4px;
}

.ctrl-clash {
  color: #c0392b;
  font-weight: 600;
}

.idx {
  opacity: 0.55;
  margin-right: 0.35rem;
}

.meta {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.pair {
  display: flex;
  gap: 0.5rem;
  font-size: 0.9rem;
}

.pair dt {
  min-width: 6.5rem;
  opacity: 0.6;
}

.pair dd {
  margin: 0;
}

.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.counts {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  font-size: 0.85rem;
  opacity: 0.8;
  margin-top: 0.35rem;
}

.axes-map {
  font-size: 0.8rem;
}

.axes-error {
  font-size: 0.8rem;
  color: #b9770e;
}

.events {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  max-height: 40vh;
  overflow-y: auto;
  font-size: 0.9rem;
}

.events li {
  display: flex;
  gap: 0.75rem;
}

.ev-device {
  min-width: 14rem;
  opacity: 0.7;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.action-scroll {
  max-height: 50vh;
  overflow-y: auto;
  border: 1px solid rgba(128, 128, 128, 0.3);
  border-radius: 8px;
  padding: 0.5rem 0.75rem;
}

.action-group {
  margin-bottom: 0.75rem;
}

.group-head {
  font-weight: 600;
  font-size: 0.9rem;
  opacity: 0.85;
  margin-bottom: 0.2rem;
}

.action-group ul {
  list-style: none;
  margin: 0;
  padding: 0;
}

.action-group li {
  font-size: 0.85rem;
  padding: 0.05rem 0;
}

.config {
  margin: 0.25rem 0 0.5rem;
}

.cfg-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.9rem;
}

.cfg-row span {
  opacity: 0.7;
  white-space: nowrap;
}

.cfg-row input {
  flex: 1;
  padding: 0.4em 0.6em;
  border-radius: 6px;
  border: 1px solid rgba(128, 128, 128, 0.4);
  background: transparent;
  color: inherit;
  font-family: inherit;
}

.toasts {
  position: fixed;
  bottom: 1rem;
  right: 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  z-index: 1000;
}

.toast {
  padding: 0.6rem 0.9rem;
  border-radius: 8px;
  font-size: 0.85rem;
  color: #fff;
  max-width: 24rem;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
}

.toast.ok {
  background: #2e7d32;
}

.toast.error {
  background: #c0392b;
}

.current {
  border: 1px solid;
  border-radius: 10px;
  padding: 0.8rem 1rem;
  margin: 0.5rem 0 1rem;
}

/* blue: the input has SC bindings */
.state-bound {
  border-color: #396cd8;
  background: rgba(57, 108, 216, 0.08);
}

/* grey: SC sees the device, nothing bound */
.state-none {
  border-color: rgba(128, 128, 128, 0.4);
  background: rgba(128, 128, 128, 0.06);
}

/* yellow: SC doesn't see the device (unseen or excluded) */
.state-unseen {
  border-color: #d4a017;
  background: rgba(212, 160, 23, 0.12);
}

.cur-head {
  display: flex;
  gap: 0.6rem;
  align-items: baseline;
  margin-bottom: 0.3rem;
}

.cur-dev {
  font-weight: 600;
}

.cur-detail {
  opacity: 0.7;
  font-size: 0.85rem;
}

.cur-actions {
  font-size: 1.05rem;
}

.cur-ctx {
  opacity: 0.5;
  font-size: 0.8rem;
}

.cur-none {
  opacity: 0.5;
  font-style: italic;
}

.binding-list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 40vh;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
  font-size: 0.85rem;
}

.binding-list li {
  display: flex;
  gap: 0.75rem;
  align-items: baseline;
}

.tok {
  min-width: 8rem;
  opacity: 0.8;
}

.blabel {
  flex: 1;
}

.bctrl {
  font-size: 0.8rem;
  white-space: nowrap;
}

.ctrl-ok {
  color: #2e7d32;
}

.ctrl-warn {
  color: #e67e22;
}

.bound-count {
  color: #2e7d32;
  font-weight: 600;
}

.default-tag {
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  opacity: 0.55;
  margin-left: 0.4rem;
  border: 1px solid rgba(128, 128, 128, 0.4);
  border-radius: 3px;
  padding: 0 0.3rem;
  vertical-align: middle;
}

button {
  border-radius: 8px;
  border: 1px solid transparent;
  padding: 0.5em 1.1em;
  font-size: 0.95em;
  font-weight: 500;
  font-family: inherit;
  cursor: pointer;
  color: #0f0f0f;
  background-color: #ffffff;
  box-shadow: 0 2px 2px rgba(0, 0, 0, 0.2);
  transition: border-color 0.25s;
}

button:hover:not(:disabled) {
  border-color: #396cd8;
}

button:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>

<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  font-weight: 400;
  color: #0f0f0f;
  background-color: #f6f6f6;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

body {
  margin: 0;
}

@media (prefers-color-scheme: dark) {
  :root {
    /* Native controls (select popups, checkboxes) follow the dark theme. */
    color-scheme: dark;
    color: #f6f6f6;
    background-color: #2f2f2f;
  }

  button {
    color: #ffffff;
    background-color: #0f0f0f98;
  }
}
</style>
