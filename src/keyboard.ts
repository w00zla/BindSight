// Keyboard capture: the webview is the only source of key events (the backend
// never sees them). A captured key becomes a `key` JoyInput with SC's own key
// name, so it travels the same path as a joystick or gamepad input.

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

// Typing into a field is text, not input capture.
function isTextTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
}

// Any open dialog (Settings, Confirm) owns the keyboard.
function dialogOpen(): boolean {
  return document.querySelector('[role="dialog"]') !== null;
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
