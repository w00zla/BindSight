<script setup lang="ts">
interface Toast {
  id: number;
  message: string;
  type: "ok" | "error" | "hint";
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
  bottom: 24px;
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

.toast.error,
.toast.hint {
  border-left-color: var(--err);
}
</style>
