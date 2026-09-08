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

const MAX_EVENTS = 50;

const devices = ref<DeviceInfo[]>([]);
const events = ref<JoyInput[]>([]);
const error = ref<string | null>(null);
const loading = ref(false);

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
