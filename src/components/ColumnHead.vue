<script setup lang="ts">
// Header row of a grid table: click a column to sort, drag the grip at its
// right edge to resize, double-click the grip to reset the width. The filler
// column absorbs every change, so a grip resizes the column on the far side
// of the filler: left of it the column itself, right of it (and after the
// filler) the next column from its left edge; the last column's grip is its
// own. The root carries the parent's `.row.cols-head` styling; the grid
// template comes from the parent's `--cols` variable.
import Icon, { type IconName } from "./Icon.vue";
import type { ColumnSpec, SortState } from "../tableColumns";

const props = defineProps<{ columns: ColumnSpec[]; sort: SortState }>();
const emit = defineEmits<{ sort: [key: string]; resize: [key: string, e: MouseEvent, fromLeft: boolean]; reset: [key: string] }>();

// The column the grip after column i resizes, see above.
function gripTarget(i: number): { key: string; fromLeft: boolean } | null {
  const filler = props.columns.findIndex((c) => c.width === null);
  const c = props.columns[i];
  const next = props.columns[i + 1];
  if (filler === -1 || i < filler) return { key: c.key, fromLeft: false };
  if (next) return { key: next.key, fromLeft: true };
  return c.width === null ? null : { key: c.key, fromLeft: false };
}
</script>

<template>
  <div class="row cols-head">
    <span
      v-for="(c, i) in columns"
      :key="c.key"
      class="col"
      :class="{ sortable: c.sortable !== false, active: sort.key === c.key }"
      @click="c.sortable !== false && emit('sort', c.key)"
    >
      <Icon v-if="c.icon" :name="c.icon as IconName" :size="13" class="col-icon" />
      <span class="txt">{{ c.label }}</span>
      <span v-if="c.note" class="txt note">{{ c.note }}</span>
      <Icon v-if="sort.key === c.key" :name="sort.dir === 'asc' ? 'chevron-up' : 'chevron-down'" :size="12" />
      <span
        v-if="gripTarget(i)"
        class="grip"
        @mousedown.stop.prevent="emit('resize', gripTarget(i)!.key, $event, gripTarget(i)!.fromLeft)"
        @dblclick.stop="emit('reset', gripTarget(i)!.key)"
        @click.stop
      />
    </span>
  </div>
</template>

<style scoped>
.col {
  position: relative;
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  user-select: none;
}

.col.sortable {
  cursor: pointer;
}

.col.sortable:hover {
  color: var(--text);
}

.col-icon {
  flex-shrink: 0;
}

.txt {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* The warning stays whole; the label gives way. */
.note {
  flex-shrink: 0;
  color: var(--warn);
}


/* Sits in the 12px column gap, right of the cell. */
.grip {
  position: absolute;
  top: -8px;
  bottom: -8px;
  right: -9px;
  width: 9px;
  cursor: col-resize;
}

.grip::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 4px;
  width: 1px;
  background: var(--border);
}

.grip:hover::after {
  left: 3px;
  width: 2px;
  background: var(--accent);
}
</style>
