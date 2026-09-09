// Device helpers shared by the views.

import type { DeviceInfo, DeviceKind } from "./types";

// The name a device is shown under: pads carry SDL's controller name, the
// rest SC's HID product string with SDL's name as the fallback.
export function deviceName(d: DeviceInfo): string {
  if (d.kind === "gamepad") return d.controller_name ?? d.sc_name ?? d.sdl_name;
  return d.sc_name ?? d.sdl_name;
}

// Device-kind order of the filter chips and the DEVICE column, matching the
// tile order: keyboard, gamepad, then the joysticks.
export const KIND_RANK: Record<DeviceKind, number> = { keyboard: 0, gamepad: 1, joystick: 2 };

// A stable per-device key for remembered GUI state (SDL's index changes on
// every replug): the image-map hardware id, else the SDL GUID.
export function deviceKey(d: DeviceInfo): string {
  return (d.hardware_id ?? d.sdl_guid).toLowerCase();
}
