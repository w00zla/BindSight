<script lang="ts">
// "2026-09-09 13:40:05", local time.
export function stamp(unixSecs: number): string {
  const d = new Date(unixSecs * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}
</script>

<script setup lang="ts">
// The Backups panel of the Bindings and the Config mode's left column: the
// app's backups of the live files, Create Backup (a description asked in a
// small dialog). The list and the busy flag are the caller's (`v-model`);
// so are the selection and what a pick means.
import { nextTick, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import Icon from "./Icon.vue";
import ConfirmDialog from "./ConfirmDialog.vue";
import type { BackupSummary } from "../types";

defineProps<{
  // The backup shown as picked (its id); null = none.
  picked: string | null;
}>();
const emit = defineEmits<{
  notify: [message: string, type: "ok" | "error"];
  pick: [id: string];
}>();
const backups = defineModel<BackupSummary[]>("backups", { required: true });
const busy = defineModel<boolean>("busy", { required: true });

async function load() {
  try {
    backups.value = await invoke<BackupSummary[]>("list_backups");
  } catch (e) {
    emit("notify", String(e), "error");
  }
}

// Create Backup: the description is asked in a small dialog, "manual" by
// default (the backend keeps that for an empty one).
const backupDialog = ref<{ reason: string } | null>(null);
const backupInput = ref<HTMLInputElement | null>(null);
const BACKUP_REASON_MAX = 64;

function openCreate() {
  backupDialog.value = { reason: "manual" };
  nextTick(() => {
    backupInput.value?.focus();
    backupInput.value?.select();
  });
}

async function onBackupChoose(value: string) {
  const d = backupDialog.value;
  backupDialog.value = null;
  if (!d || value !== "create") return;
  busy.value = true;
  try {
    await invoke<BackupSummary>("create_backup", { reason: d.reason.trim() || "manual" });
    await load();
    emit("notify", "Backup created", "ok");
  } catch (e) {
    emit("notify", String(e), "error");
  } finally {
    busy.value = false;
  }
}

defineExpose({ load, openCreate });

onMounted(load);
</script>

<template>
  <section class="panel grow">
    <div class="head">
      <Icon name="history" :size="15" />
      <span class="head-title">Backups</span>
      <button type="button" class="btn primary small" :disabled="busy" @click="openCreate">
        <Icon name="plus" :size="12" />Create Backup
      </button>
    </div>
    <div class="rows scroll">
      <div v-for="b in backups" :key="b.id" class="row-item" :class="{ b: picked === b.id }" @click="emit('pick', b.id)">
        <div class="lines">
          <span class="mono line-stamp">{{ stamp(b.created) }}</span>
          <span class="line-sub">{{ b.reason }} · <span class="mono">{{ b.game_version ?? "—" }}</span></span>
        </div>
      </div>
      <div v-if="!backups.length" class="row-none">None</div>
    </div>

    <!-- back the live files up, with a description -->
    <ConfirmDialog
      v-if="backupDialog"
      title="Create Backup"
      icon="history"
      :buttons="[
        { label: 'Create', kind: 'primary', value: 'create' },
        { label: 'Cancel', kind: 'outline', value: 'cancel' },
      ]"
      @choose="onBackupChoose"
    >
      <label class="backup-row">
        <span class="backup-label">Description</span>
        <input
          ref="backupInput"
          v-model="backupDialog.reason"
          class="name-in"
          :maxlength="BACKUP_REASON_MAX"
          spellcheck="false"
          @keydown.enter="onBackupChoose('create')"
        />
      </label>
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

.panel.grow {
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

.line-stamp {
  font-weight: 600;
  font-size: 13px;
  color: var(--text);
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

.btn.primary {
  background: var(--accent);
  color: var(--accent-text);
  border: none;
}

.btn:disabled {
  opacity: 0.4;
  cursor: default;
}

/* Create Backup dialog: label + description input on one line. */
.backup-row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 10px;
}

.backup-label {
  font-size: 13px;
  color: var(--text-2);
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
