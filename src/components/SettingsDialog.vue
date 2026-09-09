<script setup lang="ts">
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import type { DeviceInfo } from "../types";

const props = defineProps<{ basePath: string; devices: DeviceInfo[]; ignored: string[] }>();
const emit = defineEmits<{
  close: [];
  save: [settings: { basePath: string; ignored: string[] }];
  notify: [message: string, type: "ok" | "error"];
}>();

// Local copies: nothing is applied before Save.
const path = ref(props.basePath);
const excluded = ref<string[]>([...props.ignored]);
const addPick = ref("");

const withGuid = computed(() => props.devices.filter((d): d is DeviceInfo & { sc_product_guid: string } => !!d.sc_product_guid));
const isExcluded = (guid: string) => excluded.value.some((g) => g.toLowerCase() === guid.toLowerCase());
const addable = computed(() => withGuid.value.filter((d) => !isExcluded(d.sc_product_guid)));

function nameFor(guid: string): string {
  const d = withGuid.value.find((x) => x.sc_product_guid.toLowerCase() === guid.toLowerCase());
  return d ? (d.sc_name ?? d.sdl_name) : guid;
}

function remove(guid: string) {
  excluded.value = excluded.value.filter((g) => g.toLowerCase() !== guid.toLowerCase());
}

function add() {
  if (addPick.value && !isExcluded(addPick.value)) excluded.value = [...excluded.value, addPick.value];
  addPick.value = "";
}

// Immediate, not part of Save: opens the folder in the file manager.
async function openLogDir() {
  try {
    await invoke("open_log_dir");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function browse() {
  const dir = await open({ directory: true, multiple: false, defaultPath: path.value || undefined });
  if (typeof dir === "string") path.value = dir;
}
</script>

<template>
  <div class="backdrop" @click.self="emit('close')">
    <div class="dialog" role="dialog" aria-label="Settings">
      <div class="head">
        <span class="title">Settings</span>
        <button type="button" class="icon-btn" title="Close" @click="emit('close')">
          <Icon name="close" :size="18" />
        </button>
      </div>

      <div class="body">
        <section>
          <div class="panel-title">Star Citizen install</div>
          <div class="row">
            <input v-model="path" class="input mono" placeholder="…/StarCitizen/LIVE" />
            <button type="button" class="btn outline" @click="browse">Browse</button>
          </div>
          <div class="hint">Folder that holds Data.p4k</div>
        </section>

        <section>
          <div class="panel-title">Action labels</div>
          <label class="check"><input type="radio" checked disabled /> Bundled</label>
          <label class="check dim" title="Not yet available"><input type="radio" disabled /> global.ini from disk</label>
        </section>

        <section>
          <div class="panel-title">Excluded devices</div>
          <div class="chips">
            <span v-for="g in excluded" :key="g" class="chip">
              {{ nameFor(g) }}
              <button type="button" class="icon-btn small" title="Remove" @click="remove(g)">
                <Icon name="close" :size="12" />
              </button>
            </span>
            <select v-if="addable.length" v-model="addPick" class="add" @change="add">
              <option value="" disabled>Add device</option>
              <option v-for="d in addable" :key="d.index" :value="d.sc_product_guid">{{ d.sc_name ?? d.sdl_name }}</option>
            </select>
          </div>
        </section>

        <section>
          <div class="panel-title">Log</div>
          <div class="row">
            <button type="button" class="btn outline" @click="openLogDir">Open log folder</button>
          </div>
        </section>
      </div>

      <div class="foot">
        <button type="button" class="btn outline" @click="emit('close')">Cancel</button>
        <button type="button" class="btn primary" @click="emit('save', { basePath: path, ignored: excluded })">Save</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  background: rgba(4, 12, 18, 0.85);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}

.dialog {
  width: 620px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.head,
.foot {
  display: flex;
  align-items: center;
  padding: 16px 24px;
}

.head {
  border-bottom: 1px solid var(--border-dim);
}

.foot {
  justify-content: flex-end;
  gap: 8px;
  border-top: 1px solid var(--border-dim);
}

.title {
  flex: 1;
  font-weight: 600;
  font-size: 18px;
}

.body {
  padding: 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 22px;
}

section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.row {
  display: flex;
  gap: 8px;
}

.input {
  flex: 1;
  height: 40px;
  box-sizing: border-box;
  padding: 0 12px;
  border: none;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.hint {
  font-size: 12px;
  color: var(--text-3);
}

.check {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
}

.check.dim {
  color: var(--text-3);
}

.chips {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}

.chip {
  height: var(--h-chip-sm);
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 0 4px 0 10px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  font-size: 13px;
}

.add {
  height: var(--h-chip-sm);
  padding: 0 10px;
  border-radius: var(--radius-control);
  border: 1px dashed rgba(173, 211, 235, 0.4);
  background: transparent;
  color: var(--text-2);
  font-family: inherit;
  font-size: 13px;
}

.btn {
  height: var(--h-control);
  display: flex;
  align-items: center;
  padding: 0 16px;
  border-radius: var(--radius-control);
  font-family: inherit;
  font-weight: 600;
  font-size: 14px;
  cursor: pointer;
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

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}

.icon-btn.small {
  width: 20px;
  height: 20px;
}

.icon-btn:hover {
  color: var(--text);
}
</style>
