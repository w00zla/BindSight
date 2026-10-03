// Config mode logic without any GUI: the option tree, curves, the pending
// settings changes applied the way the game applies them, and the Compare
// rows. Values are in the game's display units throughout; the backend
// converts to what the files store.

import type {
  ConfigChange,
  ConfigLabels,
  CurveValue,
  DeviceConfigView,
  DeviceKind,
  DeviceOptionsView,
  MouseView,
  NodeValue,
  OptionNode,
  OptionTree,
  OptionsBlock,
} from "./types";

export type Pt = [number, number];

// A curve as the GUI draws it: an exponent (y = x^exp) or a point list.
export type Curve = { exp: number } | { pts: Pt[] };

// The game's exponent slider: 0.1 – 3.0 in 0.1 steps, typed input too.
export const EXP_MIN = 0.1;
export const EXP_MAX = 3;
export const EXP_STEP = 0.1;
// The game's curve dialog takes points without a limit; the backend takes 64.
export const MAX_POINTS = 64;

export function roundExp(v: number): number {
  return Math.round(Math.min(EXP_MAX, Math.max(EXP_MIN, v)) * 10) / 10;
}

// The GUI step of deadzone, saturation and the 0..1 sliders.
export function r2(v: number): number {
  return Math.round(v * 100) / 100;
}

// --- curves ------------------------------------------------------------------

// A node's own curve (an own point list wins over an exponent), or null.
export function ownCurve(v: NodeValue | null | undefined): Curve | null {
  if (v?.points) return { pts: v.points };
  if (v?.exponent !== null && v?.exponent !== undefined) return { exp: v.exponent };
  return null;
}

// The tree's default curve on a node, or null.
export function defaultCurve(n: OptionNode): Curve | null {
  if (n.default_points) return { pts: withEnds(n.default_points) };
  if (n.default_exponent !== null) return { exp: n.default_exponent };
  return null;
}

// Points sorted by `in`, with 0/0 and 1/1 present (the tree's default lists
// leave them out; the file and the backend always have them).
export function withEnds(pts: Pt[]): Pt[] {
  const out = pts.map((p) => [p[0], p[1]] as Pt).sort((a, b) => a[0] - b[0]);
  if (!out.length || out[0][0] > 0) out.unshift([0, 0]);
  if (out[out.length - 1][0] < 1) out.push([1, 1]);
  return out;
}

export function sameCurve(a: Curve | null, b: Curve | null): boolean {
  if (!a || !b) return a === b;
  if ("exp" in a && "exp" in b) return Math.abs(a.exp - b.exp) < 1e-4;
  if ("pts" in a && "pts" in b) {
    return a.pts.length === b.pts.length && a.pts.every((p, i) => Math.abs(p[0] - b.pts[i][0]) < 1e-6 && Math.abs(p[1] - b.pts[i][1]) < 1e-6);
  }
  return false;
}

export function curveValue(c: Curve | null): CurveValue {
  if (!c) return { kind: "default" };
  return "exp" in c ? { kind: "exponent", value: c.exp } : { kind: "points", points: c.pts };
}

// The game smooths a point list into a curve; how exactly is unknown, a
// Catmull-Rom spline through the points comes close.
export function splineSamples(p: Pt[], per = 12): Pt[] {
  const out: Pt[] = [p[0]];
  for (let i = 0; i < p.length - 1; i++) {
    const p0 = p[Math.max(0, i - 1)];
    const p1 = p[i];
    const p2 = p[i + 1];
    const p3 = p[Math.min(p.length - 1, i + 2)];
    for (let k = 1; k <= per; k++) {
      const t = k / per;
      const t2 = t * t;
      const t3 = t2 * t;
      const f = (a: number, b: number, c: number, d: number) =>
        0.5 * (2 * b + (-a + c) * t + (2 * a - 5 * b + 4 * c - d) * t2 + (-a + 3 * b - 3 * c + d) * t3);
      out.push([f(p0[0], p1[0], p2[0], p3[0]), Math.min(1.2, Math.max(-0.2, f(p0[1], p1[1], p2[1], p3[1])))]);
    }
  }
  return out;
}

// The curve's output at input x (0..1).
export function evalCurve(c: Curve, x: number): number {
  if ("exp" in c) return Math.pow(x, c.exp);
  const p = splineSamples(c.pts);
  for (let i = 1; i < p.length; i++) {
    if (x <= p[i][0]) {
      const [x0, y0] = p[i - 1];
      const [x1, y1] = p[i];
      return x1 === x0 ? y1 : y0 + ((y1 - y0) * (x - x0)) / (x1 - x0);
    }
  }
  return p[p.length - 1][1];
}

// SVG path of a curve in a `size` x `size` box with `pad` around the plot.
export function curvePath(c: Curve, size: number, pad: number): string {
  const xy = (x: number, y: number) => `${(pad + x * (size - 2 * pad)).toFixed(1)},${(size - pad - y * (size - 2 * pad)).toFixed(1)}`;
  if ("pts" in c) return "M" + splineSamples(c.pts).map(([x, y]) => xy(x, y)).join("L");
  const pts: string[] = [];
  for (let i = 0; i <= 40; i++) pts.push(xy(i / 40, Math.pow(i / 40, c.exp)));
  return "M" + pts.join("L");
}

// The game's grid for a custom curve: in = 0, 0.1 … 1 on x^exp.
export function gridPoints(exp: number): Pt[] {
  return Array.from({ length: 11 }, (_, i) => [i / 10, Math.pow(i / 10, exp)] as Pt);
}

// --- option tree ---------------------------------------------------------------

export function isGroup(n: OptionNode): boolean {
  return n.children.length > 0;
}

// The label the game shows; the raw name only where it has none.
export function nodeLabel(n: OptionNode): string {
  return n.label || n.name;
}

// Pre-order, the order the game writes the file's children in.
export function preorder(nodes: OptionNode[], out: OptionNode[] = []): OptionNode[] {
  for (const n of nodes) {
    out.push(n);
    preorder(n.children, out);
  }
  return out;
}

// A node and the labels of the groups above it.
export function findNode(nodes: OptionNode[], name: string, path: string[] = []): { node: OptionNode; path: string[] } | null {
  for (const n of nodes) {
    if (n.name === name) return { node: n, path };
    const hit = findNode(n.children, name, [...path, nodeLabel(n)]);
    if (hit) return hit;
  }
  return null;
}

// Effective curve of a node: its own value, else the tree default on the
// node, else the nearest group's effective value, else exponent 1.
export function effectiveCurve(n: OptionNode, values: Record<string, NodeValue>, parent: Curve | null): Curve {
  return ownCurve(values[n.name]) ?? defaultCurve(n) ?? parent ?? { exp: 1 };
}

// The effective curve handed down to a node's children: a node without a
// curve control passes its parent's on.
export function curveForChildren(n: OptionNode, values: Record<string, NodeValue>, parent: Curve | null): Curve | null {
  return n.show_curve === 1 ? effectiveCurve(n, values, parent) : parent;
}

// The effective curve of the node `name`'s parent chain, for the dialog.
export function parentCurveOf(nodes: OptionNode[], values: Record<string, NodeValue>, name: string, parent: Curve | null = null): Curve | null | undefined {
  for (const n of nodes) {
    if (n.name === name) return parent;
    const hit = parentCurveOf(n.children, values, name, curveForChildren(n, values, parent));
    if (hit !== undefined) return hit;
  }
  return undefined;
}

export function effectiveInvert(n: OptionNode, values: Record<string, NodeValue>): boolean {
  return values[n.name]?.invert ?? n.default_invert ?? false;
}

// Descendants with an own curve / an own invert: what a group change replaces.
export function countBelow(n: OptionNode, values: Record<string, NodeValue>, what: "curve" | "invert"): number {
  let sum = 0;
  for (const c of n.children) {
    const v = values[c.name];
    if (what === "curve" ? ownCurve(v) : v?.invert !== null && v?.invert !== undefined) sum += 1;
    sum += countBelow(c, values, what);
  }
  return sum;
}

// --- working state -------------------------------------------------------------

// What the game's screen shows for a mouse setting the settings file lacks.
export const MOUSE_DEFAULTS = { ads_percent: 100, zoom_scaling_enabled: true, zoom_scaling_percent: 75 } as const;

export interface AxisValues {
  deadzone: number | null;
  saturation: number | null;
}

// One source's settings in a shape the edits can work on.
export interface ConfigState {
  // `kind:instance` -> node -> own values.
  values: Record<string, Record<string, NodeValue>>;
  // Raw deviceoptions name -> input -> values.
  devopts: Record<string, Record<string, AxisValues>>;
  mouse: MouseView;
  gamepadSensitivity: number | null;
}

export function blockKey(kind: DeviceKind, instance: number): string {
  return `${kind}:${instance}`;
}

function cloneValue(v: NodeValue): NodeValue {
  return { invert: v.invert ?? null, exponent: v.exponent ?? null, points: v.points ? v.points.map((p) => [p[0], p[1]] as Pt) : null };
}

export function stateOf(view: DeviceConfigView): ConfigState {
  const values: ConfigState["values"] = {};
  for (const b of view.options) {
    const m: Record<string, NodeValue> = {};
    for (const [k, v] of Object.entries(b.values)) m[k] = cloneValue(v);
    values[blockKey(b.kind, b.instance)] = m;
  }
  const devopts: ConfigState["devopts"] = {};
  for (const d of view.device_options) {
    const m: Record<string, AxisValues> = {};
    for (const a of d.axes) m[a.input] = { deadzone: a.deadzone ?? null, saturation: a.saturation ?? null };
    devopts[d.name] = m;
  }
  return { values, devopts, mouse: { ...view.mouse }, gamepadSensitivity: view.gamepad_sensitivity ?? null };
}

function isEmpty(v: NodeValue): boolean {
  return v.invert === null && v.exponent === null && v.points === null;
}

function wipeBelow(n: OptionNode, values: Record<string, NodeValue>, what: "curve" | "invert") {
  for (const c of n.children) {
    const v = values[c.name];
    if (v) {
      if (what === "curve") {
        v.exponent = null;
        v.points = null;
      } else v.invert = null;
      if (isEmpty(v)) delete values[c.name];
    }
    wipeBelow(c, values, what);
  }
}

// One curve / invert change on a block's values, the way the game does it:
// a group's curve (not Set Default) removes the curves below it, the mining
// group's invert the inverts below it; an element left empty goes.
function applyNodeChange(values: Record<string, NodeValue>, ch: ConfigChange, node: OptionNode | null) {
  if (ch.type !== "curve" && ch.type !== "invert") return;
  const v = values[ch.node] ?? { invert: null, exponent: null, points: null };
  if (ch.type === "curve") {
    v.exponent = ch.value.kind === "exponent" ? ch.value.value : null;
    v.points = ch.value.kind === "points" ? ch.value.points.map((p) => [p[0], p[1]] as Pt) : null;
    if (node && ch.value.kind !== "default") wipeBelow(node, values, "curve");
  } else {
    v.invert = ch.value;
    if (node && ch.value !== null) wipeBelow(node, values, "invert");
  }
  if (isEmpty(v)) delete values[ch.node];
  else values[ch.node] = v;
}

function treeOf(trees: OptionTree[], kind: DeviceKind): OptionTree | undefined {
  return trees.find((t) => t.kind === kind);
}

// Apply one change to a working state (in place).
export function applyChange(state: ConfigState, ch: ConfigChange, trees: OptionTree[]) {
  switch (ch.type) {
    case "curve":
    case "invert": {
      const key = blockKey(ch.kind, ch.instance);
      const values = (state.values[key] ??= {});
      const tree = treeOf(trees, ch.kind);
      applyNodeChange(values, ch, tree ? (findNode(tree.nodes, ch.node)?.node ?? null) : null);
      break;
    }
    case "deadzone":
    case "saturation": {
      const dev = (state.devopts[ch.device] ??= {});
      const axis = (dev[ch.input] ??= { deadzone: null, saturation: null });
      axis[ch.type] = ch.value;
      break;
    }
    case "mouse_acceleration":
      state.mouse.acceleration = ch.value;
      break;
    case "mouse_smoothing":
      state.mouse.smoothing = ch.value;
      break;
    case "attribute":
      if (ch.setting === "mouse_sensitivity") state.mouse.sensitivity = ch.value;
      else if (ch.setting === "ads_percent") state.mouse.ads_percent = ch.value;
      else if (ch.setting === "zoom_scaling_enabled") state.mouse.zoom_scaling_enabled = ch.value !== 0;
      else if (ch.setting === "zoom_scaling_percent") state.mouse.zoom_scaling_percent = ch.value;
      else state.gamepadSensitivity = ch.value;
      break;
  }
}

// The saved state with the user's edits on top.
export function workingState(saved: DeviceConfigView, edits: ConfigChange[], trees: OptionTree[]): ConfigState {
  const s = stateOf(saved);
  for (const ch of edits) applyChange(s, ch, trees);
  return s;
}

function sameNum(a: number | null, b: number | null, digits = 2): boolean {
  if (a === null || b === null) return a === b;
  const f = 10 ** digits;
  return Math.round(a * f) === Math.round(b * f);
}

// The changes that turn `saved` into `working`, in an order the backend can
// apply one by one: the tree in pre-order, so a group's change (wiping
// everything below it) comes before the children's own values. Edits that
// cancel out leave nothing; a block the file lacks is never written.
export function diffChanges(saved: DeviceConfigView, working: ConfigState, trees: OptionTree[]): ConfigChange[] {
  const out: ConfigChange[] = [];
  const base = stateOf(saved);
  for (const b of saved.options) {
    const tree = treeOf(trees, b.kind);
    if (!tree) continue;
    const key = blockKey(b.kind, b.instance);
    const sim = base.values[key] ?? {};
    const work = working.values[key] ?? {};
    for (const n of preorder(tree.nodes)) {
      if (n.show_curve === 1) {
        const w = ownCurve(work[n.name]);
        if (!sameCurve(w, ownCurve(sim[n.name]))) {
          const ch: ConfigChange = { type: "curve", kind: b.kind, instance: b.instance, node: n.name, value: curveValue(w) };
          out.push(ch);
          applyNodeChange(sim, ch, n);
        }
      }
      if (n.show_invert === 1) {
        const w = work[n.name]?.invert ?? null;
        if (w !== (sim[n.name]?.invert ?? null)) {
          const ch: ConfigChange = { type: "invert", kind: b.kind, instance: b.instance, node: n.name, value: w };
          out.push(ch);
          applyNodeChange(sim, ch, n);
        }
      }
    }
  }
  // A missing deadzone shows as 0.00, a missing saturation as 1.00: back at
  // the shown value is no change.
  for (const [device, inputs] of Object.entries(working.devopts)) {
    for (const [input, w] of Object.entries(inputs)) {
      const s = base.devopts[device]?.[input];
      if (w.deadzone !== null && !sameNum(w.deadzone, s?.deadzone ?? 0)) out.push({ type: "deadzone", device, input, value: w.deadzone });
      if (w.saturation !== null && !sameNum(w.saturation, s?.saturation ?? 1)) out.push({ type: "saturation", device, input, value: w.saturation });
    }
  }
  const wm = working.mouse;
  const sm = base.mouse;
  if (wm.acceleration !== null && !sameNum(wm.acceleration, sm.acceleration ?? 0)) out.push({ type: "mouse_acceleration", value: wm.acceleration });
  if (wm.smoothing !== null && !sameNum(wm.smoothing, sm.smoothing ?? 0)) out.push({ type: "mouse_smoothing", value: wm.smoothing });
  const attr = (setting: "mouse_sensitivity" | "ads_percent" | "zoom_scaling_percent" | "gamepad_sensitivity", w: number | null, s: number | null, digits: number) => {
    if (w !== null && !sameNum(w, s, digits)) out.push({ type: "attribute", setting, value: w });
  };
  // Not set shows the game screen's default (sensitivity has none known):
  // back at it is no change either.
  attr("mouse_sensitivity", wm.sensitivity, sm.sensitivity, 0);
  attr("ads_percent", wm.ads_percent, sm.ads_percent ?? MOUSE_DEFAULTS.ads_percent, 0);
  if (wm.zoom_scaling_enabled !== null && wm.zoom_scaling_enabled !== (sm.zoom_scaling_enabled ?? MOUSE_DEFAULTS.zoom_scaling_enabled)) {
    out.push({ type: "attribute", setting: "zoom_scaling_enabled", value: wm.zoom_scaling_enabled ? 1 : 0 });
  }
  attr("zoom_scaling_percent", wm.zoom_scaling_percent, sm.zoom_scaling_percent ?? MOUSE_DEFAULTS.zoom_scaling_percent, 0);
  attr("gamepad_sensitivity", working.gamepadSensitivity, base.gamepadSensitivity, 2);
  return out;
}

// Do two own values differ (the changed marker of a row)?
export function nodeChanged(a: NodeValue | undefined, b: NodeValue | undefined): boolean {
  return !sameCurve(ownCurve(a), ownCurve(b)) || (a?.invert ?? null) !== (b?.invert ?? null);
}

// --- devices --------------------------------------------------------------------

// The joystick inputs the game has deadzone / saturation for (not in the
// game data, fixed), and the gamepad's sticks.
export const JOYSTICK_INPUTS = ["x", "y", "z", "rotx", "roty", "rotz"] as const;
export const GAMEPAD_INPUTS = ["thumbl", "thumbr"] as const;
export const GAMEPAD_DEVICE = "Controller (Gamepad)";

// The `{GUID}` in a raw Product string, upper-case; null without one.
export function productGuid(raw: string | null | undefined): string | null {
  const m = /\{[0-9a-f-]+\}/i.exec(raw ?? "");
  return m ? m[0].toUpperCase() : null;
}

export function guidMatches(raw: string | null | undefined, guid: string | null): boolean {
  return !!guid && productGuid(raw) === productGuid(guid);
}

// A raw Product compared the way two files may spell it (spaces differ).
export function normProduct(raw: string | null | undefined): string {
  return (raw ?? "").trim().replace(/\s+/g, " ");
}

// --- compare --------------------------------------------------------------------

export type CompareCell = { curve: Curve } | { text: string } | null;
export type CompareStatus = "added" | "removed" | "changed";

export interface CompareRow {
  key: string;
  device: string;
  rank: number;
  label: string;
  path: string;
  type: string;
  a: CompareCell;
  b: CompareCell;
  status: CompareStatus;
}

// One current device and its counterpart in both sources: the keyboard and
// the pad by kind, a joystick by its Product.
export interface CompareDevice {
  slot: string;
  rank: number;
  kind: DeviceKind;
  current: OptionsBlock | null;
  source: OptionsBlock | null;
  currentDev: DeviceOptionsView | null;
  sourceDev: DeviceOptionsView | null;
}

function axisMap(d: DeviceOptionsView | null): Record<string, AxisValues> {
  const m: Record<string, AxisValues> = {};
  for (const a of d?.axes ?? []) m[a.input] = { deadzone: a.deadzone ?? null, saturation: a.saturation ?? null };
  return m;
}

const fmt = (v: number, dec = 2) => v.toFixed(dec);

// Current (A) against the source (B), seen from Current: the source adds a
// value Current lacks, removes one Current has, or changes it.
export function compareRows(
  devices: CompareDevice[],
  trees: OptionTree[],
  labels: ConfigLabels | null,
  current: DeviceConfigView,
  source: DeviceConfigView,
): CompareRow[] {
  const rows: CompareRow[] = [];
  const push = (d: CompareDevice, key: string, label: string, path: string, type: string, a: CompareCell, b: CompareCell) => {
    const status: CompareStatus = a === null ? "added" : b === null ? "removed" : "changed";
    rows.push({ key: `${d.slot}/${key}`, device: d.slot, rank: d.rank, label, path, type, a, b, status });
  };
  const num = (v: number | null, dec = 2): CompareCell => (v === null ? null : { text: fmt(v, dec) });
  for (const d of devices) {
    const tree = treeOf(trees, d.kind);
    const av = d.current?.values ?? {};
    const bv = d.source?.values ?? {};
    if (tree && (d.current || d.source)) {
      for (const n of preorder(tree.nodes)) {
        const a = av[n.name];
        const b = bv[n.name];
        if (!a && !b) continue;
        const path = findNode(tree.nodes, n.name)?.path.join(" › ") ?? "";
        if (n.show_curve === 1) {
          const ca = ownCurve(a);
          const cb = ownCurve(b);
          if (!sameCurve(ca, cb)) push(d, `curve/${n.name}`, nodeLabel(n), path, "Curve", ca && { curve: ca }, cb && { curve: cb });
        }
        if (n.show_invert === 1) {
          const ia = a?.invert ?? null;
          const ib = b?.invert ?? null;
          if (ia !== ib) push(d, `invert/${n.name}`, nodeLabel(n), path, "Invert", ia === null ? null : { text: ia ? "Yes" : "No" }, ib === null ? null : { text: ib ? "Yes" : "No" });
        }
      }
    }
    if (d.kind === "joystick" || d.kind === "gamepad") {
      const am = axisMap(d.currentDev);
      const bm = axisMap(d.sourceDev);
      const inputs = d.kind === "joystick" ? JOYSTICK_INPUTS : GAMEPAD_INPUTS;
      for (const input of inputs) {
        const dzLabel = (d.kind === "joystick" ? labels?.joystick_deadzone[input] : labels?.gamepad_deadzone[input]) || input;
        const dza = am[input]?.deadzone ?? null;
        const dzb = bm[input]?.deadzone ?? null;
        if (!sameNum(dza, dzb)) push(d, `dz/${input}`, dzLabel, "", "Deadzone", num(dza), num(dzb));
        if (d.kind === "joystick") {
          // A missing saturation is 1.00.
          const sa = am[input]?.saturation ?? 1;
          const sb = bm[input]?.saturation ?? 1;
          if (!sameNum(sa, sb)) push(d, `sat/${input}`, labels?.joystick_saturation[input] || input, "", "Saturation", num(sa), num(sb));
        }
      }
    }
    if (d.kind === "keyboard") {
      const pairs: [string, string, number | null, number | null, number][] = [
        ["acc", labels?.mouse_acceleration ?? "", current.mouse.acceleration, source.mouse.acceleration, 2],
        ["smooth", labels?.mouse_smoothing ?? "", current.mouse.smoothing, source.mouse.smoothing, 2],
      ];
      // The settings file only where both sides have one.
      if (current.has_attributes && source.has_attributes) {
        pairs.unshift(
          ["sens", labels?.mouse_sensitivity ?? "", current.mouse.sensitivity, source.mouse.sensitivity, 0],
          ["ads", labels?.mouse_ads_percent ?? "", current.mouse.ads_percent, source.mouse.ads_percent, 0],
          ["zoom", labels?.mouse_zoom_scaling_percent ?? "", current.mouse.zoom_scaling_percent, source.mouse.zoom_scaling_percent, 0],
        );
        const za = current.mouse.zoom_scaling_enabled;
        const zb = source.mouse.zoom_scaling_enabled;
        if (za !== zb) {
          push(d, "zoomon", labels?.mouse_zoom_scaling_enabled ?? "", "", "Mouse", za === null ? null : { text: za ? "Yes" : "No" }, zb === null ? null : { text: zb ? "Yes" : "No" });
        }
      }
      for (const [key, label, a, b, dec] of pairs) if (!sameNum(a, b, dec)) push(d, key, label, "", "Mouse", num(a, dec), num(b, dec));
    }
    if (d.kind === "gamepad" && current.has_attributes && source.has_attributes) {
      const a = current.gamepad_sensitivity;
      const b = source.gamepad_sensitivity;
      if (!sameNum(a, b)) push(d, "sens", labels?.gamepad_sensitivity ?? "", "", "Sensitivity", num(a), num(b));
    }
  }
  return rows;
}
