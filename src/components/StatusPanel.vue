<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import Icon from "./Icon.vue";
import ConfirmDialog from "./ConfirmDialog.vue";
import ConsoleCommandDialog from "./ConsoleCommandDialog.vue";
import type { ClashReport, ScStatus } from "../types";

const props = defineProps<{ report: ClashReport | null; loadError: string | null; sc: ScStatus | null }>();
const emit = defineEmits<{ apply: []; copy: [command: string] }>();

// Fix via config overwrites the game's actionmaps.xml: ask first.
const confirmApply = ref(false);

// Fix via console: the swaps as one console line. The engine's console splits
// a line on ";" (Lumberyard XConsole), so SC runs them in order.
const showConsole = ref(false);
const consoleCommand = computed(() => (props.report?.resort_commands ?? []).join("; "));

function onConfirmApply(value: string) {
  confirmApply.value = false;
  if (value === "rewrite") emit("apply");
}

// When the game last listed its joysticks (the game log's own time); null
// without a usable log.
const orderDate = computed<Date | null>(() => {
  const raw = props.report?.log_timestamp;
  const t = raw ? new Date(raw) : null;
  return t && !Number.isNaN(t.getTime()) ? t : null;
});
// "6 mins ago": re-evaluated every half minute so it does not go stale.
const now = ref(Date.now());
const ticker = setInterval(() => (now.value = Date.now()), 30_000);
onUnmounted(() => clearInterval(ticker));
const orderAgo = computed(() => {
  const t = orderDate.value;
  if (!t) return "unknown";
  const s = Math.max(0, Math.floor((now.value - t.getTime()) / 1000));
  const unit = (n: number, name: string) => `${n} ${name}${n === 1 ? "" : "s"} ago`;
  if (s < 60) return "just now";
  if (s < 3600) return unit(Math.floor(s / 60), "min");
  if (s < 86400) return unit(Math.floor(s / 3600), "hour");
  return unit(Math.floor(s / 86400), "day");
});
// The exact time goes into the tooltip.
const orderTitle = computed(() => {
  const explain = "When the game last listed its joysticks (js1, js2, …) at a start. A running game keeps that order until it restarts.";
  const t = orderDate.value;
  return t ? `${explain}\n${t.toLocaleString()}` : explain;
});

// The devices behind the order difference: what the game had at its start
// and no longer has (missing), and what it did not have (new). Matched by
// GUID against the live order.
const orderChanges = computed(() => {
  const logged = props.report?.logged_order;
  if (!logged) return [];
  const key = (g: string | null) => g?.toLowerCase() ?? "";
  const live = new Set((props.report?.connected ?? []).map((s) => key(s.sc_product_guid)));
  const started = new Set(logged.joysticks.map((j) => key(j.product_guid)));
  return [
    ...logged.joysticks
      .filter((j) => !live.has(key(j.product_guid)))
      .map((j) => ({ id: `gone-${j.instance}`, name: j.product_name, instance: j.instance, state: "missing" })),
    ...(props.report?.connected ?? [])
      .filter((s) => !started.has(key(s.sc_product_guid)))
      .map((s) => ({ id: `new-${s.effective_instance}`, name: s.name ?? "?", instance: s.effective_instance, state: "new" })),
  ];
});

const hasIssue = computed(
  () =>
    !!props.sc?.error ||
    !!props.loadError ||
    !!props.report?.order_error ||
    !!props.report?.logged_order ||
    !!props.report?.has_clash,
);
</script>

<template>
  <div class="status-panel">
    <div class="head">
      <div class="panel-title">Status</div>
      <div class="order-time" :title="orderTitle">
        <Icon name="clock" :size="14" />
        <span>Game devices update <b>{{ orderAgo }}</b></span>
      </div>
    </div>
    <div class="tiles">
      <div v-if="sc?.loading" class="tile loading">
        <div class="row">
          <Icon name="clock" :size="16" />
          <span>Reading game data…</span>
        </div>
        <div class="steps">
          <span v-for="i in sc.steps" :key="i" class="step" :class="{ done: i <= sc.progress }" />
        </div>
      </div>

      <div v-else-if="!hasIssue" class="tile ok">
        <Icon name="check" :size="16" />
        <span>No issues</span>
      </div>

      <div v-if="sc?.error" class="tile error detail">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">No game data</span>
        </div>
        <div class="error mono">{{ sc.error }}</div>
      </div>

      <div v-if="loadError" class="tile error detail">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">No bindings</span>
        </div>
        <div class="error mono">{{ loadError }}</div>
      </div>

      <!-- Without the game's joystick order no joystick input resolves. -->
      <div v-if="report?.order_error" class="tile issue detail">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">No joystick order</span>
        </div>
        <div class="note">{{ report.order_error }}</div>
      </div>

      <!-- The game started with another order (a device plugged in or out
           since): it keeps that order until it restarts. -->
      <div v-if="report?.logged_order" class="tile issue detail">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">Game started with another device order</span>
        </div>
        <div class="moves">
          <div v-for="c in orderChanges" :key="c.id" class="move">
            <span class="dot filled" />
            <span class="name">{{ c.name }}</span>
            <span class="chip mono">js{{ c.instance }}</span>
            <span>{{ c.state }}</span>
          </div>
        </div>
        <div class="note">If game is running, restart for changes to take effect</div>
      </div>

      <div v-if="report?.has_clash" class="tile issue clash">
        <div class="row">
          <Icon name="warning" :size="16" />
          <span class="name">Joystick order clash!</span>
          <span class="spacer" />
          <button type="button" class="fix-btn" @click="confirmApply = true">
            <Icon name="file" :size="14" />
            Fix via Config
          </button>
          <button type="button" class="copy-btn" @click="showConsole = true">
            <Icon name="terminal" :size="14" />
            Fix via Console
          </button>
        </div>
        <!-- Moves without a saved device only shuffle empty slots to close the
             swap cycle: nothing to show, the console commands still carry them. -->
        <div class="moves">
          <div v-for="m in report.resort.filter((r) => r.name)" :key="m.from" class="move">
            <span class="dot filled" />
            <span class="name">{{ m.name }}</span>
            <span class="chip mono">js{{ m.from }}</span>
            <Icon name="arrow-right" :size="16" />
            <span class="chip mono">js{{ m.to }}</span>
          </div>
        </div>
      </div>
    </div>

    <ConfirmDialog
      v-if="confirmApply"
      title="Rewrite bindings configuration?"
      icon="file"
      :buttons="[
        { label: 'Rewrite', kind: 'primary', value: 'rewrite' },
        { label: 'Cancel', kind: 'outline', value: 'cancel' },
      ]"
      @choose="onConfirmApply"
    >
      <p class="dialog-note">If game is running, restart for changes to take effect</p>
    </ConfirmDialog>

    <ConsoleCommandDialog v-if="showConsole" :command="consoleCommand" @close="showConsole = false" @copy="emit('copy', $event)" />
  </div>
</template>

<style scoped>
.status-panel {
  background: var(--bg-surface);
  border-radius: var(--radius-panel);
  padding: 12px 16px 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

/* The log's enumeration time, top right of the panel. */
.order-time {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-3);
  white-space: nowrap;
}

.order-time b {
  font-weight: 600;
  color: var(--text-2);
}

.tiles {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}

/* Child tiles share the device tile footprint (84px high). */
.tile {
  min-width: 240px;
  min-height: 84px;
  box-sizing: border-box;
  padding: 12px 16px;
  border-radius: var(--radius-panel);
  background: var(--bg-surface-2);
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
  color: var(--text);
}

.tile.ok {
  flex: 1;
  color: var(--ok);
}

.tile.issue {
  background: rgba(242, 179, 76, 0.1);
  border: 1px solid rgba(242, 179, 76, 0.5);
  color: var(--warn);
}

/* Hard failures (no game data at all), as opposed to warnings. */
.tile.error {
  background: rgba(255, 92, 108, 0.1);
  border: 1px solid rgba(255, 92, 108, 0.5);
  color: var(--err);
}

.tile.clash {
  flex-direction: column;
  align-items: stretch;
  justify-content: center;
  gap: 8px;
  min-width: 480px;
}

.tile.loading,
.tile.detail {
  flex-direction: column;
  align-items: stretch;
  justify-content: center;
  gap: 8px;
}

.tile.detail {
  max-width: 480px;
}

/* Segmented progress: one cell per load step, filled as they complete. */
.steps {
  display: flex;
  gap: 4px;
}

.step {
  flex: 1;
  height: 4px;
  border-radius: 2px;
  background: var(--bg-surface-3);
  transition: background 150ms;
}

.step.done {
  background: var(--accent);
}

.error {
  font-size: 12px;
  color: var(--text);
  overflow-wrap: anywhere;
}

.note {
  font-size: 12px;
  color: var(--text-2);
}

.row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.name {
  font-weight: 600;
  color: var(--text);
}

.spacer {
  flex: 1;
}

/* Passive slot tokens, like the device tile's clash chip: a tinted fill, no
   border — the outline belongs to the buttons. */
.chip {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 3px;
  background: rgba(242, 179, 76, 0.15);
  color: var(--warn);
  font-size: 13px;
}

.fix-btn,
.copy-btn {
  height: var(--h-chip-sm);
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  border-radius: var(--radius-control);
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  cursor: pointer;
}

.fix-btn {
  background: var(--warn);
  color: #000;
  border: none;
}

.copy-btn {
  background: transparent;
  color: var(--warn);
  border: 1px solid rgba(242, 179, 76, 0.6);
}

.moves {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  padding-top: 4px;
}

.move {
  display: flex;
  align-items: center;
  gap: 8px;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  border: 1px solid rgba(242, 179, 76, 0.6);
  box-sizing: border-box;
}

.dot.filled {
  background: var(--warn);
  border-color: var(--warn);
}

/* Fix via config dialog body (slot content of ConfirmDialog). */
.dialog-note {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
}

</style>
