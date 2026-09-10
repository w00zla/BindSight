<script lang="ts">
// One question, a row of buttons, one answer. The caller decides the wording
// and the icon; optional body content (default slot) sits between title and
// buttons.
import type { IconName } from "./Icon.vue";

export type ConfirmIcon = IconName;

export interface ConfirmButton {
  label: string;
  kind: "primary" | "outline" | "danger";
  value: string;
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import Icon from "./Icon.vue";

const props = defineProps<{ title: string; icon: ConfirmIcon; buttons: ConfirmButton[] }>();
const emit = defineEmits<{ choose: [value: string] }>();

// A click on the backdrop answers with the first outline button (the way out),
// or does nothing when there is none.
const dismiss = computed(() => props.buttons.find((b) => b.kind === "outline")?.value ?? null);

function onBackdrop() {
  if (dismiss.value !== null) emit("choose", dismiss.value);
}
</script>

<template>
  <div class="backdrop" @click.self="onBackdrop">
    <div class="dialog" :class="{ wide: !!$slots.default }" role="dialog" :aria-label="title">
      <div class="head">
        <Icon :name="icon" :size="18" />
        <span class="title">{{ title }}</span>
      </div>
      <div v-if="$slots.default" class="body">
        <slot />
      </div>
      <div class="foot">
        <button
          v-for="b in buttons"
          :key="b.value"
          type="button"
          class="btn"
          :class="b.kind"
          @click="emit('choose', b.value)"
        >
          {{ b.label }}
        </button>
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
  z-index: 60;
}

.dialog {
  width: 420px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* A dialog with body content gets more room. */
.dialog.wide {
  width: 560px;
}

.head,
.foot {
  display: flex;
  align-items: center;
  padding: 16px 24px;
}

.head {
  gap: 10px;
  border-bottom: 1px solid var(--border-dim);
}

.body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px 24px;
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

.btn.danger {
  background: transparent;
  color: var(--err);
  border: 1px solid color-mix(in srgb, var(--err) 60%, transparent);
}
</style>
