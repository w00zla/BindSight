// The backend's raw axis stream (`axis-raw`, see input.rs): off unless a
// view needs it. Every view that shows raw axes holds it while it does; the
// backend flag follows "anyone holds it", so a view leaving and another one
// arriving in the same tick never turn it off under the newcomer.

import { invoke } from "@tauri-apps/api/core";
import type { DeviceInfo } from "./types";

let holders = 0;

async function setStream(enabled: boolean) {
  try {
    await invoke("set_axis_stream", { enabled });
  } catch (e) {
    console.error(`axis stream ${enabled ? "on" : "off"} failed`, e);
  }
}

// Turn the stream on for the caller; the returned function lets go (once).
export function holdAxisStream(): () => void {
  holders++;
  if (holders === 1) void setStream(true);
  let held = true;
  return () => {
    if (!held) return;
    held = false;
    holders--;
    if (holders === 0) void setStream(false);
  };
}

// The six values of a pad's `axis-raw` payload, in order, by SC's names.
export const PAD_RAW_AXES = ["thumblx", "thumbly", "thumbrx", "thumbry", "triggerl", "triggerr"] as const;

// The fixed resting zones of the Monitor's input stream (input.rs
// `JOYSTICK_DEADZONE`, `PAD_THUMB_DEADZONE`, `PAD_TRIGGER_DEADZONE`), used
// where the game's settings configure none.
export const JOYSTICK_ZONE = 4000 / 32767;
export const PAD_THUMB_ZONE = 8000 / 32767;
export const PAD_TRIGGER_ZONE = 4000 / 32767;

// What the Monitor makes of a raw value (input.rs): 0 inside the resting
// zone, full travel at or beyond saturation, the raw value in between — no
// rescale. `zone` / `saturation` are the stored values (fractions of full
// travel); a null saturation is none.
export function monitorOutput(raw: number, zone: number, saturation: number | null): number {
  const m = Math.abs(raw);
  if (m < zone) return 0;
  if (saturation !== null && m >= saturation) return Math.sign(raw);
  return raw;
}

// Display -> stored, as the game writes the deadzone / saturation (see
// devconfig.rs): the Monitor applies the stored value.
export const JOYSTICK_STORED = 0.99;
export const PAD_STORED = 0.899;

// The axes the Monitor treats differently: a joystick axis, a pad stick, a
// pad trigger (fixed zone, never configured).
export type AxisClass = "joystick" | "thumb" | "trigger";

// The zone and saturation the Monitor applies to an axis (stored values),
// from the configured deadzone / saturation in display units (null = not
// configured: the fixed zone, no saturation). `fixed`: the zone is the
// fixed one.
export function axisLimits(cls: AxisClass, deadzone: number | null, saturation: number | null): { zone: number; saturation: number | null; fixed: boolean } {
  if (cls === "trigger") return { zone: PAD_TRIGGER_ZONE, saturation: null, fixed: true };
  const stored = cls === "joystick" ? JOYSTICK_STORED : PAD_STORED;
  const fixedZone = cls === "joystick" ? JOYSTICK_ZONE : PAD_THUMB_ZONE;
  return {
    zone: deadzone !== null ? deadzone * stored : fixedZone,
    saturation: cls === "joystick" && saturation !== null ? saturation * JOYSTICK_STORED : null,
    fixed: deadzone === null,
  };
}

// The one output rule every view shows (axis cards, the curve dialog's live
// dot, the Axis Test): `monitorOutput` with `axisLimits`.
export function axisOutput(cls: AxisClass, raw: number, deadzone: number | null, saturation: number | null): number {
  const l = axisLimits(cls, deadzone, saturation);
  return monitorOutput(raw, l.zone, l.saturation);
}

// The device an `axis-raw` payload (or an input event) came from: SDL GUID
// plus instance id — two sticks of the same type share the GUID.
export function deviceOfRaw(devices: DeviceInfo[], guid: string, instanceId: number): DeviceInfo | undefined {
  return devices.find((d) => d.sdl_guid === guid && d.sdl_instance_id === instanceId);
}
