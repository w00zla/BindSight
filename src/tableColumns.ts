// Resizable, sortable column state for the grid-based tables (Bindings deck,
// Compare). Widths and the sort choice persist per table in localStorage.
import { computed, ref, watch } from "vue";

export type SortDir = "asc" | "desc";
export interface SortState {
  key: string;
  dir: SortDir;
}

export interface ColumnSpec {
  key: string;
  label: string;
  // Default width in px; null = the flexible filler column (never resized).
  width: number | null;
  sortable?: boolean;
}

const MIN_WIDTH = 40;

// Natural ordering: "button2" < "button10", case-insensitive.
export const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

interface Stored {
  widths?: Record<string, number>;
  sort?: SortState;
}

function load(storageKey: string): Stored {
  try {
    return JSON.parse(localStorage.getItem(storageKey) ?? "{}") as Stored;
  } catch {
    return {};
  }
}

const GAP = 12;

export function useTableColumns(storageKey: string, columns: ColumnSpec[], defaultSort: SortState) {
  const stored = load(storageKey);
  const widths = ref<Record<string, number>>({});
  for (const c of columns) {
    if (c.width === null) continue;
    const w = stored.widths?.[c.key];
    widths.value[c.key] = typeof w === "number" && w >= MIN_WIDTH ? w : c.width;
  }
  const storedSort = columns.some((c) => c.key === stored.sort?.key) ? stored.sort! : defaultSort;
  const sort = ref<SortState>({ ...storedSort });

  const template = computed(() =>
    columns.map((c) => (c.width === null ? "minmax(0, 1fr)" : `${widths.value[c.key]}px`)).join(" "),
  );
  // Fixed columns plus gaps: the row width below which the table scrolls
  // horizontally instead of squeezing the filler column further.
  const minWidth = computed(
    () => Object.values(widths.value).reduce((a, b) => a + b, 0) + GAP * (columns.length - 1),
  );

  function toggleSort(key: string) {
    sort.value = sort.value.key === key ? { key, dir: sort.value.dir === "asc" ? "desc" : "asc" } : { key, dir: "asc" };
  }

  // `fromLeft`: the drag moves the column's left edge, so it grows leftwards.
  function startResize(key: string, e: MouseEvent, fromLeft = false) {
    const startX = e.clientX;
    const startWidth = widths.value[key];
    if (startWidth === undefined) return;
    const sign = fromLeft ? -1 : 1;
    const move = (ev: MouseEvent) => {
      widths.value[key] = Math.max(MIN_WIDTH, Math.round(startWidth + sign * (ev.clientX - startX)));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.cursor = "";
    };
    document.body.style.cursor = "col-resize";
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  function resetWidth(key: string) {
    const c = columns.find((x) => x.key === key);
    if (c && c.width !== null) widths.value[key] = c.width;
  }

  watch(
    [widths, sort],
    () => {
      try {
        localStorage.setItem(storageKey, JSON.stringify({ widths: widths.value, sort: sort.value } satisfies Stored));
      } catch {
        // localStorage unavailable: widths simply do not persist.
      }
    },
    { deep: true },
  );

  return { widths, template, minWidth, sort, toggleSort, startResize, resetWidth };
}

// Stable sort by the current column; `valueOf` yields the cell value for a
// column key, `tieBreak` orders rows whose values compare equal.
export function sortRows<T>(
  rows: T[],
  sort: SortState,
  valueOf: (row: T, key: string) => string | number,
  tieBreak: (a: T, b: T) => number = () => 0,
): T[] {
  const dir = sort.dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const va = valueOf(a, sort.key);
    const vb = valueOf(b, sort.key);
    const c = typeof va === "number" && typeof vb === "number" ? va - vb : collator.compare(String(va), String(vb));
    return c * dir || tieBreak(a, b);
  });
}
