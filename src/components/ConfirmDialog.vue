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
  disabled?: boolean;
  // "left" parks the button at the far left of the footer.
  side?: "left";
}
</script>

<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import Icon from "./Icon.vue";

// `captureKeys`: the keyboard capture stays on while this dialog is open
// (see keyboard.ts) — for a dialog that waits for a key press.
// `subtitle`: a dim second line under the title (the rebind dialog's category).
// `badge`: a small chip next to the title (the App Update dialog's channel).
// `width`: dialog width in px when the default (420, or 560 with body
// content) is not enough.
const props = defineProps<{
  title: string;
  subtitle?: string;
  badge?: string;
  icon: ConfirmIcon;
  buttons: ConfirmButton[];
  captureKeys?: boolean;
  width?: number;
}>();
const emit = defineEmits<{ choose: [value: string] }>();

// A click on the backdrop or Escape answers with the first outline button
// (the way out), or does nothing when there is none. A `captureKeys` dialog
// owns its keys, Escape included.
const dismiss = computed(() => props.buttons.find((b) => b.kind === "outline")?.value ?? null);

function onBackdrop() {
  if (dismiss.value !== null) emit("choose", dismiss.value);
}

function onKey(e: KeyboardEvent) {
  if (e.key !== "Escape" || props.captureKeys) return;
  onBackdrop();
}

onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="backdrop" @click.self="onBackdrop">
    <div
      class="dialog"
      :class="{ wide: !!$slots.default }"
      :style="width ? { width: `${width}px` } : undefined"
      role="dialog"
      :aria-label="title"
      :data-capture-keys="captureKeys ? '' : undefined"
    >
      <div class="head">
        <Icon :name="icon" :size="18" />
        <div class="titles">
          <span class="title-line">
            <span class="title">{{ title }}</span>
            <span v-if="badge" class="badge">{{ badge }}</span>
          </span>
          <span v-if="subtitle" class="subtitle">{{ subtitle }}</span>
        </div>
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
          :class="[b.kind, { left: b.side === 'left' }]"
          :disabled="b.disabled"
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

.titles {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.title-line {
  display: flex;
  align-items: center;
  gap: 10px;
}

.title {
  font-weight: 600;
  font-size: 18px;
}

.badge {
  display: inline-flex;
  align-items: center;
  height: 20px;
  padding: 0 8px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-control);
  font-size: 11px;
  font-weight: 600;
  color: var(--accent);
}

.subtitle {
  font-size: 12px;
  color: var(--text-2);
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

.btn.left {
  margin-right: auto;
}

.btn:disabled {
  opacity: 0.4;
  cursor: default;
}
</style>
