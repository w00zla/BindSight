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
      return `${eventDeviceKey(ev)}#${ev.kind}#${ev.index}`;
    case "padbutton":
    case "padaxis":
    case "key":
      return `${eventDeviceKey(ev)}#${ev.kind}#${ev.name}`;
  }
}

// The name a device is shown under: pads carry SDL's controller name, the
// rest SC's HID product string with SDL's name as the fallback.
export function deviceName(d: Pick<DeviceInfo, "kind" | "controller_name" | "sc_name" | "sdl_name">, instance?: number | null): string {
  if (d.kind === "gamepad") return d.controller_name ?? d.sc_name ?? d.sdl_name;
  return (d.sc_name ?? d.sdl_name) + (d.kind === "joystick" && instance ? ` js${instance}` : "");
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

// Unit identity for GUI state; model IDs remain image-map compatibility keys.
const session = crypto.randomUUID();
export function deviceKey(d: DeviceInfo): string {
  if (d.kind === "keyboard") return "keyboard";
  const path = d.sdl_path?.trim();
  return path
    ? JSON.stringify([d.sdl_guid.toLowerCase(), "path", path.startsWith("\\\\") ? path.toLowerCase() : path])
    : `${session}:${liveDeviceKey(d)}`;
}

export function liveDeviceKey(d: DeviceInfo): string {
  return `${d.sdl_guid}#${d.sdl_instance_id}`;
}

export function mapChoice(d: DeviceInfo, choices: Record<string, string>): string | undefined {
  return choices[deviceKey(d)] ?? (d.hardware_id ? choices[d.hardware_id.toLowerCase()] : undefined);
}

export function eventDeviceKey(ev: JoyInput): string {
  return `${ev.guid}#${ev.instance_id}`;
}

export function slotChoiceKey(d: DeviceInfo, devices: DeviceInfo[], slots: { sc_product_guid: string | null; effective_instance: number }[]): string {
  const rememberedKey = (p: DeviceInfo): string => {
    const serial = p.sdl_serial?.trim();
    const uniqueSerial = serial && devices.filter((other) =>
      other.sdl_guid === p.sdl_guid && other.sdl_serial?.trim() === serial).length === 1;
    return !p.sdl_path?.trim() && uniqueSerial
      ? JSON.stringify([p.sdl_guid.toLowerCase(), "serial", serial]) : deviceKey(p);
  };
  const key = rememberedKey(d);
  const unit = devices.filter((p) => rememberedKey(p) === key).length > 1 ? session + liveDeviceKey(d) : key;
  const context = slots.map((s) => [s.effective_instance, s.sc_product_guid?.toLowerCase()]).sort((a, b) => Number(a[0]) - Number(b[0]));
  const peers = devices.filter((p) => p.kind === "joystick" && p.sc_product_guid?.toLowerCase() === d.sc_product_guid?.toLowerCase()).map(rememberedKey).sort();
  return JSON.stringify([unit, context, peers]);
}

export function assignJoystickSlot<T extends { sc_product_guid: string | null; effective_instance: number }>(
  device: DeviceInfo, devices: DeviceInfo[], slots: T[], previous: Record<string, number>, slot: number,
): Record<string, number> {
  const choices = { ...previous, [`model:${device.sc_product_guid?.toLowerCase()}`]: -1 };
  for (const peer of devices) {
    const key = slotChoiceKey(peer, devices, slots);
    if (choices[key] === slot) delete choices[key];
  }
  const key = slotChoiceKey(device, devices, slots);
  delete choices[key];
  if (slots.some((s) => s.effective_instance === slot && s.sc_product_guid?.toLowerCase() === device.sc_product_guid?.toLowerCase())) choices[key] = slot;
  return choices;
}

// Product GUIDs identify models. Only a one-to-one match is automatic.
// Explicit choices are accepted only while both the product and slot match,
// and no other connected device claims that slot.
export function joystickSlot<T extends { sc_product_guid: string | null; effective_instance: number }>(
  device: DeviceInfo, devices: DeviceInfo[], slots: T[], choices: Record<string, number>,
): T | null {
  const same = (a: string | null, b: string | null) => !!a && !!b && a.toLowerCase() === b.toLowerCase();
  const peers = devices.filter((d) => d.kind === "joystick" && same(d.sc_product_guid, device.sc_product_guid));
  const candidates = slots.filter((s) => same(s.sc_product_guid, device.sc_product_guid));
  if (!choices[`model:${device.sc_product_guid?.toLowerCase()}`] && peers.length === 1 && candidates.length === 1) return candidates[0];
  const choice = choices[slotChoiceKey(device, devices, slots)];
  if (!choice || peers.filter((d) => choices[slotChoiceKey(d, devices, slots)] === choice).length !== 1) return null;
  return candidates.find((s) => s.effective_instance === choice) ?? null;
}
