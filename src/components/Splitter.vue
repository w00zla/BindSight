<script setup lang="ts">
// A draggable gutter. Emits the pointer offset from drag start in px on every
// move (the parent decides what to resize), `end` on release and `reset` on
// double-click.
const props = defineProps<{ direction: "col" | "row" }>();
const emit = defineEmits<{ drag: [delta: number]; end: []; reset: [] }>();

function start(e: PointerEvent) {
  const target = e.currentTarget as HTMLElement;
  const origin = props.direction === "col" ? e.clientX : e.clientY;
  target.setPointerCapture(e.pointerId);
  const move = (ev: PointerEvent) => emit("drag", (props.direction === "col" ? ev.clientX : ev.clientY) - origin);
  const up = () => {
    target.removeEventListener("pointermove", move);
    target.removeEventListener("pointerup", up);
    target.removeEventListener("pointercancel", up);
    emit("end");
  };
  target.addEventListener("pointermove", move);
  target.addEventListener("pointerup", up);
  target.addEventListener("pointercancel", up);
}
</script>

<template>
  <div class="splitter" :class="direction" @pointerdown="start" @dblclick="emit('reset')" />
</template>

<style scoped>
.splitter {
  flex: 0 0 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  touch-action: none;
}

.splitter.col {
  cursor: col-resize;
}

.splitter.row {
  cursor: row-resize;
}

.splitter::before {
  content: "";
  border-radius: 1px;
  background: var(--border);
}

.splitter.col::before {
  width: 2px;
  height: 40px;
}

.splitter.row::before {
  width: 40px;
  height: 2px;
}

.splitter:hover::before {
  background: var(--accent);
}
</style>
