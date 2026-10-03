<script setup lang="ts">
// Device Info's Axis Test: a tester plays the game while the app records the
// raw axes in the background (SDL input arrives without window focus) and
// presses a freely chosen joystick / pad button — the marker — the moment
// the game reacts. Each marker press keeps every axis's raw value next to
// what the Monitor makes of it with the configured deadzone / saturation,
// and which axes moved most since the previous marker; Save Report writes it
// all with the game's device settings verbatim. The raw stream runs only
// between Start and Stop (and never past leaving the view).
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import { DERIVED_PAD_BUTTONS, deviceName, inputIdentity, recordEdge } from "../devices";
import { PAD_RAW_AXES, axisLimits, axisOutput, deviceOfRaw, holdAxisStream, type AxisClass } from "../axisStream";
import { GAMEPAD_DEVICE, JOYSTICK_INPUTS, guidMatches } from "../configModel";
import type { AxisRaw, ClashReport, DeviceConfigView, DeviceInfo, JoyInput, ToastType } from "../types";

const props = defineProps<{
  devices: DeviceInfo[];
  // The device-order report: which jsN the game gives each joystick.
  clash: ClashReport | null;
  // The report's head: app version, OS, toolkits; game environment + version.
  systemLine: string;
  gameLine: string;
}>();
const emit = defineEmits<{ notify: [message: string, type: ToastType] }>();

// Below this travel since the previous marker an axis did not move.
const MOVED_MIN = 0.01;
const MOVED_TOP = 3;

// --- devices -----------------------------------------------------------------

const axisDevices = computed(() => props.devices.filter((d) => d.kind === "joystick" || d.kind === "gamepad"));

// The slot the game gives a device ("js2", "gp1"), else a dash.
function slotOf(d: DeviceInfo): string {
  if (d.kind === "gamepad") return d.gamepad_slot !== null ? `gp${d.gamepad_slot}` : "—";
  const guid = d.sc_product_guid?.toLowerCase();
  const s = props.clash?.connected.find((c) => !!guid && c.sc_product_guid?.toLowerCase() === guid);
  return s ? `js${s.effective_instance}` : "—";
}

// A short device tag for the entries: the slot, else SDL's index.
function tagOf(d: DeviceInfo): string {
  const s = slotOf(d);
  return s === "—" ? `#${d.index}` : s;
}

function axisNames(d: DeviceInfo): string[] {
  if (d.kind === "gamepad") return [...PAD_RAW_AXES];
  return Array.from({ length: d.num_axes }, (_, i) => d.axes[i] ?? `axis${i}`);
}

// --- marker --------------------------------------------------------------------

// Record-style: the next joystick / pad button press is the candidate, its
// release takes it (`recordEdge`). Escape cancels.
const picking = ref(false);
const marker = ref<{ id: string; text: string } | null>(null);
let candidate: { id: string; text: string } | null = null;

function markerId(ev: JoyInput): string {
  return `${ev.instance_id}#${inputIdentity(ev)}`;
}

function buttonText(ev: JoyInput): string | null {
  const d = deviceOfRaw(props.devices, ev.guid, ev.instance_id);
  if (!d) return null;
  if (ev.kind === "button") return `${deviceName(d)} · button ${ev.index}`;
  if (ev.kind === "padbutton" && !DERIVED_PAD_BUTTONS.has(ev.name)) return `${deviceName(d)} · pad ${ev.name}`;
  return null;
}

function startPick() {
  candidate = null;
  picking.value = true;
}

function takeInput(ev: JoyInput) {
  if (ev.kind !== "button" && ev.kind !== "padbutton") return;
  const edge = recordEdge(ev);
  const id = markerId(ev);
  if (picking.value) {
    if (edge === "press") {
      const text = buttonText(ev);
      if (text) candidate = { id, text };
    } else if (edge === "release" && candidate?.id === id) {
      marker.value = candidate;
      candidate = null;
      picking.value = false;
    }
    return;
  }
  if (running.value && edge === "press" && marker.value?.id === id) addEntry();
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && picking.value) {
    picking.value = false;
    candidate = null;
  }
}

// --- recording -------------------------------------------------------------------

interface Sample {
  axis: string;
  raw: number;
  out: number;
}

interface Entry {
  n: number;
  // Seconds since Start.
  t: number;
  devices: { tag: string; samples: Sample[] }[];
  moved: string[];
}

const running = ref(false);
const entries = ref<Entry[]>([]);
const startedAt = ref<Date | null>(null);
let startMs = 0;
// The game's device settings as of Start (display units); null = unknown,
// the fixed zones then.
let config: DeviceConfigView | null = null;
// Per device ("guid#instance"): the latest raw values; per axis
// ("guid#instance#i"): its value at the previous marker and the furthest it
// travelled from there since.
let latest: Record<string, number[]> = {};
let anchor: Record<string, number> = {};
let travel: Record<string, number> = {};
let releaseStream: (() => void) | null = null;

const devKey = (guid: string, instance: number) => `${guid}#${instance}`;

function takeRaw(p: AxisRaw) {
  if (!running.value) return;
  const k = devKey(p.guid, p.instance_id);
  latest[k] = p.values;
  p.values.forEach((v, i) => {
    const a = `${k}#${i}`;
    if (anchor[a] === undefined) anchor[a] = v;
    travel[a] = Math.max(travel[a] ?? 0, Math.abs(v - anchor[a]));
  });
}

// The configured deadzone / saturation of an axis, display units.
function configured(d: DeviceInfo, axis: string): { deadzone: number | null; saturation: number | null } {
  const none = { deadzone: null, saturation: null };
  if (!config) return none;
  if (d.kind === "gamepad") {
    const stick = /^(thumb[lr])[xy]$/.exec(axis)?.[1];
    const o = stick ? config.device_options.find((x) => x.name === GAMEPAD_DEVICE)?.axes.find((a) => a.input === stick) : undefined;
    return { deadzone: o?.deadzone ?? null, saturation: null };
  }
  if (!(JOYSTICK_INPUTS as readonly string[]).includes(axis)) return none;
  const o = config.device_options.find((x) => guidMatches(x.name, d.sc_product_guid))?.axes.find((a) => a.input === axis);
  return { deadzone: o?.deadzone ?? null, saturation: o?.saturation ?? null };
}

function axisClass(d: DeviceInfo, axis: string): AxisClass {
  if (d.kind !== "gamepad") return "joystick";
  return axis.startsWith("trigger") ? "trigger" : "thumb";
}

// What the Monitor makes of an axis's raw value: the configured (stored)
// zone and saturation, else the fixed zone.
function output(d: DeviceInfo, axis: string, raw: number): number {
  const c = configured(d, axis);
  return axisOutput(axisClass(d, axis), raw, c.deadzone, c.saturation);
}

// The report's line on what an axis's output was computed with: the
// configured value (display -> stored), else the fixed zone; no saturation
// is none.
function limitsText(d: DeviceInfo, axis: string): string {
  const c = configured(d, axis);
  const l = axisLimits(axisClass(d, axis), c.deadzone, c.saturation);
  const zone = l.fixed ? `fixed ${l.zone.toFixed(4)}` : `${c.deadzone!.toFixed(2)} -> ${l.zone.toFixed(4)}`;
  const sat = l.saturation === null ? "none" : `${c.saturation!.toFixed(2)} -> ${l.saturation.toFixed(4)}`;
  return `${axis} deadzone ${zone} · saturation ${sat}`;
}

function addEntry() {
  const moves: { axis: string; travel: number }[] = [];
  const devices = axisDevices.value.map((d) => {
    const k = devKey(d.sdl_guid, d.sdl_instance_id);
    const values = latest[k] ?? [];
    const names = axisNames(d);
    const tag = tagOf(d);
    const samples = values.map((raw, i): Sample => {
      const axis = names[i] ?? `axis${i}`;
      const a = `${k}#${i}`;
      moves.push({ axis: `${tag} ${axis}`, travel: travel[a] ?? 0 });
      anchor[a] = raw;
      travel[a] = 0;
      return { axis, raw, out: output(d, axis, raw) };
    });
    return { tag, samples };
  });
  const moved = moves
    .filter((m) => m.travel >= MOVED_MIN)
    .sort((a, b) => b.travel - a.travel)
    .slice(0, MOVED_TOP)
    .map((m) => m.axis);
  const n = entries.value.length + 1;
  entries.value = [{ n, t: (performance.now() - startMs) / 1000, devices, moved }, ...entries.value];
}

let starting = false;

async function start() {
  if (running.value || starting || !marker.value) return;
  starting = true;
  try {
    config = await invoke<DeviceConfigView>("get_device_config", { source: { kind: "current" } });
  } catch (e) {
    config = null;
    emit("notify", `Settings unavailable: ${e}`, "error");
  } finally {
    starting = false;
  }
  // Left the view while the settings were being read.
  if (unmounted) return;
  entries.value = [];
  latest = {};
  anchor = {};
  travel = {};
  startMs = performance.now();
  startedAt.value = new Date();
  running.value = true;
  releaseStream = holdAxisStream();
}

function stop() {
  running.value = false;
  releaseStream?.();
  releaseStream = null;
}

// The entries go; a running test keeps recording, numbered from 1 again.
function clear() {
  entries.value = [];
}

// --- report ----------------------------------------------------------------------

const num = (v: number) => `${v >= 0 ? "+" : ""}${v.toFixed(3)}`;

function sampleText(s: Sample): string {
  return `${s.axis} ${num(s.raw)} ${num(s.out)}`;
}

async function reportText(): Promise<string> {
  const lines = [`BindSight axis test ${new Date().toISOString()}`, props.systemLine, props.gameLine];
  lines.push(`started ${startedAt.value?.toISOString() ?? "—"} · marker ${marker.value?.text ?? "—"} · ${entries.value.length} entries`, "");
  lines.push("devices");
  for (const d of axisDevices.value) {
    lines.push(`    ${slotOf(d).padEnd(4)} ${deviceName(d)} · hardware id ${d.hardware_id ?? "—"} · axes ${axisNames(d).join(" ")}`);
  }
  if (!axisDevices.value.length) lines.push("    none");
  lines.push("", "game settings");
  try {
    lines.push(await invoke<string>("device_config_text"));
  } catch (e) {
    console.error("device config text failed", e);
    lines.push(`game settings error: ${e}`);
  }
  lines.push("", "deadzone / saturation used (display -> stored; fixed = the app's own resting zone, nothing configured)");
  for (const d of axisDevices.value) {
    const names = axisNames(d);
    lines.push(`    ${tagOf(d).padEnd(4)} ${names.length ? limitsText(d, names[0]) : "no axes"}`);
    for (const a of names.slice(1)) lines.push(`         ${limitsText(d, a)}`);
  }
  if (!axisDevices.value.length) lines.push("    none");
  lines.push("", "entries (oldest first; per axis: raw, output with the configured deadzone / saturation)");
  for (const e of [...entries.value].reverse()) {
    lines.push(`#${e.n} ${e.t.toFixed(3)} s · moved ${e.moved.join(", ") || "—"}`);
    for (const d of e.devices) lines.push(`    ${d.tag.padEnd(4)} ${d.samples.map(sampleText).join(" · ") || "no data"}`);
  }
  if (!entries.value.length) lines.push("    none");
  return lines.join("\n") + "\n";
}

async function saveReport() {
  try {
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const dest = await save({
      defaultPath: `bindsight-axis-test-${stamp}.txt`,
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (!dest) return;
    await invoke("write_text_file", { path: dest, text: await reportText() });
    emit("notify", "Axis test saved", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// --- wiring ----------------------------------------------------------------------

let unlisten: UnlistenFn[] = [];
let unmounted = false;

onMounted(async () => {
  window.addEventListener("keydown", onKey);
  const fns = [
    await listen<JoyInput>("joy-input", (e) => takeInput(e.payload)),
    await listen<AxisRaw>("axis-raw", (e) => takeRaw(e.payload)),
  ];
  // Left again while the listeners were being set up.
  if (unmounted) fns.forEach((fn) => fn());
  else unlisten = fns;
});

onUnmounted(() => {
  unmounted = true;
  window.removeEventListener("keydown", onKey);
  unlisten.forEach((fn) => fn());
  unlisten = [];
  stop();
});
</script>

<template>
  <section class="panel log-tile">
    <div class="head">
      <Icon name="axis" :size="15" />
      <span class="head-title">Axis Test</span>
      <span class="head-count">{{ entries.length }}</span>
      <div class="grow" />
      <span class="marker mono" :class="{ unset: !marker && !picking }">
        {{ picking ? "Press a button" : (marker?.text ?? "No marker") }}
      </span>
      <button
        type="button"
        class="btn small"
        :class="picking ? 'primary' : 'outline'"
        :disabled="running"
        @click="startPick"
      >
        <Icon name="target" :size="13" />
        Marker
      </button>
      <button v-if="!running" type="button" class="btn primary small" :disabled="!marker || picking" @click="start">
        <Icon name="bolt" :size="13" />
        Start
      </button>
      <button v-else type="button" class="btn danger small" @click="stop">
        <Icon name="close" :size="13" />
        Stop
      </button>
      <button type="button" class="btn outline small" :disabled="!entries.length" @click="clear">
        <Icon name="trash" :size="13" />
        Clear
      </button>
      <button type="button" class="btn primary small" :disabled="!entries.length" @click="saveReport">
        <Icon name="save" :size="13" />
        Save
      </button>
    </div>
    <div class="log mono">
      <div v-for="e in entries" :key="e.n" class="log-dev">
        <div class="log-line">
          <span class="log-key">#{{ e.n }}</span>
          <span class="log-time">{{ e.t.toFixed(3) }} s</span>
          <span class="log-moved">{{ e.moved.join(", ") || "—" }}</span>
        </div>
        <div v-for="(d, i) in e.devices" :key="i" class="log-kv">
          <span class="log-dim">{{ d.tag }}</span>
          <span class="log-val">{{ d.samples.map(sampleText).join(" · ") || "no data" }}</span>
        </div>
      </div>
      <div v-if="!entries.length" class="log-line log-dim">{{ running ? "Recording" : "None" }}</div>
    </div>
  </section>
</template>

<style scoped>
.panel {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.log-tile {
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
  flex: none;
  font-weight: 600;
  font-size: 14px;
}

.head-count {
  font-size: 12px;
  color: var(--text-2);
}

.grow {
  flex: 1;
}

/* The marker's text: fixed width, so the head never shifts. */
.marker {
  width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: right;
  font-size: 12px;
  color: var(--accent);
}

.marker.unset {
  color: var(--text-3);
}

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

.log-dev {
  margin-bottom: 10px;
}

.log-key {
  color: var(--live);
  min-width: 3rem;
}

.log-time {
  color: var(--text-2);
}

.log-moved {
  color: var(--accent);
}

.log-kv {
  display: grid;
  grid-template-columns: 4rem minmax(0, 1fr);
  gap: 10px;
  padding: 1px 0 1px 1rem;
}

.log-val {
  overflow-wrap: anywhere;
}

.log-dim {
  color: var(--text-3);
}
</style>
