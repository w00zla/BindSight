<script setup lang="ts">
import { computed } from "vue";
import Icon from "./Icon.vue";
import type { DeviceInfo, SlotStatus } from "../types";
import { deviceName } from "../devices";
import { KEY_COUNT } from "../keyboard";

const props = defineProps<{
  device: DeviceInfo;
  slot: SlotStatus | null;
  ignored: boolean;
  unseen: boolean;
  bindingCount: number;
  // Taken off the image stage by the user.
  hidden: boolean;
}>();
const emit = defineEmits<{ toggleMap: [] }>();

// A further pad holds no SC slot: it cannot carry bindings.
const noSlot = computed(() => props.device.kind === "gamepad" && props.device.gamepad_slot === null);
const dimmed = computed(() => props.ignored || props.unseen || noSlot.value);

// dot: filled ok = seen with no clash, filled warn = clash, hollow otherwise.
const dotState = computed<"ok" | "warn" | "hollow">(() => {
  if (dimmed.value) return "hollow";
  if (props.device.kind !== "joystick") return "ok";
  if (props.slot?.clash) return "warn";
  if (props.slot) return "ok";
  return "hollow";
});

// SC's fixed instance chip for the keyboard and the slotted pad.
const kindChip = computed(() => {
  if (props.device.kind === "keyboard") return "kb1";
  return props.device.kind === "gamepad" && !noSlot.value ? "gp1" : null;
});

const name = computed(() => deviceName(props.device));

const state = computed(() => {
  const d = props.device;
  if (props.ignored) return "excluded";
  if (noSlot.value) return "no slot";
  if (props.unseen) return "not seen by SC";
  const counts =
    d.kind === "keyboard" ? [`${KEY_COUNT} keys`] : [`${d.num_buttons} btns`, `${d.num_axes} axes`, `${d.num_hats} hats`];
  return [`${props.bindingCount} bindings`, ...counts].join(" · ");
});

</script>

<template>
  <div class="tile" :class="{ clash: slot?.clash }" :style="{ opacity: dimmed ? 0.55 : 1 }">
    <div class="row1">
      <span class="dot" :class="dotState" />
      <span class="name">{{ name }}</span>
      <template v-if="device.kind === 'joystick'">
        <span v-if="slot?.clash" class="chip clash-chip mono">js{{ slot.stored_instance }} → js{{ slot.effective_instance }}</span>
        <span v-else-if="slot" class="chip mono">js{{ slot.effective_instance }}</span>
      </template>
      <span v-else-if="kindChip" class="chip mono">{{ kindChip }}</span>
    </div>
    <div class="row2">
      <button
        v-if="device.hardware_id"
        type="button"
        class="eye"
        :title="hidden ? 'Show on stage' : 'Hide from stage'"
        @click="emit('toggleMap')"
      >
        <Icon :name="hidden ? 'eye-off' : 'eye'" :size="14" />
      </button>
      <span class="status">{{ state }}</span>
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
  display: flex;
  align-items: center;
  gap: 8px;
}

.status {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Sized to the row so the button does not stretch the tile. */
.eye {
  width: 16px;
  height: 16px;
  line-height: 0;
  padding: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  flex-shrink: 0;
}

.eye:hover {
  color: var(--accent);
}
</style>
