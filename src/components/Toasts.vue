<script setup lang="ts">
import type { ToastType } from "../types";

interface Toast {
  id: number;
  message: string;
  type: ToastType;
}

defineProps<{ toasts: Toast[] }>();
</script>

<template>
  <div class="toasts">
    <div v-for="t in toasts" :key="t.id" class="toast" :class="t.type">
      {{ t.message }}
    </div>
  </div>
</template>

<style scoped>
.toasts {
  position: fixed;
  /* Clear of the 28px footer. */
  bottom: 40px;
  right: 24px;
  display: flex;
  flex-direction: column-reverse;
  gap: 8px;
  z-index: 1000;
}

.toast {
  background: var(--bg-surface-2);
  border-radius: 6px;
  padding: 10px 14px;
  font-size: 13px;
  color: var(--text);
  border-left: 3px solid var(--ok);
  max-width: 24rem;
}

/* The accent says what kind of message it is: green done, red broken,
   amber degraded, blue guidance. */
.toast.error {
  border-left-color: var(--err);
}

.toast.warn {
  border-left-color: var(--warn);
}

.toast.hint {
  border-left-color: var(--accent);
}
</style>
