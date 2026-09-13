<script setup lang="ts">
// The "run this in the game's console" dialog: how to open the console, the
// command line, a Copy button. Used by the order fix (Status panel) and the
// reorder (Bindings mode); the caller puts the line on the clipboard.
import ConfirmDialog from "./ConfirmDialog.vue";
import Icon from "./Icon.vue";

defineProps<{ command: string }>();
const emit = defineEmits<{ close: []; copy: [command: string] }>();
</script>

<template>
  <ConfirmDialog
    title="Fix via console"
    icon="terminal"
    :buttons="[{ label: 'Close', kind: 'outline', value: 'close' }]"
    @choose="emit('close')"
  >
    <ol class="console-steps">
      <li>Open the Star Citizen console in-game with <span class="console-key mono">^</span></li>
      <li>Paste the command and press Enter</li>
    </ol>
    <div class="cmd-row">
      <input class="cmd-input mono" :value="command" readonly spellcheck="false" @focus="($event.target as HTMLInputElement).select()" />
      <button type="button" class="cmd-copy" @click="emit('copy', command)">
        <Icon name="copy" :size="14" />
        Copy
      </button>
    </div>
  </ConfirmDialog>
</template>

<style scoped>
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
