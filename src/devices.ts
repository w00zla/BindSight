// Device helpers shared by the views.

import type { DeviceInfo, DeviceKind } from "./types";

// Past this deflection an axis counts as pressed wherever an input is
// recorded (rebind dialog, image-map editor): near full travel, the same
// point past which the backend derives a pad's trigger and stick direction
// buttons (`DERIVED_BUTTON_THRESHOLD` in input.rs).
export const AXIS_PRESS = 30000;

// The name a device is shown under: pads carry SDL's controller name, the
// rest SC's HID product string with SDL's name as the fallback.
export function deviceName(d: DeviceInfo): string {
  if (d.kind === "gamepad") return d.controller_name ?? d.sc_name ?? d.sdl_name;
  return d.sc_name ?? d.sdl_name;
}

// The icon for a device's kind (Icon names); the Devices icon is a joystick.
export function deviceIcon(d: DeviceInfo): "keyboard" | "gamepad" | "devices" {
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
