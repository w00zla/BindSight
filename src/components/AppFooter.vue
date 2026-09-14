<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import Icon from "./Icon.vue";
import logo from "../assets/logo.png";

// The app version from `system_info` (empty until it arrived).
defineProps<{ version: string }>();

const REPO_URL = "https://github.com/w00zla/BindSight";

// A plain href would navigate the webview itself; the system browser opens it.
async function openRepo() {
  try {
    await openUrl(REPO_URL);
  } catch (e) {
    console.error("open GitHub page", e);
  }
}
</script>

<template>
  <footer class="footer">
    <span class="credits">Made with ❤️ and ☕ by w00zla &amp; Claude</span>
    <span class="spacer" />
    <span class="meta">
      <img class="logo" :src="logo" alt="" />
      <span v-if="version" class="version mono">v{{ version }}</span>
      <a class="repo" :href="REPO_URL" @click.prevent="openRepo">
        <Icon name="github" :size="14" fill="currentColor" />
        GitHub Page
      </a>
    </span>
  </footer>
</template>

<style scoped>
.footer {
  position: sticky;
  bottom: 0;
  z-index: 30;
  height: 28px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 16px;
  background: var(--bg-surface);
  border-top: 1px solid var(--border-dim);
  font-size: 12px;
  color: var(--text-3);
}

.spacer {
  flex: 1;
}

.meta {
  display: flex;
  align-items: center;
  gap: 10px;
}

.logo {
  width: 16px;
  height: 16px;
  display: block;
}

.version {
  color: var(--text-2);
}

.repo {
  margin-left: 10px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--text-2);
  text-decoration: none;
}

.repo:hover {
  color: var(--accent);
  text-decoration: underline;
}
</style>
