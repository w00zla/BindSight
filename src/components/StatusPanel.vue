<script setup lang="ts">
import { computed, ref } from "vue";
import Icon from "./Icon.vue";
import ConfirmDialog from "./ConfirmDialog.vue";
import type { ClashReport, ScStatus } from "../types";

const props = defineProps<{ report: ClashReport | null; loadError: string | null; sc: ScStatus | null }>();
const emit = defineEmits<{ apply: []; copy: [command: string] }>();

// Fix via config overwrites the game's actionmaps.xml: ask first.
const confirmApply = ref(false);

// Fix via console: the swaps as one console line. The engine's console splits
// a line on ";" (Lumberyard XConsole), so SC runs them in order.
const showConsole = ref(false);
const consoleCommand = computed(() => (props.report?.resort_commands ?? []).join("; "));

function onConfirmApply(value: string) {
  confirmApply.value = false;
  if (value === "rewrite") emit("apply");
}

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
          <span>Reading game data…</span>
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
          <span class="name">No game data</span>
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
        <span>No joystick order found</span>
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
          <span class="name">Joystick order clash!</span>
          <span class="spacer" />
          <button type="button" class="fix-btn" @click="confirmApply = true">
            <Icon name="file" :size="14" />
            Fix via Config
          </button>
          <button type="button" class="copy-btn" @click="showConsole = true">
            <Icon name="terminal" :size="14" />
            Fix via Console
          </button>
        </div>
        <!-- Moves without a saved device only shuffle empty slots to close the
             swap cycle: nothing to show, the console commands still carry them. -->
        <div class="moves">
          <div v-for="m in report.resort.filter((r) => r.name)" :key="m.from" class="move">
            <span class="dot filled" />
            <span class="name">{{ m.name }}</span>
            <span class="chip mono">js{{ m.from }}</span>
            <Icon name="arrow-right" :size="16" />
            <span class="chip mono">js{{ m.to }}</span>
          </div>
        </div>
      </div>
    </div>

    <ConfirmDialog
      v-if="confirmApply"
      title="Rewrite bindings configuration?"
      icon="file"
      :buttons="[
        { label: 'Rewrite', kind: 'primary', value: 'rewrite' },
        { label: 'Cancel', kind: 'outline', value: 'cancel' },
      ]"
      @choose="onConfirmApply"
    >
      <p class="dialog-note">Restart the game afterwards for the changes to take effect.</p>
    </ConfirmDialog>

    <ConfirmDialog
      v-if="showConsole"
      title="Fix via console"
      icon="terminal"
      :buttons="[{ label: 'Close', kind: 'outline', value: 'close' }]"
      @choose="showConsole = false"
    >
      <ol class="console-steps">
        <li>Open the Star Citizen console in-game with <span class="console-key mono">^</span></li>
        <li>Paste the command and press Enter</li>
      </ol>
      <div class="cmd-row">
        <input
          class="cmd-input mono"
          :value="consoleCommand"
          readonly
          spellcheck="false"
          @focus="($event.target as HTMLInputElement).select()"
        />
        <button type="button" class="cmd-copy" @click="emit('copy', consoleCommand)">
          <Icon name="copy" :size="14" />
          Copy
        </button>
      </div>
    </ConfirmDialog>
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

/* Fix via config dialog body (slot content of ConfirmDialog). */
.dialog-note {
  margin: 0;
  font-size: 14px;
  color: var(--text-2);
}

/* Fix via console dialog body (slot content of ConfirmDialog). */
.console-steps {
  margin: 0;
  padding-left: 20px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 14px;
  color: var(--text);
}

.console-key {
  padding: 1px 8px;
  border-radius: var(--radius-control);
  border: 1px solid var(--border);
  background: var(--bg-surface-2);
}

.cmd-row {
  display: flex;
  gap: 8px;
}

.cmd-input {
  flex: 1;
  min-width: 0;
  height: var(--h-control);
  box-sizing: border-box;
  padding: 0 12px;
  border: none;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.cmd-copy {
  height: var(--h-control);
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 16px;
  border: none;
  border-radius: var(--radius-control);
  background: var(--accent);
  color: var(--accent-text);
  font-family: inherit;
  font-weight: 600;
  font-size: 14px;
  cursor: pointer;
}
</style>
