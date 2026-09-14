<script setup lang="ts">
// The footer's mark while an update waits (before the app logo); a click
// on it opens the version dialog (listener from App.vue).
import Icon from "./Icon.vue";
</script>

<template>
  <span class="update-mark" title="Update available">
    <Icon name="download" :size="14" />
  </span>
</template>

<style scoped>
.update-mark {
  display: inline-flex;
  color: var(--accent);
  cursor: pointer;
}
</style>
