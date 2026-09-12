<script setup lang="ts">
// One row that never wraps: when its content is wider than the panel it
// scrolls sideways — mouse wheel over it, or the arrow buttons that appear
// at both ends. The scrollbar itself stays hidden.
import { onBeforeUnmount, onMounted, ref } from "vue";
import Icon from "./Icon.vue";

// How far one button click moves, as a fraction of the visible width.
const STEP = 0.6;

const scroller = ref<HTMLElement | null>(null);
const overflowing = ref(false);
const atStart = ref(true);
const atEnd = ref(true);

function update() {
  const el = scroller.value;
  if (!el) return;
  overflowing.value = el.scrollWidth > el.clientWidth + 1;
  atStart.value = el.scrollLeft <= 0;
  atEnd.value = el.scrollLeft + el.clientWidth >= el.scrollWidth - 1;
}

function scrollBy(direction: -1 | 1) {
  const el = scroller.value;
  if (!el) return;
  el.scrollBy({ left: direction * el.clientWidth * STEP, behavior: "smooth" });
}

// A vertical wheel over the rail scrolls it sideways (the row has no
// vertical extent to scroll); a horizontal wheel works as is.
function onWheel(e: WheelEvent) {
  const el = scroller.value;
  if (!el || !overflowing.value) return;
  const delta = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
  if (!delta) return;
  e.preventDefault();
  el.scrollLeft += delta;
}

let observer: ResizeObserver | null = null;
onMounted(() => {
  const el = scroller.value;
  if (!el) return;
  observer = new ResizeObserver(update);
  observer.observe(el);
  // The tiles come and go with hot-plugs: watch the content too.
  for (const child of Array.from(el.children)) observer.observe(child);
  new MutationObserver(() => {
    observer?.disconnect();
    observer?.observe(el);
    for (const child of Array.from(el.children)) observer?.observe(child);
    update();
  }).observe(el, { childList: true });
  update();
});
onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div class="scroll-rail" :class="{ overflowing }">
    <button v-if="overflowing" type="button" class="edge left" :disabled="atStart" title="Scroll left" @click="scrollBy(-1)">
      <Icon name="chevron-left" :size="16" />
    </button>
    <div ref="scroller" class="scroller" @wheel="onWheel" @scroll="update">
      <slot />
    </div>
    <button v-if="overflowing" type="button" class="edge right" :disabled="atEnd" title="Scroll right" @click="scrollBy(1)">
      <Icon name="chevron-right" :size="16" />
    </button>
  </div>
</template>

<style scoped>
.scroll-rail {
  display: flex;
  align-items: stretch;
  gap: 8px;
  min-width: 0;
}

.scroller {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: flex-start;
  gap: 12px;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: none;
}

.scroller::-webkit-scrollbar {
  display: none;
}

/* Narrow full-height strips at both ends, in the tile's look. */
.edge {
  flex-shrink: 0;
  width: 24px;
  padding: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--bg-surface-2);
  color: var(--text-2);
  cursor: pointer;
}

.edge:hover:not(:disabled) {
  color: var(--accent);
  border-color: var(--accent);
}

.edge:disabled {
  opacity: 0.35;
  cursor: default;
}
</style>
