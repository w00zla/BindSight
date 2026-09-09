<script setup lang="ts">
import { computed } from "vue";
import Icon from "./Icon.vue";
import type { ClashReport, ScStatus } from "../types";

const props = defineProps<{ report: ClashReport | null; loadError: string | null; sc: ScStatus | null }>();
const emit = defineEmits<{ apply: []; copy: [] }>();

const logErrorTitle = computed(() => {
  const le = props.report?.log_error;
  if (!le) return "";
  return le.kind === "not_found" ? `${le.path}: ${le.reason}` : le.path;
});

const hasIssue = computed(
  () =>
    !!props.sc?.error ||
    !!props.loadError ||
    !!props.report?.log_error ||
    !!props.report?.missing.length ||
    !!props.report?.has_clash,
);
</script>

<template>
  <div class="status-panel">
    <div class="panel-title">Status</div>
    <div class="tiles">
      <div v-if="sc?.loading" class="tile loading">
        <div class="row">
          <Icon name="clock" :size="16" />
          <span>Reading SC data…</span>
        </div>
        <div class="steps">
          <span v-for="i in sc.steps" :key="i" class="step" :class="{ done: i <= sc.progress }" />
        </div>
      </div>

      <div v-else-if="!hasIssue" class="tile ok">
        <Icon name="check" :size="16" />
        <span>No issues</span>
      </div>

      <div v-if="sc?.error" class="tile error detail">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">No SC data</span>
        </div>
        <div class="error mono">{{ sc.error }}</div>
      </div>

      <div v-if="loadError" class="tile error detail">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">No bindings</span>
        </div>
        <div class="error mono">{{ loadError }}</div>
      </div>

      <div v-if="report?.log_error" class="tile issue" :title="logErrorTitle">
        <Icon name="warning" :size="16" />
        <span>No device order found</span>
      </div>

      <div v-for="m in report?.missing ?? []" :key="m.stored_instance" class="tile issue">
        <Icon name="warning" :size="16" />
        <span class="name">{{ m.name }}</span>
        <span class="chip mono">js{{ m.stored_instance }}</span>
        <span>missing</span>
      </div>

      <div v-if="report?.has_clash" class="tile issue clash">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">Order clash</span>
          <span class="spacer" />
          <button type="button" class="fix-btn" @click="emit('apply')">
            <Icon name="rotate" :size="14" />
            Fix via config
          </button>
          <button type="button" class="copy-btn" @click="emit('copy')">
            <Icon name="copy" :size="14" />
            Fix via console
          </button>
        </div>
        <div class="moves">
          <div v-for="m in report.resort" :key="m.from" class="move">
            <span class="dot" :class="{ filled: m.name }" />
            <span v-if="m.name" class="name">{{ m.name }}</span>
            <span v-else class="unplugged">unplugged</span>
            <span class="chip mono" :class="{ dim: !m.name }">js{{ m.from }}</span>
            <Icon name="arrow-right" :size="16" />
            <span class="chip mono" :class="{ dim: !m.name }">js{{ m.to }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.status-panel {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  padding: 12px 16px 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.tiles {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}

/* Child tiles share the device tile footprint (84px high). */
.tile {
  min-width: 240px;
  min-height: 84px;
  box-sizing: border-box;
  padding: 12px 16px;
  border-radius: var(--radius-panel);
  background: var(--bg-surface-2);
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
  color: var(--text);
}

.tile.ok {
  color: var(--ok);
}

.tile.issue {
  background: rgba(242, 179, 76, 0.1);
  border: 1px solid rgba(242, 179, 76, 0.5);
  color: var(--warn);
}

/* Hard failures (no game data at all), as opposed to warnings. */
.tile.error {
  background: rgba(255, 92, 108, 0.1);
  border: 1px solid rgba(255, 92, 108, 0.5);
  color: var(--err);
}

.tile.clash {
  flex-direction: column;
  align-items: stretch;
  justify-content: center;
  gap: 8px;
  min-width: 480px;
}

.tile.loading,
.tile.detail {
  flex-direction: column;
  align-items: stretch;
  justify-content: center;
  gap: 8px;
}

.tile.detail {
  max-width: 480px;
}

/* Segmented progress: one cell per load step, filled as they complete. */
.steps {
  display: flex;
  gap: 4px;
}

.step {
  flex: 1;
  height: 4px;
  border-radius: 2px;
  background: var(--bg-surface-3);
  transition: background 150ms;
}

.step.done {
  background: var(--accent);
}

.error {
  font-size: 12px;
  color: var(--text);
  overflow-wrap: anywhere;
}

.row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.name {
  font-weight: 600;
  color: var(--text);
}

.spacer {
  flex: 1;
}

.chip {
  height: 24px;
  display: inline-flex;
  align-items: center;
  padding: 0 8px;
  border-radius: var(--radius-control);
  border: 1px solid rgba(242, 179, 76, 0.7);
  color: var(--warn);
  font-size: 13px;
}

.chip.dim {
  border-color: rgba(242, 179, 76, 0.35);
  color: rgba(242, 179, 76, 0.6);
}

.fix-btn,
.copy-btn {
  height: var(--h-chip-sm);
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  border-radius: var(--radius-control);
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  cursor: pointer;
}

.fix-btn {
  background: var(--warn);
  color: #000;
  border: none;
}

.copy-btn {
  background: transparent;
  color: var(--warn);
  border: 1px solid rgba(242, 179, 76, 0.6);
}

.moves {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
}

.move {
  display: flex;
  align-items: center;
  gap: 8px;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  border: 1px solid rgba(242, 179, 76, 0.6);
  box-sizing: border-box;
}

.dot.filled {
  background: var(--warn);
  border-color: var(--warn);
}

.unplugged {
  color: rgba(173, 211, 235, 0.7);
}
</style>
