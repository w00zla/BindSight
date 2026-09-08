<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
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
  category: string | null;
  actions: Action[];
}

interface ResolvedBinding {
  token: string;
  device: string | null;
  actionmap: string;
  action: string;
  label: string | null;
}

interface LoadStatus {
  base_path: string;
  actionmaps_path: string;
  loaded: boolean;
  error: string | null;
  bindings: ResolvedBinding[];
}

const MAX_EVENTS = 50;

const devices = ref<DeviceInfo[]>([]);
const events = ref<JoyInput[]>([]);
const actionMaps = ref<ActionMap[]>([]);
const basePath = ref("");
const bindings = ref<ResolvedBinding[]>([]);
const error = ref<string | null>(null);
const loading = ref(false);

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
}

function nameFor(guid: string): string {
  const d = devices.value.find((dev) => dev.sdl_guid === guid);
  return d?.sc_name ?? guid;
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
      events.value.unshift(e.payload);
      if (events.value.length > MAX_EVENTS) events.value.pop();
    }),
  );
  unlisten.push(await listen("devices-changed", () => refresh()));
  await refresh();

  try {
    actionMaps.value = await invoke<ActionMap[]>("get_actions");
    basePath.value = (await invoke<{ base_path: string }>("get_config")).base_path;
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

    <p class="hint">
      Devices as seen by SDL. The index is SDL's enumeration order, which is
      <em>not</em> the same as SC's <code>jsN</code> instance number — the SC
      Product GUID is the stable link between them.
    </p>

    <p v-if="error" class="error">{{ error }}</p>
    <p v-else-if="!loading && devices.length === 0" class="empty">
      No joysticks detected. Plug in a device and hit Refresh.
    </p>

    <ul class="devices">
      <li v-for="d in devices" :key="d.index" class="device">
        <div class="device-name">
          <span class="idx">#{{ d.index }}</span>
          {{ d.sc_name ?? "(unknown device)" }}
        </div>
        <dl class="meta">
          <div class="pair">
            <dt>SDL GUID</dt>
            <dd class="mono">{{ d.sdl_guid }}</dd>
          </div>
          <div class="pair">
            <dt>SC Product</dt>
            <dd class="mono">{{ d.sc_product_guid ?? "—" }}</dd>
          </div>
          <div class="counts">
            <span>{{ d.num_buttons }} buttons</span>
            <span>{{ d.num_axes }} axes</span>
            <span>{{ d.num_hats }} hats</span>
          </div>
        </dl>
      </li>
    </ul>

    <section class="bindings" v-if="bindings.length">
      <h2>Joystick bindings ({{ bindings.length }})</h2>
      <ul class="binding-list">
        <li v-for="(b, i) in bindings" :key="i">
          <span class="tok mono">{{ b.token }}</span>
          <span class="blabel">{{ b.label ?? b.action }}</span>
          <span class="bdev">{{ b.device ?? "?" }}</span>
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

.bdev {
  opacity: 0.5;
  font-size: 0.8rem;
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
