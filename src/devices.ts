// Device helpers shared by the views.

import type { DeviceInfo } from "./types";

// The name a device is shown under: pads carry SDL's controller name, the
// rest SC's HID product string with SDL's name as the fallback.
export function deviceName(d: DeviceInfo): string {
  if (d.kind === "gamepad") return d.controller_name ?? d.sc_name ?? d.sdl_name;
  return d.sc_name ?? d.sdl_name;
}

// A stable per-device key for remembered GUI state (SDL's index changes on
// every replug): the image-map hardware id, else the SDL GUID.
export function deviceKey(d: DeviceInfo): string {
  return (d.hardware_id ?? d.sdl_guid).toLowerCase();
}
