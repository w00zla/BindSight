<script setup lang="ts">
import type { ScStatus } from "../types";
// The top bar's logo artwork, large enough for this tile.
import logo from "../../src-tauri/icons/128x128@2x.png";

// Shown instead of every mode while the first game-data load after start
// runs; App.vue swaps it for the normal view once that load ends. `version`
// is the app version from `system_info` (empty until it arrived).
defineProps<{ sc: ScStatus | null; version: string }>();
</script>

<template>
  <div class="startup">
    <div class="brand">
      <img class="logo" :src="logo" alt="" />
      <span class="wordmark">BIND<span class="sight">SIGHT</span></span>
      <span class="tagline">BindSight gets your binds right!</span>
      <span v-if="version" class="version mono">v{{ version }}</span>
    </div>
    <div class="progress">
      <span class="label">Reading game data…</span>
      <div class="steps">
        <span v-for="i in sc?.steps ?? 0" :key="i" class="step" :class="{ done: i <= (sc?.progress ?? 0) }" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.startup {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 48px;
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
}

.brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
}

.logo {
  width: 128px;
  height: 128px;
  display: block;
}

/* Same wordmark as the top bar, large. The negative margin takes back the
   letter spacing after the last letter, so the word sits centred. */
.wordmark {
  font-weight: 700;
  font-size: 40px;
  letter-spacing: 0.22em;
  margin-right: -0.22em;
  color: var(--text);
}

.sight {
  color: var(--accent);
}

.tagline {
  margin-top: -8px;
  font-size: 14px;
  color: var(--text-2);
}

.version {
  margin-top: -12px;
  font-size: 14px;
  font-weight: 700;
  color: var(--text-2);
}

.progress {
  width: 420px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.label {
  font-size: 14px;
  color: var(--text-2);
  text-align: center;
}

/* Segmented progress: one cell per load step, filled as they complete. */
.steps {
  display: flex;
  gap: 6px;
}

.step {
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background: var(--bg-surface-3);
  transition: background 150ms;
}

.step.done {
  background: var(--accent);
}
</style>
