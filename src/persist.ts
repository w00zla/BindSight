// A ref mirrored into localStorage, for GUI choices that must survive a
// restart (filter chips, hidden columns). Falls back to `fallback` when
// nothing is stored or storage is unavailable.
import { ref, watch, type Ref } from "vue";

export function persistedRef<T>(storageKey: string, fallback: T): Ref<T> {
  let initial = fallback;
  try {
    const raw = localStorage.getItem(storageKey);
    if (raw !== null) initial = JSON.parse(raw) as T;
  } catch {
    // Unreadable or unavailable storage: start from the default.
  }
  const value = ref(initial) as Ref<T>;
  watch(
    value,
    (v) => {
      try {
        localStorage.setItem(storageKey, JSON.stringify(v));
      } catch {
        // Storage unavailable: the choice simply does not persist.
      }
    },
    { deep: true },
  );
  return value;
}
