<script setup lang="ts">
// A curve's shape in a small box: the custom curves of the option table and
// of Compare, which have no single number to show.
import { computed } from "vue";
import { curvePath, type Curve } from "../configModel";

const props = withDefaults(defineProps<{ curve: Curve; dim?: boolean; size?: number }>(), { dim: false, size: 20 });
const d = computed(() => curvePath(props.curve, 20, 2));
</script>

<template>
  <svg class="thumb" :class="{ dim }" :width="size" :height="size" viewBox="0 0 20 20" aria-hidden="true">
    <path :d="d" />
  </svg>
</template>

<style scoped>
.thumb {
  flex-shrink: 0;
}

.thumb path {
  fill: none;
  stroke: var(--accent);
  stroke-width: 1.6;
}

.thumb.dim path {
  stroke: var(--text-3);
}
</style>
