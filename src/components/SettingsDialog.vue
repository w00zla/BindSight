<script setup lang="ts">
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import Dropdown from "./Dropdown.vue";
import { ENVIRONMENTS, type DeviceInfo, type Environment } from "../types";
import { deviceName } from "../devices";

const props = defineProps<{ environments: Record<string, Environment>; devices: DeviceInfo[]; ignored: string[] }>();
const emit = defineEmits<{
  close: [];
  save: [settings: { environments: Record<string, Environment>; ignored: string[] }];
  notify: [message: string, type: "ok" | "error"];
}>();

// Local copies: nothing is applied before Save.
const envs = ref<Record<string, Environment>>(
  Object.fromEntries(
    ENVIRONMENTS.map((slug) => [
      slug,
      { ...(props.environments[slug] ?? { path: "", global_ini_override: false, global_ini: "" }) },
    ]),
  ),
);
const excluded = ref<string[]>([...props.ignored]);
const withGuid = computed(() => props.devices.filter((d): d is DeviceInfo & { sc_product_guid: string } => !!d.sc_product_guid));
const isExcluded = (guid: string) => excluded.value.some((g) => g.toLowerCase() === guid.toLowerCase());
const addable = computed(() => withGuid.value.filter((d) => !isExcluded(d.sc_product_guid)));

function nameFor(guid: string): string {
  const d = withGuid.value.find((x) => x.sc_product_guid.toLowerCase() === guid.toLowerCase());
  return d ? deviceName(d) : guid;
}

function remove(guid: string) {
  excluded.value = excluded.value.filter((g) => g.toLowerCase() !== guid.toLowerCase());
}

function add(guid: string) {
  if (guid && !isExcluded(guid)) excluded.value = [...excluded.value, guid];
}

// Immediate, not part of Save: opens the folder in the file manager.
async function openLogDir() {
  try {
    await invoke("open_log_dir");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

async function browse(slug: string) {
  const env = envs.value[slug];
  const dir = await open({ directory: true, multiple: false, defaultPath: env.path || undefined });
  if (typeof dir === "string") env.path = dir;
}

async function browseIni(slug: string) {
  const env = envs.value[slug];
  const file = await open({
    multiple: false,
    defaultPath: env.global_ini || undefined,
    filters: [{ name: "global.ini", extensions: ["ini"] }],
  });
  if (typeof file === "string") env.global_ini = file;
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
          <div class="panel-title">Star Citizen Environments</div>
          <div v-for="slug in ENVIRONMENTS" :key="slug" class="env">
            <div class="env-slug mono">{{ slug }}</div>
            <div class="env-fields">
              <div class="row">
                <input v-model="envs[slug].path" class="input mono" :placeholder="`…/StarCitizen/${slug}`" />
                <button type="button" class="btn outline" @click="browse(slug)">Browse</button>
              </div>
              <div class="row">
                <label class="check"><input v-model="envs[slug].global_ini_override" type="checkbox" /> Override global.ini</label>
                <input
                  v-model="envs[slug].global_ini"
                  class="input mono"
                  :disabled="!envs[slug].global_ini_override"
                  placeholder="…/global.ini"
                />
                <button type="button" class="btn outline" :disabled="!envs[slug].global_ini_override" @click="browseIni(slug)">
                  Browse
                </button>
              </div>
            </div>
          </div>
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
            <Dropdown
              v-if="addable.length"
              variant="dashed"
              modelValue=""
              placeholder="Add device"
              :options="addable.map((d) => ({ value: d.sc_product_guid, label: deviceName(d) }))"
              @update:modelValue="add"
            />
          </div>
        </section>

        <section>
          <div class="panel-title">Other</div>
          <div class="row">
            <button type="button" class="btn outline" @click="openLogDir">Open log folder</button>
          </div>
        </section>
      </div>

      <div class="foot">
        <button type="button" class="btn outline" @click="emit('close')">Cancel</button>
        <button type="button" class="btn primary" @click="emit('save', { environments: envs, ignored: excluded })">Save</button>
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
  max-height: 75vh;
  overflow-y: auto;
}

.env {
  display: flex;
  gap: 12px;
  align-items: flex-start;
}

.env-slug {
  width: 64px;
  padding-top: 11px;
  font-weight: 700;
  font-size: 13px;
  letter-spacing: 0.08em;
  color: var(--text);
}

.env-fields {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.input:disabled {
  opacity: 0.4;
}

.check {
  white-space: nowrap;
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

.check {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
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
