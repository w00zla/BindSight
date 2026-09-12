<script setup lang="ts">
import { computed } from "vue";
import Icon from "./Icon.vue";
import type { CurrentInput, LiveState } from "../types";

const props = defineProps<{
  input: CurrentInput | null;
  state: LiveState;
  // Extra distinction the plain state does not carry: "excluded" (user marked
  // it) vs "not seen by game" (SC's device order never listed it) both show as "unseen".
  excluded: boolean;
  tokenLabel: (token: string) => string;
  categoryLabel: (actionmap: string) => string;
}>();

const bound = computed(() => props.state === "bound");
const bigLabel = computed(() => {
  const c = props.input;
  if (!c) return "No input registered";
  return c.token ? props.tokenLabel(c.token) : c.sdl;
});
</script>

<template>
  <div class="card" :class="{ bordered: bound }">
    <div class="panel-title">Last Input</div>
    <div class="row1">
      <Icon
        name="bolt"
        :size="22"
        :fill="bound ? 'rgba(78,224,255,0.25)' : 'none'"
        :style="{ color: bound ? 'var(--live)' : 'var(--text-3)' }"
      />
      <span class="big-label" :class="{ bound, empty: !input }">{{ bigLabel }}</span>
    </div>
    <div v-if="input" class="row2">
      <span>{{ input.device }}</span>
      <span v-if="state === 'unseen'" class="chip">{{ excluded ? "excluded" : "not seen by game" }}</span>
      <span v-else-if="state === 'noorder'" class="chip">no joystick order</span>
    </div>
    <div v-if="input?.actions.length" class="actions">
      <div v-for="(a, i) in input.actions" :key="i" class="action-row">
        <span class="action-label">{{ a.label ?? a.action }}</span>
        <span class="action-cat">{{ categoryLabel(a.actionmap) }}</span>
      </div>
    </div>
    <div v-else-if="input" class="no-binding">No binding</div>
  </div>
</template>

<style scoped>
.card {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  padding: 12px 16px 16px;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 10px;
  border: 1px solid transparent;
  min-height: 0;
  overflow: hidden;
}

.card.bordered {
  border-color: rgba(78, 224, 255, 0.35);
}

.row1 {
  display: flex;
  align-items: center;
  gap: 10px;
}

.big-label {
  font-size: 22px;
  font-weight: 600;
  line-height: 1.1;
  color: var(--text-2);
}

.big-label.empty {
  color: var(--text-3);
}

.big-label.bound {
  color: var(--live);
}

.row2 {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-2);
}

.chip {
  color: var(--warn);
}

.actions {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 2px;
  overflow-y: auto;
}

.action-row {
  background: var(--bg-surface-2);
  border-radius: 6px;
  padding: 10px 12px;
  display: flex;
  align-items: center;
  gap: 10px;
}

.action-label {
  font-weight: 600;
  font-size: 15px;
  flex: 1;
}

.action-cat {
  font-size: 12px;
  color: var(--text-2);
}

.no-binding {
  font-size: 12px;
  color: var(--text-3);
}
</style>
