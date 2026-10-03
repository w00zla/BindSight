<script setup lang="ts">
// The Apply dialog for a profile or backup, shared by the Bindings and the
// Config mode: what to take over (Bindings, Settings), the devices (the
// caller's list, default slot), Apply. The caller decides what Apply does
// with the answer.
import { computed, ref } from "vue";
import ConfirmDialog, { type ConfirmButton } from "./ConfirmDialog.vue";

const props = defineProps<{
  title: string;
  // The primary button's label ("Apply", "Restore").
  applyLabel: string;
  // The caller's device list allows an apply (something ticked, no clash).
  devicesOk: boolean;
  width?: number;
}>();
const emit = defineEmits<{ apply: [what: { bindings: boolean; settings: boolean }]; cancel: [] }>();

const bindings = ref(true);
const settings = ref(true);

const buttons = computed<ConfirmButton[]>(() => [
  { label: props.applyLabel, kind: "primary", value: "apply", disabled: !props.devicesOk || !(bindings.value || settings.value) },
  { label: "Cancel", kind: "outline", value: "cancel" },
]);

function onChoose(value: string) {
  if (value === "apply") emit("apply", { bindings: bindings.value, settings: settings.value });
  else emit("cancel");
}
</script>

<template>
  <ConfirmDialog :title="title" icon="check" :buttons="buttons" :width="width" @choose="onChoose">
    <div class="what">
      <label class="check">
        <input v-model="bindings" type="checkbox" />
        <span>Bindings</span>
      </label>
      <label class="check">
        <input v-model="settings" type="checkbox" />
        <span>Settings</span>
      </label>
    </div>
    <slot />
  </ConfirmDialog>
</template>

<style scoped>
.what {
  display: flex;
  gap: 24px;
  padding: 0 4px 12px;
  border-bottom: 1px solid var(--border-dim);
}

/* Own checkbox look (mirrors SettingsDialog): WebKitGTK would paint GTK's. */
.check {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
  cursor: pointer;
  user-select: none;
}

.check input {
  appearance: none;
  width: 16px;
  height: 16px;
  margin: 0;
  flex-shrink: 0;
  display: grid;
  place-content: center;
  border: 1px solid rgba(255, 255, 255, 0.5);
  border-radius: 3px;
  background: transparent;
  cursor: pointer;
}

.check input:hover {
  border-color: var(--accent);
}

.check input:checked {
  background: var(--accent);
  border-color: var(--accent);
}

.check input:checked::after {
  content: "";
  width: 8px;
  height: 4px;
  border-left: 2px solid var(--accent-text);
  border-bottom: 2px solid var(--accent-text);
  transform: translateY(-1px) rotate(-45deg);
}
</style>
