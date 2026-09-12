// Shared frontend types for data coming from the Rust backend.

import type { ImageMap } from "./imagemap";

// What a device is: SC binds one keyboard (kb1) and one gamepad (gp1) plus
// any number of joysticks (jsN).
export type DeviceKind = "joystick" | "gamepad" | "keyboard";

export interface DeviceInfo {
  index: number;
  kind: DeviceKind;
  sc_name: string | null;
  sdl_name: string;
  sdl_guid: string;
  sc_product_guid: string | null;
  // Key for image-maps and the image-map choice: a joystick's SC Product GUID,
  // "gamepad" for the pad holding the gp1 slot, "keyboard" for the keyboard,
  // null for anything that cannot hold bindings (a further pad).
  hardware_id: string | null;
  // 1 for the first game controller (= gp1, first come first serve like SC),
  // null for every further pad and for non-pads.
  gamepad_slot: number | null;
  // SDL_GameControllerName, for pads only.
  controller_name: string | null;
  num_buttons: number;
  num_axes: number;
  num_hats: number;
  // SC axis name per SDL axis index (from the HID descriptor); empty with
  // axes_error set when it could not be derived.
  axes: string[];
  axes_error: string | null;
  // Troubleshooting detail for the Device Info view (input.rs `DeviceInfo`).
  sdl_instance_id: number;
  sdl_vendor: number;
  sdl_product: number;
  sdl_product_version: number;
  sdl_type: string;
  sdl_path: string | null;
  power_level: string;
  num_balls: number;
  has_rumble: boolean;
  has_led: boolean;
  hid_interfaces: HidInterface[];
  // The joystick interface's report descriptor as hex, and its input fields
  // in report order.
  hid_descriptor: string | null;
  hid_usages: string[];
}

// One hidapi interface behind a device's vendor/product.
export interface HidInterface {
  path: string;
  interface_number: number;
  usage_page: number;
  usage: number;
  manufacturer: string | null;
  product: string | null;
  serial: string | null;
  release: number;
  bus_type: string;
}

// `timestamp` = SDL's event time in ms since SDL init, `instance_id` = the
// SDL joystick instance. Gamepad inputs carry SC's name instead of an index;
// `key` events are made in the webview (guid "keyboard", instance 0).
interface JoyInputBase {
  guid: string;
  timestamp: number;
  instance_id: number;
}
export type JoyInput =
  | (JoyInputBase & { kind: "button"; index: number; pressed: boolean })
  | (JoyInputBase & { kind: "axis"; index: number; value: number })
  | (JoyInputBase & { kind: "hat"; index: number; direction: string; raw: number })
  | (JoyInputBase & { kind: "padbutton"; name: string; pressed: boolean })
  | (JoyInputBase & { kind: "padaxis"; name: string; value: number })
  | (JoyInputBase & { kind: "key"; name: string; pressed: boolean });

// A JoyInput as kept in the Device Info view, with the wall-clock time it
// arrived and the SC token it stood for then (null when SC cannot bind it).
// `id` is a running number, the stable key of a log line.
export type LoggedInput = JoyInput & { id: number; at: number; token: string | null };

// Top-level GUI mode.
export type Mode = "monitor" | "bindings" | "devices";

// SC channels, in GUI order; the config always holds all of them.
export const ENVIRONMENTS = ["LIVE", "HOTFIX", "PTU", "EPTU"] as const;

// One SC channel install (config.rs `Environment`).
export interface Environment {
  // The folder that holds Data.p4k.
  path: string;
  // Take the action labels from `global_ini` instead of the install's own.
  global_ini_override: boolean;
  global_ini: string;
}

export interface Config {
  environments: Record<string, Environment>;
  active_env: string;
  // Wire name kept as `ignored_devices` (Rust serde rename on the renamed
  // `excluded_devices` field) so existing config.json files still apply.
  ignored_devices: string[];
  imagemap_choices: Record<string, string>;
  // Back up actionmaps.xml before BindSight overwrites it.
  auto_backup: boolean;
  // Write DEBUG records to bindsight.log (else INFO and up).
  debug_logging: boolean;
}

export interface Action {
  name: string;
  label: string | null;
  description: string | null;
  joystick_default: string | null;
  keyboard_default: string | null;
  gamepad_default: string | null;
  // The mouse is part of the keyboard device (SC binds it as kb1_mouse1).
  mouse_default: string | null;
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
  device_kind: DeviceKind;
  // jsN for joysticks; always 1 for the keyboard and the gamepad.
  instance: number;
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
  device_kind: DeviceKind;
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
  | { kind: "no_device_lines"; path: string }
  // The game listed a gamepad but no joystick at its last start.
  | { kind: "no_joystick_lines" };

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
  // SC listed a gamepad at its last start (`Connected xinput0:`).
  gamepad_seen: boolean;
  resort: ResortMove[];
  // In-game equivalent of `resort`: pp_resortdevices swaps, in order.
  resort_commands: string[];
}

// Last Input card colour: "unseen" (SC does not see the device), "noorder"
// (a joystick while the game's device order is unknown), "bound", "none".
export type LiveState = "unseen" | "noorder" | "bound" | "none";

// `sdl` is the SDL-side input name, shown when SC has no token for the input;
// `sc_guid` lets the tile tell whether SC sees the device at all.
export interface CurrentInput {
  device: string;
  kind: DeviceKind;
  sc_guid: string | null;
  token: string | null;
  sdl: string;
  actions: BoundAction[];
  // Whether the device's image-map has an area for this input; `null` when
  // the device has no image-map at all.
  in_imagemap: boolean | null;
}

// --- Bindings mode: binding profiles, backups, compare ----------------------

// One exported SC keybinding layout in the binding profiles folder.
export interface BindingProfileSummary {
  file: string;
  name: string;
  bindings: number;
  // File mtime, unix seconds.
  modified: number;
}

// One saved copy of actionmaps.xml under the app data dir.
export interface BackupSummary {
  id: string;
  // Unix seconds.
  created: number;
  reason: string;
  // SC version label at backup time; null for older backups.
  game_version: string | null;
  bindings: number;
}

// One joystick named in the profile's <options> block.
export interface JoystickDevice {
  instance: number;
  product_name: string;
  product_guid: string | null;
}

// Facts about the loaded actionmaps.xml (lib.rs `CurrentBindingsInfo`).
export interface CurrentBindingsInfo {
  path: string;
  // Unix seconds; 0 if unknown.
  modified: number;
  size: number;
  rebinds: number;
  joysticks: JoystickDevice[];
}

// One rebind to write: the full SC input as SC stores it (`js2_button5`,
// `kb1_lalt+x`, `gp1_a`); it replaces the action's binding of that kind.
export interface RebindChange {
  actionmap: string;
  action: string;
  kind: DeviceKind;
  input: string;
}

// One side of a comparison.
export type DiffSource =
  | { kind: "current" }
  | { kind: "profile"; file: string }
  | { kind: "backup"; id: string };

// Bound in A only / in B only / on both sides with different actions / on
// both sides to the same actions.
export type DiffKind = "added" | "removed" | "changed" | "same";

// One action a token is bound to, for a diff row.
export interface ActionRef {
  actionmap: string;
  action: string;
  label: string | null;
}

// One SC token whose bound actions differ between A and B.
export interface DiffRow {
  token: string;
  device_kind: DeviceKind;
  // The N in jsN_..., 1 for the keyboard and the gamepad, null for a token
  // with no recognisable device prefix.
  instance: number | null;
  kind: DiffKind;
  a: ActionRef[];
  b: ActionRef[];
}

// Result of comparing two binding sets; rows come sorted.
export interface DiffReport {
  rows: DiffRow[];
  added: number;
  removed: number;
  changed: number;
  same: number;
}

// One device to take over when applying a profile or backup (apply.rs); a
// joystick's bindings can land on another slot (`target`).
export interface DeviceSel {
  kind: DeviceKind;
  instance: number;
  target?: number;
}

// One device with a chosen image-map, shown on the image stage.
export interface ImageMapView {
  device: DeviceInfo;
  map: ImageMap;
}
