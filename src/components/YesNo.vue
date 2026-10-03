<script setup lang="ts">
// A No | Yes toggle like the game's settings screen; `dim` shows a value
// that is not set (a default).
defineProps<{ value: boolean; dim?: boolean; disabled?: boolean }>();
const emit = defineEmits<{ set: [value: boolean] }>();
</script>

<template>
  <span class="yesno" :class="{ dim, disabled }">
    <button type="button" :disabled="disabled" :aria-pressed="!value" @click="emit('set', false)">No</button>
    <button type="button" class="yes" :disabled="disabled" :aria-pressed="value" @click="emit('set', true)">Yes</button>
  </span>
</template>

<style scoped>
.yesno {
  display: inline-flex;
  flex-shrink: 0;
  width: max-content;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  overflow: hidden;
}

.yesno.dim,
.yesno.disabled {
  opacity: 0.55;
}

.yesno button {
  border: none;
  background: transparent;
  padding: 2px 12px;
  font-family: inherit;
  font-size: 12px;
  color: var(--text-3);
  cursor: pointer;
}

.yesno button:disabled {
  cursor: default;
}

.yesno button[aria-pressed="true"] {
  background: var(--bg-surface-3);
  color: var(--text);
}

.yesno button.yes[aria-pressed="true"] {
  background: color-mix(in srgb, var(--accent) 35%, transparent);
}
</style>
