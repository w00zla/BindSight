<script setup lang="ts">
// The Profiles panel of the Bindings and the Config mode's left column: the
// game's exported keybinding layouts, Save Profile (the live file as a new
// profile, named in a small dialog), Import, Export. The list and the busy
// flag are the caller's (`v-model`); so are the selection and what a pick
// or an import means.
import { nextTick, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import Icon from "./Icon.vue";
import ConfirmDialog from "./ConfirmDialog.vue";
import { stamp } from "./BackupsPanel.vue";
import { NAME_MAX, sanitizeName, stripNameChars } from "../names";
import type { BindingProfileSummary } from "../types";

const props = defineProps<{
  // The profile shown as picked (its file); null = none.
  picked: string | null;
  // The profile Export writes (its file); null disables Export.
  target: string | null;
  // The live file is loaded: there is something to save.
  canSave: boolean;
  // Runs after an import, the list already reloaded, before the toast.
  afterImport?: (s: BindingProfileSummary) => Promise<void> | void;
}>();
const emit = defineEmits<{
  notify: [message: string, type: "ok" | "error"];
  pick: [file: string];
}>();
const profiles = defineModel<BindingProfileSummary[]>("profiles", { required: true });
const busy = defineModel<boolean>("busy", { required: true });

async function load() {
  try {
    profiles.value = await invoke<BindingProfileSummary[]>("list_binding_profiles");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// Import writes into the game's controls/mappings folder: asked first.
const importAsk = ref<{ name: string; resolve: (ok: boolean) => void } | null>(null);

function confirmImport(name: string): Promise<boolean> {
  return new Promise((resolve) => {
    importAsk.value = { name, resolve };
  });
}

function onImportChoose(value: string) {
  const a = importAsk.value;
  importAsk.value = null;
  a?.resolve(value === "import");
}

async function importProfile() {
  busy.value = true;
  try {
    const src = await open({ multiple: false, filters: [{ name: "Profile", extensions: ["xml"] }] });
    if (!src) return;
    const name = src.split(/[\\/]/).pop() ?? src;
    if (!(await confirmImport(name))) return;
    const s = await invoke<BindingProfileSummary>("import_binding_profile", { sourcePath: src });
    await load();
    await props.afterImport?.(s);
    emit("notify", `Imported ${s.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

async function exportProfile() {
  const m = profiles.value.find((p) => p.file === props.target);
  if (!m) return;
  busy.value = true;
  try {
    const dest = await save({ defaultPath: m.file, filters: [{ name: "Profile", extensions: ["xml"] }] });
    if (!dest) return;
    await invoke("export_binding_profile", { file: m.file, destPath: dest });
    emit("notify", `Exported ${m.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

// Save the live file as a new profile: the name is asked in a small dialog
// (letters, digits, space, _ - and brackets, like image-maps).
const nameDialog = ref<{ name: string } | null>(null);
const nameInput = ref<HTMLInputElement | null>(null);

function openSave() {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  nameDialog.value = { name: `Bindings ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}` };
  nextTick(() => {
    nameInput.value?.focus();
    nameInput.value?.select();
  });
}

async function onNameChoose(value: string) {
  const d = nameDialog.value;
  nameDialog.value = null;
  if (!d || value !== "save") return;
  busy.value = true;
  try {
    const s = await invoke<BindingProfileSummary>("save_binding_profile", { name: sanitizeName(d.name, "") });
    await load();
    emit("notify", `Saved ${s.name}`, "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

defineExpose({ load, openSave });

onMounted(load);
</script>

<template>
  <section class="panel">
    <div class="head">
      <Icon name="file" :size="15" />
      <span class="head-title">Profiles</span>
      <button type="button" class="btn primary small" :disabled="busy || !canSave" @click="openSave">
        <Icon name="plus" :size="12" />Save Profile
      </button>
    </div>
    <div class="rows scroll">
      <div v-for="m in profiles" :key="m.file" class="row-item" :class="{ b: picked === m.file }" @click="emit('pick', m.file)">
        <div class="lines">
          <span class="line-title">{{ m.name }}</span>
          <span class="mono line-sub">{{ m.file }} · {{ stamp(m.modified) }}</span>
        </div>
      </div>
      <div v-if="!profiles.length" class="row-none">None</div>
    </div>
    <div class="foot">
      <button type="button" class="btn outline" :disabled="busy" @click="importProfile">
        <Icon name="download" :size="14" />Import
      </button>
      <button type="button" class="btn outline" :disabled="busy || !target" @click="exportProfile">
        <Icon name="upload" :size="14" />Export
      </button>
    </div>

    <ConfirmDialog
      v-if="importAsk"
      :title="`Import ${importAsk.name}?`"
      icon="download"
      :buttons="[
        { label: 'Import', kind: 'primary', value: 'import' },
        { label: 'Cancel', kind: 'outline', value: 'cancel' },
      ]"
      @choose="onImportChoose"
    />

    <!-- save the live file as a profile -->
    <ConfirmDialog
      v-if="nameDialog"
      title="Save Profile"
      icon="file"
      :buttons="[
        { label: 'Save', kind: 'primary', value: 'save', disabled: !sanitizeName(nameDialog.name, '') },
        { label: 'Cancel', kind: 'outline', value: 'cancel' },
      ]"
      @choose="onNameChoose"
    >
      <input
        ref="nameInput"
        class="name-in"
        :value="nameDialog.name"
        :maxlength="NAME_MAX"
        spellcheck="false"
        placeholder="Name"
        @input="nameDialog.name = stripNameChars(($event.target as HTMLInputElement).value)"
        @keydown.enter="sanitizeName(nameDialog.name, '') && onNameChoose('save')"
      />
    </ConfirmDialog>
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

.head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border-dim);
}

.head-title {
  flex: 1;
  font-weight: 600;
  font-size: 14px;
}

/* Row lists styled like the Devices mode's image-map rows: outlined items
   with a little air between them. */
.rows {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
}

.rows.scroll {
  overflow-y: auto;
  min-height: 0;
  flex: 1;
}

.row-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 6px;
  border: 1px solid var(--border);
  color: var(--text-2);
  cursor: pointer;
}

.row-item.b {
  border-color: var(--accent);
}

.lines {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.line-title {
  font-weight: 600;
  font-size: 13px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.line-sub {
  font-size: 11px;
  color: var(--text-2);
}

.row-none {
  padding: 9px 10px;
  font-size: 13px;
  color: var(--text-3);
}

.foot {
  display: flex;
  gap: 6px;
  padding: 6px 12px 12px;
}

.foot .btn {
  flex: 1;
  height: 34px;
  padding: 0 8px;
}

.btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  height: var(--h-control);
  padding: 0 16px;
  border-radius: var(--radius-control);
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  cursor: pointer;
  white-space: nowrap;
}

.btn.small {
  height: var(--h-chip-sm);
  padding: 0 10px;
  gap: 6px;
  font-size: 12px;
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

.btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.name-in {
  width: 100%;
  height: var(--h-control);
  padding: 0 10px;
  box-sizing: border-box;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-family: inherit;
  font-size: 14px;
  outline: none;
}

.name-in:focus {
  border-color: var(--accent);
}
</style>
