// Device helpers shared by the views.

import type { DeviceInfo, DeviceKind, JoyInput } from "./types";

// Past this deflection an axis counts as pressed where inputs light a row
// (the rebind dialog's list): near full travel, the same point past which
// the backend derives a pad's trigger and stick direction buttons
// (`DERIVED_BUTTON_THRESHOLD` in input.rs).
export const AXIS_PRESS = 30000;

// Recording (rebind dialog, image-map editor): the release decides. Every
// press replaces the candidate, and the recording takes the candidate once
// that same input is let go of — so a dual-stage trigger pulled through
// records stage 2 (stage 1 fires first and stays held, the game itself can
// only ever bind stage 1), a combo records with the modifiers held at the
// press. An axis becomes the candidate past half travel and is taken when
// it is back at the centre (the backend reports the resting zone as 0);
// a hat when it is centred again.
export const AXIS_RECORD = 16384;

export type RecordEdge = "press" | "release" | null;

export function recordEdge(ev: JoyInput): RecordEdge {
  switch (ev.kind) {
    case "button":
    case "padbutton":
    case "key":
      return ev.pressed ? "press" : "release";
    case "hat":
      return ev.direction === "centered" ? "release" : "press";
    case "axis":
    case "padaxis":
      return Math.abs(ev.value) >= AXIS_RECORD ? "press" : ev.value === 0 ? "release" : null;
  }
}

// What tells one physical input from another across its press and release
// events (the token or key may differ between the two: modifiers, hat
// direction).
export function inputIdentity(ev: JoyInput): string {
  switch (ev.kind) {
    case "button":
    case "axis":
    case "hat":
      return `${ev.guid}#${ev.kind}#${ev.index}`;
    case "padbutton":
    case "padaxis":
    case "key":
      return `${ev.guid}#${ev.kind}#${ev.name}`;
  }
}

// The name a device is shown under: pads carry SDL's controller name, the
// rest SC's HID product string with SDL's name as the fallback.
export function deviceName(d: Pick<DeviceInfo, "kind" | "controller_name" | "sc_name" | "sdl_name">): string {
  if (d.kind === "gamepad") return d.controller_name ?? d.sc_name ?? d.sdl_name;
  return d.sc_name ?? d.sdl_name;
}

// The icon for a device's kind (Icon names); the Devices icon is a joystick.
export function deviceIcon(d: Pick<DeviceInfo, "kind">): "keyboard" | "gamepad" | "devices" {
  return kindIcon(d.kind);
}

export function kindIcon(kind: DeviceKind): "keyboard" | "gamepad" | "devices" {
  if (kind === "keyboard") return "keyboard";
  return kind === "gamepad" ? "gamepad" : "devices";
}

// Device-kind order of the filter chips and the DEVICE column, matching the
// tile order: keyboard, gamepad, then the joysticks.
export const KIND_RANK: Record<DeviceKind, number> = { keyboard: 0, gamepad: 1, joystick: 2 };

// A stable per-device key for remembered GUI state (SDL's index changes on
// every replug): the image-map hardware id, else the SDL GUID.
export function deviceKey(d: DeviceInfo): string {
  return (d.hardware_id ?? d.sdl_guid).toLowerCase();
}
