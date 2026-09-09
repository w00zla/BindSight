<script setup lang="ts">
import { computed } from "vue";
import Icon from "./Icon.vue";
import logo from "../assets/logo.png";
import type { Mode } from "../types";

const props = defineProps<{ mode: Mode; basePath: string; scVersion: string; loading: boolean }>();
const emit = defineEmits<{ "update:mode": [mode: Mode]; refresh: []; settings: [] }>();

const TABS: { mode: Mode; label: string; icon: "live" | "bindings" | "devices" }[] = [
  { mode: "live", label: "Monitor", icon: "live" },
  { mode: "tools", label: "Bindings", icon: "bindings" },
  { mode: "devices", label: "Devices", icon: "devices" },
];

// Install chip label: last path segment of the base path, e.g.
// ".../StarCitizen/LIVE" -> "LIVE".
const installSlug = computed(() => {
  const parts = props.basePath.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1].toUpperCase() : "";
});
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
    <div v-if="installSlug" class="install-chip mono">{{ installSlug }}</div>
    <div v-if="scVersion" class="version-chip mono">{{ scVersion }}</div>
    <button type="button" class="refresh-btn" :disabled="loading" @click="emit('refresh')">
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

.install-chip {
  display: flex;
  align-items: center;
  height: var(--h-chip);
  padding: 0 12px;
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  font-weight: 700;
  font-size: 13px;
  letter-spacing: 0.08em;
  color: var(--text);
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
