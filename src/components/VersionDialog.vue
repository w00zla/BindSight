<script setup lang="ts">
// The App Update dialog (footer: version, logo or update mark): the
// running version and, in an install with an updater, the update state —
// one chip per row, the download progress, the updater's buttons. Without
// an updater (bare executable, deb / rpm) only the Current row.
import { computed } from "vue";
import ConfirmDialog, { type ConfirmButton } from "./ConfirmDialog.vue";
import type { Updater } from "../update";
import type { UpdateChannel } from "../types";

// `channel`: shown as a chip next to the title unless it is the stable one.
const props = defineProps<{ version: string; updater: Updater | null; channel: UpdateChannel }>();
const emit = defineEmits<{ close: [] }>();

const buttons = computed<ConfirmButton[]>(() => [
  { label: "Close", kind: "outline", value: "close", side: "left" },
  ...(props.updater?.buttons() ?? []),
]);

// The available version once one is known, else a dash.
const update = computed(() => {
  const u = props.updater;
  if (!u) return null;
  if (u.info) return { text: `v${u.info.version}`, kind: "accent" };
  return { text: "–", kind: "dim" };
});

const percent = computed(() => {
  const p = props.updater?.progress ?? null;
  return p === null ? null : Math.round(p * 100);
});

// What the updater is doing right now, on its own line under the versions
// (the line keeps its height while empty, so the dialog never jumps); the
// download shares it with the progress bar.
const status = computed(() => {
  const u = props.updater;
  if (!u) return { text: "", kind: "" };
  switch (u.state) {
    case "checking":
      return { text: "Checking…", kind: "" };
    case "downloading":
      return { text: `Downloading… ${percent.value === null ? "" : `${percent.value} %`}`, kind: "busy" };
    case "installing":
      return { text: "Installing…", kind: "busy" };
    case "error":
      return { text: u.error, kind: "err" };
    default:
      return { text: "", kind: "" };
  }
});

function choose(value: string) {
  if (value === "close") emit("close");
  else if (value === "check") void props.updater?.check();
  else if (value === "install") void props.updater?.install();
}
</script>

<template>
  <ConfirmDialog
    title="App Update"
    :badge="channel === 'stable' ? undefined : 'Pre-Release'"
    icon="download"
    :buttons="buttons"
    @choose="choose"
  >
    <div class="rows">
      <div class="row">
        <span class="label">Current Version:</span>
        <span class="chip mono">v{{ version }}</span>
      </div>
      <div v-if="update" class="row">
        <span class="label">Available Version:</span>
        <span class="chip mono" :class="update.kind">{{ update.text }}</span>
      </div>
    </div>
    <template v-if="updater">
      <div class="status" :class="status.kind">
        <span class="status-text" :title="status.text">{{ status.text }}</span>
        <div v-if="updater.state === 'downloading'" class="progress">
          <div class="bar" :class="{ unknown: percent === null }" :style="{ width: `${percent ?? 0}%` }" />
        </div>
      </div>
      <button v-if="updater.simulated !== null" type="button" class="dev" @click="updater.simulate(!updater.simulated)">
        {{ updater.simulated ? "Dev: reset" : "Dev: simulate an update" }}
      </button>
    </template>
  </ConfirmDialog>
</template>

<style scoped>
.rows {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.row {
  display: flex;
  align-items: center;
  gap: 14px;
}

.label {
  width: 130px;
  font-size: 14px;
  color: var(--text-2);
}

.chip {
  font-size: 15px;
  color: var(--text);
}

.chip.dim {
  color: var(--text-2);
}

.chip.accent {
  color: var(--accent);
  font-weight: 600;
}

.status {
  min-height: 18px;
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 13px;
  color: var(--text-3);
}

.status-text {
  flex-shrink: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.status.busy {
  color: var(--warn);
}

.status.err {
  color: var(--err);
}

.progress {
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background: var(--border-dim);
  overflow: hidden;
}

.bar {
  height: 100%;
  background: var(--accent);
}

.bar.unknown {
  width: 30% !important;
  animation: slide 1s ease-in-out infinite alternate;
}

@keyframes slide {
  from {
    margin-left: 0;
  }
  to {
    margin-left: 70%;
  }
}

.dev {
  align-self: flex-start;
  height: 28px;
  padding: 0 10px;
  border: 1px dashed var(--warn);
  border-radius: var(--radius-control);
  background: transparent;
  font: inherit;
  font-size: 12px;
  color: var(--warn);
  cursor: pointer;
}
</style>
