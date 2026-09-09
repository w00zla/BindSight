// Shared frontend types for data coming from the Rust backend.

import type { ImageMap, ImageMapSummary } from "./imagemap";

export interface DeviceInfo {
  index: number;
  sc_name: string | null;
  sdl_name: string;
  sdl_guid: string;
  sc_product_guid: string | null;
  num_buttons: number;
  num_axes: number;
  num_hats: number;
  // SC axis name per SDL axis index (from the HID descriptor); empty with
  // axes_error set when it could not be derived.
  axes: string[];
  axes_error: string | null;
}

export type JoyInput =
  | { kind: "button"; guid: string; index: number; pressed: boolean }
  | { kind: "axis"; guid: string; index: number; value: number }
  | { kind: "hat"; guid: string; index: number; direction: string };

// Top-level GUI mode.
export type Mode = "live" | "tools" | "devices";

export interface Action {
  name: string;
  label: string | null;
  description: string | null;
  joystick_default: string | null;
}

export interface ActionMap {
  name: string;
  label: string | null;
  actions: Action[];
}

export interface ResolvedBinding {
  token: string;
  device: string | null;
  device_guid: string | null;
  actionmap: string;
  action: string;
  label: string | null;
  // A shipped default from defaultProfile.xml (on js1), not a user rebind.
  is_default: boolean;
}

// Version of the configured SC install (from build_manifest.id).
export interface ScVersion {
  // Display label / cache key, e.g. "4.10.0-hotfix.12572603".
  label: string;
  branch: string;
  version: string;
  changelist: string;
  build_id: string;
}

// The install's version and how loading its game data went.
export interface ScStatus {
  version: ScVersion | null;
  loading: boolean;
  // Completed load steps out of `steps` while loading.
  progress: number;
  steps: number;
  error: string | null;
}

export interface LoadStatus {
  base_path: string;
  actionmaps_path: string;
  loaded: boolean;
  error: string | null;
  bindings: ResolvedBinding[];
  sc: ScStatus;
}

export interface BoundAction {
  actionmap: string;
  action: string;
  label: string | null;
  is_default: boolean;
}

// One joystick in SC's order: the jsN SC assigns it vs the jsN its bindings
// were saved under. connected_now: SDL sees it right now (a Game.log slot can
// be unplugged since SC started).
export interface SlotStatus {
  effective_instance: number;
  stored_instance: number | null;
  sc_product_guid: string | null;
  name: string | null;
  clash: boolean;
  connected_now: boolean;
}

// A saved slot whose device is not in SC's list — it dangles and shifts the rest.
export interface MissingSlot {
  stored_instance: number;
  name: string;
  sc_product_guid: string | null;
}

// An SDL-visible device SC did not list at its last start: hidden by Wine, or
// plugged in after SC started.
export interface UnseenDevice {
  name: string | null;
  sc_product_guid: string | null;
}

// Why Game.log could not be used, so the GUI can say exactly what is wrong.
export type GameLogError =
  | { kind: "not_found"; path: string; reason: string }
  | { kind: "no_device_lines"; path: string };

// One resort step: the bindings saved under js{from} belong on js{to}.
export interface ResortMove {
  from: number;
  to: number;
  name: string | null;
}

// Game.log is the only order source: with log_error set, everything else is
// empty and nothing is said about the order.
export interface ClashReport {
  connected: SlotStatus[];
  missing: MissingSlot[];
  unseen: UnseenDevice[];
  log_timestamp: string | null;
  log_error: GameLogError | null;
  has_clash: boolean;
  resort: ResortMove[];
  // In-game equivalent of `resort`: pp_resortdevices swaps, in order.
  resort_commands: string[];
}

// `sdl` is the SDL-side input name, shown when SC has no token for the input;
// `sc_guid` lets the tile tell whether SC sees the device at all.
export interface CurrentInput {
  device: string;
  sc_guid: string | null;
  token: string | null;
  sdl: string;
  actions: BoundAction[];
  // Whether the device's image-map has an area for this input; `null` when
  // the device has no image-map at all.
  in_imagemap: boolean | null;
}

// One device with a chosen image-map, shown on the image stage.
export interface ImageMapView {
  device: DeviceInfo;
  map: ImageMap;
  options: ImageMapSummary[];
}
