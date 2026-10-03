<script setup lang="ts">
// The game's curve dialog: one curve value, an exponent or a custom point
// list. The exponent slider and the number field set the exponent (and drop
// any points, like the game); a click on the grid turns the curve into the
// game's grid points (in = 0, 0.1 … 1 on x^exp) plus the new one, points
// drag in both directions, the end points stay, nothing is deleted (the game
// cannot either). Closes only via Cancel / Apply or Escape.
import { computed, onMounted, ref } from "vue";
import ConfirmDialog from "./ConfirmDialog.vue";
import { EXP_MAX, EXP_MIN, EXP_STEP, MAX_POINTS, curvePath, evalCurve, gridPoints, roundExp, type Curve, type Pt } from "../configModel";

const props = defineProps<{
  title: string;
  subtitle: string;
  curve: Curve;
  // The live input's position (0..1) on this device, null when none is
  // known; it puts a dot on the curve.
  liveX: number | null;
}>();
const emit = defineEmits<{ apply: [curve: Curve]; cancel: [] }>();

const W = 200;
const P = 12;

const cur = ref<Curve>("pts" in props.curve ? { pts: props.curve.pts.map((p) => [p[0], p[1]] as Pt) } : { exp: props.curve.exp });
const numText = ref("");
const numFocused = ref(false);
const svg = ref<SVGSVGElement | null>(null);
let drag = -1;

// The slider's position: the exponent, or 1 while points are shown.
const sliderValue = computed(() => ("exp" in cur.value ? cur.value.exp : 1));
const sliderFill = computed(() => `${((sliderValue.value - EXP_MIN) / (EXP_MAX - EXP_MIN)) * 100}%`);

function showNumber() {
  if (numFocused.value) return;
  numText.value = "exp" in cur.value ? cur.value.exp.toFixed(1) : "";
}
onMounted(showNumber);

function setExp(v: number) {
  cur.value = { exp: v };
  showNumber();
}

function onSlider(e: Event) {
  setExp(Number((e.target as HTMLInputElement).value));
}

// Typing previews any value in range; Enter / leaving the field clamps it
// and rounds it to the game's 0.1 steps. A comma counts as the point.
function parseNum(): number | null {
  const v = parseFloat(numText.value.replace(",", "."));
  return Number.isFinite(v) ? v : null;
}

function onNumInput() {
  const v = parseNum();
  if (v !== null && v >= EXP_MIN && v <= EXP_MAX) cur.value = { exp: v };
}

function onNumCommit() {
  const v = parseNum();
  numFocused.value = false;
  if (v !== null) setExp(roundExp(v));
  else showNumber();
}

const toSvg = (x: number, y: number): [number, number] => [P + x * (W - 2 * P), W - P - y * (W - 2 * P)];

function fromEvent(e: PointerEvent): Pt {
  const r = svg.value!.getBoundingClientRect();
  const sx = ((e.clientX - r.left) / r.width) * W;
  const sy = ((e.clientY - r.top) / r.height) * W;
  return [Math.min(1, Math.max(0, (sx - P) / (W - 2 * P))), Math.min(1, Math.max(0, (W - P - sy) / (W - 2 * P)))];
}

const grid = computed(() => {
  const out: string[] = [];
  for (let i = 0; i <= 10; i++) {
    const c = P + (i * (W - 2 * P)) / 10;
    out.push(`M${c} ${P}V${W - P}M${P} ${c}H${W - P}`);
  }
  return out.join("");
});

const path = computed(() => curvePath(cur.value, W, P));

// At the point limit a click on the grid adds nothing; the points still drag.
const full = computed(() => "pts" in cur.value && cur.value.pts.length >= MAX_POINTS);

const points = computed(() =>
  "pts" in cur.value
    ? cur.value.pts.map(([x, y], i, all) => {
        const [cx, cy] = toSvg(x, y);
        return { cx, cy, end: i === 0 || i === all.length - 1 };
      })
    : [],
);

const live = computed(() => {
  if (props.liveX === null) return null;
  const x = Math.min(1, Math.max(0, props.liveX));
  const [lx, ly] = toSvg(x, evalCurve(cur.value, x));
  return { lx, ly, line: `M${lx} ${W - P}V${ly}H${P}` };
});

function onDown(e: PointerEvent) {
  const t = (e.target as Element).closest("[data-p]");
  if (!("pts" in cur.value)) cur.value = { pts: gridPoints(cur.value.exp) };
  const pts = (cur.value as { pts: Pt[] }).pts;
  if (t) {
    const i = Number(t.getAttribute("data-p"));
    if (i === 0 || i === pts.length - 1) return;
    drag = i;
  } else {
    if (pts.length >= MAX_POINTS) return;
    const p = fromEvent(e);
    p[0] = Math.min(0.999, Math.max(0.001, p[0]));
    pts.push(p);
    pts.sort((a, b) => a[0] - b[0]);
    drag = pts.indexOf(p);
  }
  showNumber();
  svg.value?.setPointerCapture(e.pointerId);
}

function onMove(e: PointerEvent) {
  if (drag < 0 || !("pts" in cur.value)) return;
  const pts = cur.value.pts;
  const [x, y] = fromEvent(e);
  const moved: Pt = [Math.min(0.999, Math.max(0.001, x)), y];
  pts[drag] = moved;
  pts.sort((a, b) => a[0] - b[0]);
  drag = pts.indexOf(moved);
}

function onUp() {
  drag = -1;
}

function onChoose(value: string) {
  if (value !== "apply") {
    emit("cancel");
    return;
  }
  const c = cur.value;
  emit("apply", "pts" in c ? { pts: c.pts.map((p) => [Number(p[0].toFixed(4)), Number(p[1].toFixed(4))] as Pt) } : { exp: roundExp(c.exp) });
}
</script>

<template>
  <ConfirmDialog
    :title="title"
    :subtitle="subtitle"
    icon="curve"
    :width="460"
    :buttons="[
      { label: 'Apply', kind: 'primary', value: 'apply' },
      { label: 'Cancel', kind: 'outline', value: 'cancel' },
    ]"
    @choose="onChoose"
  >
    <svg
      ref="svg"
      class="canvas"
      :class="{ full }"
      :viewBox="`0 0 ${W} ${W}`"
      @pointerdown="onDown"
      @pointermove="onMove"
      @pointerup="onUp"
      @pointercancel="onUp"
    >
      <path class="grid" :d="grid" />
      <path class="diag" :d="`M${P} ${W - P}L${W - P} ${P}`" />
      <path class="curve" :d="path" />
      <circle
        v-for="(p, i) in points"
        :key="i"
        :data-p="i"
        class="pt"
        :class="{ end: p.end }"
        :cx="p.cx"
        :cy="p.cy"
        :r="p.end ? 3 : 4.5"
      />
      <template v-if="live">
        <path class="live-line" :d="live.line" />
        <circle class="live-dot" :cx="live.lx" :cy="live.ly" r="4" />
      </template>
    </svg>
    <div class="exp-line">
      <span>Exponent</span>
      <input
        class="range"
        type="range"
        :min="EXP_MIN"
        :max="EXP_MAX"
        :step="EXP_STEP"
        :value="sliderValue"
        :style="{ '--p': sliderFill }"
        aria-label="Exponent"
        @input="onSlider"
      />
      <input
        v-model="numText"
        class="exp-num mono"
        type="text"
        inputmode="decimal"
        :placeholder="'pts' in cur ? '—' : ''"
        aria-label="Exponent"
        @focus="numFocused = true"
        @input="onNumInput"
        @blur="onNumCommit"
        @keydown.enter.prevent="($event.target as HTMLInputElement).blur()"
      />
    </div>
  </ConfirmDialog>
</template>

<style scoped>
.canvas {
  display: block;
  width: 100%;
  aspect-ratio: 1;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: 6px;
  touch-action: none;
  cursor: crosshair;
}

.canvas.full {
  cursor: not-allowed;
}

.grid {
  stroke: var(--border-dim);
  stroke-width: 1;
  fill: none;
}

.diag {
  stroke: var(--text-3);
  stroke-dasharray: 3 4;
  fill: none;
}

.curve {
  stroke: var(--accent);
  stroke-width: 2;
  fill: none;
}

.pt {
  fill: var(--bg-surface);
  stroke: var(--accent);
  stroke-width: 1.5;
  cursor: grab;
}

.pt.end {
  fill: var(--text-3);
  cursor: default;
}

.live-line {
  stroke: var(--live);
  stroke-opacity: 0.35;
  fill: none;
  pointer-events: none;
}

.live-dot {
  fill: var(--live);
  pointer-events: none;
}

.exp-line {
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr) 64px;
  gap: 10px;
  align-items: center;
  font-size: 13px;
  color: var(--text-2);
}

.exp-num {
  width: 100%;
  height: 28px;
  padding: 0 8px;
  box-sizing: border-box;
  border-radius: var(--radius-control);
  border: 1px solid var(--border);
  background: var(--bg-surface-2);
  color: var(--text);
  font-size: 13px;
  text-align: right;
  outline: none;
}

.exp-num:focus {
  border-color: var(--accent);
}
</style>
