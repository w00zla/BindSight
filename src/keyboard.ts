// Keyboard and mouse capture: the webview is the only source of key and mouse
// events (the backend never sees them). A captured key or mouse input becomes
// a `key` JoyInput with SC's own name, so it travels the same path as a
// joystick or gamepad input. SC treats keyboard and mouse as one device
// (`kb1_w`, `kb1_mouse1`), and so does the app.
//
// Keys are captured all the time; the mouse only while `recording` is on —
// the mouse drives the app the rest of the time. Recording is armed by a
// Record button (rebind dialog, image-map editor) and disarmed by whoever
// armed it once an input arrived, or by Escape.

import { ref } from "vue";
import type { JoyInput } from "./types";

// The keyboard's synthetic SDL GUID (the backend uses the same string).
export const KEYBOARD_GUID = "keyboard";

// `KeyboardEvent.code` -> SC key name (DirectInput scancode names: physical,
// layout independent). Codes without an SC name (Backquote, the Meta keys,
// ContextMenu) stay unmapped, as do SC's `underline` and `colon`, which no
// browser code produces.
function buildCodes(): Record<string, string> {
  const codes: Record<string, string> = {};
  for (const c of "abcdefghijklmnopqrstuvwxyz") codes[`Key${c.toUpperCase()}`] = c;
  for (let i = 0; i <= 9; i++) {
    codes[`Digit${i}`] = String(i);
    codes[`Numpad${i}`] = `np_${i}`;
  }
  for (let i = 1; i <= 12; i++) codes[`F${i}`] = `f${i}`;
  return Object.assign(codes, {
    NumLock: "numlock",
    NumpadDivide: "np_divide",
    NumpadMultiply: "np_multiply",
    NumpadSubtract: "np_subtract",
    NumpadAdd: "np_add",
    NumpadDecimal: "np_period",
    NumpadEnter: "np_enter",
    AltLeft: "lalt",
    AltRight: "ralt",
    ShiftLeft: "lshift",
    ShiftRight: "rshift",
    ControlLeft: "lctrl",
    ControlRight: "rctrl",
    Insert: "insert",
    Home: "home",
    Delete: "delete",
    End: "end",
    PageUp: "pgup",
    PageDown: "pgdn",
    PrintScreen: "print",
    ScrollLock: "scrolllock",
    Pause: "pause",
    ArrowUp: "up",
    ArrowDown: "down",
    ArrowLeft: "left",
    ArrowRight: "right",
    Escape: "escape",
    Minus: "minus",
    Equal: "equals",
    Semicolon: "semicolon",
    Quote: "apostrophe",
    Backspace: "backspace",
    Tab: "tab",
    BracketLeft: "lbracket",
    BracketRight: "rbracket",
    Enter: "enter",
    CapsLock: "capslock",
    Backslash: "backslash",
    Comma: "comma",
    Period: "period",
    Slash: "slash",
    Space: "space",
    IntlBackslash: "oem_102",
  });
}

export const KEY_CODES: Record<string, string> = buildCodes();

// Keys the capture knows: the keyboard's input count in the GUI.
export const KEY_COUNT = Object.keys(KEY_CODES).length;

// `MouseEvent.button` -> SC mouse button name: SC counts left, right, middle
// (its `mouse3` carries the wheel icon), then the thumb buttons.
const MOUSE_BUTTONS: Record<number, string> = { 0: "mouse1", 2: "mouse2", 1: "mouse3", 3: "mouse4", 4: "mouse5" };

// Mouse inputs the capture knows (SC's `maxis_*` are not captured).
export const MOUSE_INPUTS: string[] = [...Object.values(MOUSE_BUTTONS), "mwheel_up", "mwheel_down"];

// Record mode: the next input of any device is meant as a binding / an area's
// input, and the mouse is captured too. Shared by the Record buttons and
// the capture.
export const recording = ref(false);

// Typing into a field is text, not input capture.
function isTextTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
}

// Any open dialog (Settings, Confirm) owns the keyboard — unless it asks
// for the capture itself (the rebind dialog, `data-capture-keys`).
function dialogOpen(): boolean {
  return document.querySelector('[role="dialog"]:not([data-capture-keys])') !== null;
}

function keyEvent(name: string, pressed: boolean): JoyInput {
  return {
    kind: "key",
    guid: KEYBOARD_GUID,
    name,
    pressed,
    timestamp: performance.now() | 0,
    instance_id: 0,
  };
}

// Capture keys for as long as `isActive()` says so; returns the stop function.
// A key that went down is always released, even if capture turned off or the
// window lost focus in between, so nothing stays stuck.
export function startKeyboardCapture(handler: (ev: JoyInput) => void, isActive: () => boolean): () => void {
  const held = new Set<string>();

  function release(name: string) {
    held.delete(name);
    handler(keyEvent(name, false));
  }

  function onKeyDown(e: KeyboardEvent) {
    // Auto-repeat is not a new press.
    if (e.repeat) return;
    const name = KEY_CODES[e.code];
    if (!name || !isActive() || isTextTarget(e.target) || dialogOpen()) return;
    e.preventDefault();
    if (held.has(name)) return;
    held.add(name);
    handler(keyEvent(name, true));
  }

  function onKeyUp(e: KeyboardEvent) {
    const name = KEY_CODES[e.code];
    if (!name || !held.has(name)) return;
    e.preventDefault();
    release(name);
  }

  function onBlur() {
    for (const name of [...held]) release(name);
  }

  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", onBlur);

  return () => {
    window.removeEventListener("keydown", onKeyDown);
    window.removeEventListener("keyup", onKeyUp);
    window.removeEventListener("blur", onBlur);
    held.clear();
  };
}

// Capture mouse buttons and the wheel while `recording` is on; returns the
// stop function. Every pointer event is taken in the capture phase and
// stopped, so nothing underneath reacts (no click, no context menu, no
// backdrop dismiss) — while recording, a left click IS `mouse1`. A button
// that went down is always released, and the click that follows a captured
// press is swallowed even though recording has ended by then. The wheel is
// momentary: one notch is a press and a release.
export function startMouseCapture(handler: (ev: JoyInput) => void): () => void {
  const held = new Set<number>();
  let swallowClick = false;

  function swallow(e: Event) {
    e.preventDefault();
    e.stopImmediatePropagation();
  }

  function onDown(e: MouseEvent) {
    swallowClick = false;
    const name = MOUSE_BUTTONS[e.button];
    if (!name || !recording.value) return;
    swallow(e);
    held.add(e.button);
    handler(keyEvent(name, true));
  }

  function onUp(e: MouseEvent) {
    if (!held.has(e.button)) return;
    swallow(e);
    held.delete(e.button);
    swallowClick = true;
    handler(keyEvent(MOUSE_BUTTONS[e.button], false));
  }

  function onClick(e: MouseEvent) {
    if (recording.value || held.size || swallowClick) swallow(e);
    swallowClick = false;
  }

  function onWheel(e: WheelEvent) {
    if (!recording.value || e.deltaY === 0) return;
    swallow(e);
    const name = e.deltaY < 0 ? "mwheel_up" : "mwheel_down";
    handler(keyEvent(name, true));
    handler(keyEvent(name, false));
  }

  function onBlur() {
    for (const b of [...held]) {
      held.delete(b);
      handler(keyEvent(MOUSE_BUTTONS[b], false));
    }
  }

  const opts: AddEventListenerOptions = { capture: true, passive: false };
  window.addEventListener("mousedown", onDown, opts);
  window.addEventListener("mouseup", onUp, opts);
  window.addEventListener("click", onClick, opts);
  window.addEventListener("auxclick", onClick, opts);
  window.addEventListener("dblclick", onClick, opts);
  window.addEventListener("contextmenu", onClick, opts);
  window.addEventListener("wheel", onWheel, opts);
  window.addEventListener("blur", onBlur);

  return () => {
    window.removeEventListener("mousedown", onDown, opts);
    window.removeEventListener("mouseup", onUp, opts);
    window.removeEventListener("click", onClick, opts);
    window.removeEventListener("auxclick", onClick, opts);
    window.removeEventListener("dblclick", onClick, opts);
    window.removeEventListener("contextmenu", onClick, opts);
    window.removeEventListener("wheel", onWheel, opts);
    window.removeEventListener("blur", onBlur);
    held.clear();
  };
}
