<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
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

// Window controls: the window has no native frame, the bar is the title bar.
const win = getCurrentWindow();
const maximized = ref(false);
let unlisten: UnlistenFn | null = null;

async function refreshMaximized() {
  try {
    maximized.value = await win.isMaximized();
  } catch {
    maximized.value = false;
  }
}

onMounted(async () => {
  await refreshMaximized();
  unlisten = await win.onResized(refreshMaximized);
});
onUnmounted(() => {
  unlisten?.();
});
</script>

<template>
  <header class="topbar" data-tauri-drag-region>
    <div class="brand" data-tauri-drag-region>
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
    <div class="spacer" data-tauri-drag-region />
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
      title="Devices, bindings, device order"
      @click="emit('refresh')"
    >
      <Icon name="refresh" :size="16" />
      {{ loading ? "Scanning…" : "Refresh" }}
    </button>
    <button type="button" class="gear-btn" @click="emit('settings')">
      <Icon name="settings" :size="18" />
    </button>
    <div class="divider" />
    <div class="win-btns">
      <button type="button" class="win-btn" title="Minimize" @click="win.minimize()">
        <Icon name="minimize" :size="16" />
      </button>
      <button type="button" class="win-btn" :title="maximized ? 'Restore' : 'Maximize'" @click="win.toggleMaximize()">
        <Icon :name="maximized ? 'restore' : 'maximize'" :size="14" />
      </button>
      <button type="button" class="win-btn close" title="Close" @click="win.close()">
        <Icon name="close" :size="16" />
      </button>
    </div>
  </header>
</template>

<style scoped>
.topbar {
  position: sticky;
  top: 0;
  z-index: 30;
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
.divider {
  width: 1px;
  height: 24px;
  background: var(--border-dim);
}

/* Stick to the viewport's right edge while the bar scrolls sideways. */
.win-btns {
  position: sticky;
  right: 16px;
  display: flex;
  gap: 2px;
  background: var(--bg-surface);
}

.win-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}

.win-btn:hover {
  background: var(--bg-surface-2);
  color: var(--text);
}

.win-btn.close:hover {
  background: var(--err);
  color: var(--accent-text);
}
</style>
