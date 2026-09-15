<script setup lang="ts">
// A select in the app's look: WebKitGTK paints a native <select> popup with
// the GTK theme, which no CSS reaches. Closes on outside click and Escape.
import { computed, onMounted, onUnmounted, ref } from "vue";
import Icon from "./Icon.vue";

export type DropdownOption = { value: string; label: string };

const props = withDefaults(
  defineProps<{
    modelValue: string;
    options: DropdownOption[];
    // Shown while no option matches the value (e.g. an "Add …" picker).
    placeholder?: string;
    // chip: a full-height chip with a bold name (Compare); small: a compact
    // chip (stage caption); dashed: a transparent, dashed "add" chip; mono:
    // an uppercase mono slug (the top bar's environment); outline: a
    // button-high field, filled surface with a border (Settings rows).
    variant?: "chip" | "small" | "dashed" | "mono" | "outline";
    title?: string;
    // Open the list above the button (a dropdown at the bottom of a
    // scrolling container would otherwise grow the container).
    up?: boolean;
  }>(),
  { placeholder: "", variant: "chip", title: undefined, up: false },
);
const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);

const current = computed(() => props.options.find((o) => o.value === props.modelValue));

function pick(value: string) {
  open.value = false;
  emit("update:modelValue", value);
}

function onDocDown(e: MouseEvent) {
  if (open.value && !root.value?.contains(e.target as Node)) open.value = false;
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") open.value = false;
}

onMounted(() => {
  document.addEventListener("mousedown", onDocDown);
  document.addEventListener("keydown", onKey);
});
onUnmounted(() => {
  document.removeEventListener("mousedown", onDocDown);
  document.removeEventListener("keydown", onKey);
});
</script>

<template>
  <div ref="root" class="dd" :class="variant">
    <button type="button" class="dd-btn" :title="title" @click="open = !open">
      <span class="dd-name" :class="{ empty: !current, mono: variant === 'mono' }">{{ current?.label ?? placeholder }}</span>
      <Icon name="chevron-down" :size="variant === 'mono' ? 14 : 12" />
    </button>
    <div v-if="open" class="dd-menu" :class="{ up }">
      <button
        v-for="o in options"
        :key="o.value"
        type="button"
        class="dd-item"
        :class="{ active: o.value === modelValue, mono: variant === 'mono' }"
        @click="pick(o.value)"
      >
        {{ o.label }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.dd {
  position: relative;
  max-width: 240px;
}

.dd-btn {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: var(--h-chip);
  padding: 0 12px;
  border: none;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text-2);
  font-family: inherit;
  font-size: 13px;
  cursor: pointer;
}

.dd-btn:hover {
  background: var(--bg-surface-3);
}

.dd-name {
  flex: 1;
  text-align: left;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dd-name.empty {
  font-weight: 400;
  color: var(--text-2);
}

.small .dd-btn {
  height: var(--h-chip-sm);
  padding: 0 8px;
  font-size: 12px;
}

.small .dd-name {
  font-weight: 400;
}

.dashed .dd-btn {
  height: var(--h-chip-sm);
  padding: 0 10px;
  border: 1px dashed rgba(173, 211, 235, 0.4);
  background: transparent;
}

.dashed .dd-btn:hover {
  background: var(--bg-surface-2);
}

.outline {
  min-width: 96px;
}

.outline .dd-btn {
  height: var(--h-control);
  padding: 0 12px 0 16px;
  border: 1px solid var(--border);
  background: var(--bg-surface-2);
  font-size: 14px;
}

.outline .dd-btn:hover {
  border-color: var(--accent);
  background: var(--bg-surface-3);
}

.outline .dd-name {
  font-weight: 500;
}

.mono .dd-btn {
  gap: 6px;
  padding: 0 10px 0 12px;
  font-weight: 700;
  letter-spacing: 0.08em;
}

.mono .dd-name {
  font-weight: 700;
}

.dd-item.mono {
  font-weight: 700;
  letter-spacing: 0.08em;
}

.dd-menu {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  min-width: 100%;
  max-height: 320px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  padding: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface);
  z-index: 20;
}

.dd-menu.up {
  top: auto;
  bottom: calc(100% + 4px);
}

.dd-item {
  display: flex;
  align-items: center;
  height: var(--h-chip-sm);
  padding: 0 12px;
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  font-family: inherit;
  font-size: 13px;
  color: var(--text-2);
  cursor: pointer;
  text-align: left;
  white-space: nowrap;
}

.dd-item:hover {
  background: var(--bg-surface-2);
  color: var(--text);
}

.dd-item.active {
  color: var(--live);
}
</style>
