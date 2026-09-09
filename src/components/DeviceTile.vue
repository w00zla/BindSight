<script setup lang="ts">
import { computed } from "vue";
import type { DeviceInfo, SlotStatus } from "../types";

const props = defineProps<{
  device: DeviceInfo;
  slot: SlotStatus | null;
  ignored: boolean;
  unseen: boolean;
  bindingCount: number;
}>();

const dimmed = computed(() => props.ignored || props.unseen);

// dot: filled ok = seen with no clash, filled warn = clash, hollow otherwise.
const dotState = computed<"ok" | "warn" | "hollow">(() => {
  if (dimmed.value) return "hollow";
  if (props.slot?.clash) return "warn";
  if (props.slot) return "ok";
  return "hollow";
});

const statusText = computed(() => {
  if (props.ignored) return "excluded";
  if (props.unseen) return "not seen by SC";
  return `${props.bindingCount} bindings · ${props.device.num_buttons} btn · ${props.device.num_axes} axes · ${props.device.num_hats} hats`;
});
</script>

<template>
  <div class="tile" :class="{ clash: slot?.clash }" :style="{ opacity: dimmed ? 0.55 : 1 }">
    <div class="row1">
      <span class="dot" :class="dotState" />
      <span class="name">{{ device.sc_name ?? device.sdl_name }}</span>
      <span v-if="slot?.clash" class="chip clash-chip mono">js{{ slot.stored_instance }} → js{{ slot.effective_instance }}</span>
      <span v-else-if="slot" class="chip mono">js{{ slot.effective_instance }}</span>
    </div>
    <div class="row2">
      {{ statusText }}<template v-if="device.axes_error">
        · <span class="axes-error" :title="device.axes_error">axes: {{ device.axes_error }}</span>
      </template>
    </div>
  </div>
</template>

<style scoped>
.tile {
  width: 340px;
  height: 84px;
  border-radius: var(--radius-panel);
  background: var(--bg-surface-2);
  border: 1px solid var(--border);
  padding: 12px 16px;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 6px;
  box-sizing: border-box;
  flex-shrink: 0;
}

.tile.clash {
  border-color: rgba(242, 179, 76, 0.6);
}

.row1 {
  display: flex;
  align-items: center;
  gap: 8px;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  display: inline-block;
  box-sizing: border-box;
  flex-shrink: 0;
}

.dot.ok {
  background: var(--ok);
}

.dot.warn {
  background: var(--warn);
}

.dot.hollow {
  border: 1px solid var(--text-2);
}

.name {
  font-weight: 600;
  font-size: 15px;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip {
  font-size: 13px;
  padding: 2px 8px;
  border-radius: 3px;
  background: var(--bg-surface-3);
  color: var(--text-2);
  white-space: nowrap;
}

.clash-chip {
  background: rgba(242, 179, 76, 0.15);
  color: var(--warn);
}

.row2 {
  font-size: 13px;
  color: var(--text-2);
}

.axes-error {
  color: var(--warn);
}
</style>
