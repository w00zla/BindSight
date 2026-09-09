<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface DeviceInfo {
  index: number;
  sc_name: string | null;
  sdl_name: string;
  sdl_guid: string;
  sc_product_guid: string | null;
  num_buttons: number;
  num_axes: number;
  num_hats: number;
}

type JoyInput =
  | { kind: "button"; guid: string; index: number; pressed: boolean }
  | { kind: "axis"; guid: string; index: number; value: number }
  | { kind: "hat"; guid: string; index: number; direction: string };

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

type OrderSource =
  | { kind: "game_log"; timestamp: string | null }
  | { kind: "sdl_derived"; log_error: GameLogError | null };

interface ClashReport {
  connected: SlotStatus[];
  missing: MissingSlot[];
  unseen: UnseenDevice[];
  source: OrderSource;
  // Whether the SC order is trustworthy (always for Game.log; for the SDL
  // fallback only where the platform rule is verified). When false,
  // effective_instance is a guess and no rank clash is asserted.
  order_verified: boolean;
  has_clash: boolean;
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
} | null>(null);
const clash = ref<ClashReport | null>(null);
// SC Product GUIDs the user marked "SC doesn't see this device" (persisted per OS).
const ignoredDevices = ref<string[]>([]);
const error = ref<string | null>(null);
const loading = ref(false);

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

// The Game.log source, when that is where the order came from.
const logSource = computed(() => (clash.value?.source.kind === "game_log" ? clash.value.source : null));

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

// Is this binding's slot misdirected by the current device-order clash? Only
// asserted when the derived SC order is verified on this platform — a guessed
// order must not accuse bindings; missing devices still show as "not connected".
function bindingClash(token: string): boolean {
  if (!clash.value?.has_clash || !clash.value.order_verified) return false;
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

// Resolve a live button/hat input to its token and bound action(s) and show it.
async function showBinding(
  guid: string,
  kind: "button" | "hat",
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
    currentInput.value = {
      device: nameFor(guid),
      sc_guid: devices.value.find((d) => d.sdl_guid === guid)?.sc_product_guid ?? null,
      token: res.token,
      sdl: sdlInputName(kind, index, direction),
      actions: res.actions,
    };
  } catch {
    /* ignore transient resolve errors */
  }
}

interface Toast {
  id: number;
  message: string;
  type: "ok" | "error";
}

const toasts = ref<Toast[]>([]);
let toastSeq = 0;

// Show a transient toast that dismisses itself after a few seconds.
function notify(message: string, type: "ok" | "error" = "ok") {
  const id = ++toastSeq;
  toasts.value.push({ id, message, type });
  setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }, 4000);
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

// SDL-side name of a live input, e.g. "button 5" or "hat 0 up".
function sdlInputName(kind: "button" | "hat", index: number, direction: string | null): string {
  return kind === "button" ? `button ${index}` : `hat ${index} ${direction ?? ""}`.trim();
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
      const p = e.payload;
      events.value.unshift(p);
      if (events.value.length > MAX_EVENTS) events.value.pop();

      if (p.kind === "button" && p.pressed) {
        showBinding(p.guid, "button", p.index, null);
      } else if (p.kind === "hat" && p.direction !== "centered") {
        showBinding(p.guid, "hat", p.index, p.direction);
      }
    }),
  );
  unlisten.push(await listen("devices-changed", () => refresh()));
  await refresh();

  try {
    actionMaps.value = await invoke<ActionMap[]>("get_actions");
    tokens.value = await invoke<Record<string, string>>("get_tokens");
    const cfg = await invoke<{ base_path: string; ignored_devices: string[] }>("get_config");
    basePath.value = cfg.base_path;
    ignoredDevices.value = cfg.ignored_devices;
    bindings.value = await invoke<ResolvedBinding[]>("get_bindings");
  } catch (e) {
    error.value = String(e);
  }
});

onUnmounted(() => {
  unlisten.forEach((fn) => fn());
  unlisten = [];
});
</script>

<template>
  <main class="container">
    <header class="topbar">
      <h1>BindSight</h1>
      <button @click="refresh" :disabled="loading">
        {{ loading ? "Scanning…" : "Refresh" }}
      </button>
    </header>

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
          <span v-else-if="slotFor(d.sc_product_guid) && !clash?.order_verified" class="js-unverified">
            order unknown
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

    <p v-if="logSource" class="source-note">
      <code>Game.log</code> {{ fmtTimestamp(logSource.timestamp) }}
      <template v-if="clash?.unseen.length"> · {{ clash?.unseen.length }} not seen by SC</template>
      <template v-if="unpluggedSinceStart.length"> · {{ unpluggedSinceStart.length }} unplugged since</template>
    </p>
    <p v-else-if="clash && clash.source.kind === 'sdl_derived' && devices.length" class="order-note">
      <template v-if="clash.source.log_error?.kind === 'not_found'">
        <code>Game.log</code> not found: <code>{{ clash.source.log_error.path }}</code>
      </template>
      <template v-else-if="clash.source.log_error?.kind === 'no_device_lines'">
        <code>Game.log</code> lists no joysticks: <code>{{ clash.source.log_error.path }}</code>
      </template>
      <template v-else><code>Game.log</code> not consulted</template>
      <template v-if="!clash.order_verified"> · order unknown</template>
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

    <section class="bindings" v-if="bindings.length">
      <h2>Joystick bindings ({{ bindings.length }})</h2>
      <ul class="binding-list">
        <li v-for="(b, i) in bindings" :key="i" :class="{ 'binding-clash': bindingClash(b.token) }">
          <span class="tok">{{ tokenLabel(b.token) }}</span>
          <span class="blabel">
            {{ b.label ?? b.action }}
            <span v-if="b.is_default" class="default-tag">default</span>
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

.binding-clash {
  background: rgba(192, 57, 43, 0.08);
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
  gap: 0.75rem;
  font-size: 0.85rem;
  opacity: 0.8;
  margin-top: 0.35rem;
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
    color: #f6f6f6;
    background-color: #2f2f2f;
  }

  button {
    color: #ffffff;
    background-color: #0f0f0f98;
  }
}
</style>
