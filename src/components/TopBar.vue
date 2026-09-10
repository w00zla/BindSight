<script setup lang="ts">
import Icon from "./Icon.vue";
import Dropdown from "./Dropdown.vue";
import logo from "../assets/logo.png";
import { ENVIRONMENTS, type Mode } from "../types";

defineProps<{ mode: Mode; activeEnv: string; scVersion: string; loading: boolean }>();
const emit = defineEmits<{ "update:mode": [mode: Mode]; "update:env": [slug: string]; refresh: []; settings: [] }>();

const TABS: { mode: Mode; label: string; icon: "live" | "bindings" | "devices" }[] = [
  { mode: "live", label: "Monitor", icon: "live" },
  { mode: "tools", label: "Bindings", icon: "bindings" },
  { mode: "devices", label: "Devices", icon: "devices" },
];

const ENV_OPTIONS = ENVIRONMENTS.map((slug) => ({ value: slug, label: slug }));
</script>

<template>
  <header class="topbar">
    <div class="brand">
      <img class="logo" :src="logo" alt="" />
      <span class="wordmark">BINDSIGHT</span>
    </div>
    <div class="tabs">
      <button
        v-for="t in TABS"
        :key="t.mode"
        type="button"
        class="tab"
        :class="{ active: mode === t.mode }"
        @click="emit('update:mode', t.mode)"
      >
        <Icon :name="t.icon" :size="16" />
        {{ t.label }}
      </button>
    </div>
    <div class="spacer" />
    <Dropdown
      variant="mono"
      :modelValue="activeEnv"
      :options="ENV_OPTIONS"
      title="Change environment"
      @update:modelValue="emit('update:env', $event)"
    />
    <div v-if="scVersion" class="version-chip mono">{{ scVersion }}</div>
    <button
      type="button"
      class="refresh-btn"
      :disabled="loading"
      title="Devices, actionmaps.xml, Game.log"
      @click="emit('refresh')"
    >
      <Icon name="refresh" :size="16" />
      {{ loading ? "Scanning…" : "Refresh" }}
    </button>
    <button type="button" class="gear-btn" @click="emit('settings')">
      <Icon name="settings" :size="18" />
    </button>
  </header>
</template>

<style scoped>
.topbar {
  height: 56px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 20px;
  padding: 0 16px;
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border-dim);
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
}

.logo {
  width: 28px;
  height: 28px;
  display: block;
}

.wordmark {
  font-weight: 700;
  font-size: 16px;
  letter-spacing: 0.22em;
  color: var(--text);
}

.tabs {
  display: flex;
  gap: 2px;
  background: var(--bg-base);
  border-radius: 6px;
  padding: 3px;
}

.tab {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 14px;
  border-radius: 4px;
  border: none;
  background: transparent;
  font-family: inherit;
  font-weight: 600;
  font-size: 14px;
  color: var(--text-2);
  cursor: pointer;
}

.tab.active {
  background: var(--accent);
  color: var(--accent-text);
}

.spacer {
  flex: 1;
}


.version-chip {
  display: flex;
  align-items: center;
  height: var(--h-chip);
  padding: 0 12px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  font-size: 13px;
  color: var(--text-2);
}

.refresh-btn {
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-control);
  padding: 0 16px;
  border-radius: var(--radius-control);
  border: 1px solid rgba(255, 255, 255, 0.5);
  background: transparent;
  font-family: inherit;
  font-weight: 600;
  font-size: 14px;
  color: var(--text);
  cursor: pointer;
}

.refresh-btn:disabled {
  opacity: 0.6;
  cursor: default;
}

.gear-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-control);
  border: 1px solid rgba(255, 255, 255, 0.25);
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}
</style>
