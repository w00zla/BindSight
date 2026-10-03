// Shared frontend types for data coming from the Rust backend.

import type { ImageMap } from "./imagemap";

// What a device is: SC binds one keyboard (kb1) and one gamepad (gp1) plus
// any number of joysticks (jsN).
export type DeviceKind = "joystick" | "gamepad" | "keyboard";

// Payload of `devices-changed`: what a re-enumeration plugged in or out.
// Both empty at startup, so only real hot-plugs are announced.
export interface DevicesChanged {
  added: DeviceInfo[];
  removed: DeviceInfo[];
}

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
  // Linux: a gamepad by winebus's rule (the game's XInput device) that SDL
  // has no controller mapping for; its input is mapped like Wine maps it.
  wine_gamepad: boolean;
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
  sdl_serial: string | null;
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

// Payload of `axis-raw` (sent only while `set_axis_stream` is on): a
// device's unfiltered axes, normalized. A joystick: one value per SDL axis
// index, -1..1 (`DeviceInfo::axes` names them). A gamepad: exactly six —
// left x, left y, right x, right y (-1..1), trigger left, trigger right
// (0..1).
export interface AxisRaw {
  instance_id: number;
  guid: string;
  kind: DeviceKind;
  values: number[];
}

// A JoyInput as kept in the Device Info view, with the wall-clock time it
// arrived and the SC token it stood for then (null when SC cannot bind it).
// `id` is a running number, the stable key of a log line.
export type LoggedInput = JoyInput & { id: number; at: number; token: string | null };

// The backend's environment facts (app version, OS, toolkit versions).
// An update channel the App Update dialog offers: `stable`, or a
// channel from the repo's channels.json.
export interface UpdateChannel {
  id: string;
}

// `update_channels`: the channels to pick from and the one a check reads.
export interface ChannelList {
  channels: UpdateChannel[];
  current: string;
}

// Where the input-preview overlay appears: pinned to a screen corner, or
// offset from the mouse pointer.
export type OverlayPosition = "top-left" | "top-right" | "bottom-left" | "bottom-right" | "mouse-offset";

export interface SystemInfo {
  app_version: string;
  os: string;
  arch: string;
  tauri: string;
  webview: string;
  sdl: string;
  // This install checks for and installs its own updates (the Windows
  // installer and the AppImage; not the bare executable, not deb / rpm).
  updater: boolean;
}

// Toast kinds: "ok" done, "error" broken, "warn" degraded but running,
// "hint" guidance.
export type ToastType = "ok" | "error" | "warn" | "hint";

// Top-level GUI mode.
export type Mode = "monitor" | "bindings" | "config" | "devices";

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
  imagemap_choices: Record<string, string>;
  // Back up actionmaps.xml before BindSight overwrites it.
  auto_backup: boolean;
  // Write DEBUG records to bindsight.log (else INFO and up).
  debug_logging: boolean;
  // Check for an update at startup (never installs anything unasked).
  update_check: boolean;
  // The input-preview overlay's size in px (its longer side).
  overlay_size: number;
  // Where the input-preview overlay appears.
  overlay_position: OverlayPosition;
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
// were saved under.
export interface SlotStatus {
  effective_instance: number;
  stored_instance: number | null;
  sc_product_guid: string | null;
  name: string | null;
  clash: boolean;
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

// One resort step: the bindings saved under js{from} belong on js{to}.
export interface ResortMove {
  from: number;
  to: number;
  name: string | null;
}

// A device hidapi lists with a joystick-class interface that SDL does not
// list (Device List only, `input.rs::HidOnlyDevice`).
// A joystick the game lists (its Game.log order) that SDL does not, so no
// input can reach the app from it — e.g. a device on SDL's joystick
// blacklist that Wine still takes through hidraw. The Monitor shows it as a
// tile without input; `hardware_id` is null, an image-map needs input.
export interface LogOnlyJoystick {
  kind: "joystick";
  log_only: true;
  sc_name: string | null;
  sdl_name: string;
  sc_product_guid: string;
  hardware_id: null;
  gamepad_slot: null;
  controller_name: null;
}

// What a Monitor tile stands for: an SDL device or a log-only joystick.
export type TileDevice = DeviceInfo | LogOnlyJoystick;

export interface HidOnlyDevice {
  vid: number;
  pid: number;
  product_guid: string;
  name: string | null;
  // 4 joystick, 5 gamepad, 8 multi-axis.
  usage: number;
  path: string;
  interfaces: HidInterface[];
}

// One HID interface as Wine registers it (Linux, Device List only): the key
// Wine's device order sorts by, and the device it belongs to (its Product
// GUID, `DeviceInfo.sc_product_guid`). A gamepad is registered without a slot.
export interface WineKey {
  product_guid: string;
  key: string;
  is_gamepad: boolean;
}

// A game controller as DirectInput enumerates it (Windows, Device List
// only): diagnostics, never the game's order. `path` is empty when it could
// not be read.
export interface DiDevice {
  product_name: string;
  instance_name: string;
  product_guid: string;
  instance_guid: string;
  dev_type: number;
  usage_page: number;
  usage: number;
  path: string;
}

// A joystick as the game's order has it: its jsN, name and GUID.
export interface JoystickDevice {
  instance: number;
  product_name: string;
  product_guid: string | null;
}

// The joysticks in the game's order (js1, js2, …) and when it was taken.
export interface DeviceOrder {
  joysticks: JoystickDevice[];
  timestamp: string | null;
}

// The saved slots held against the game's joystick order (from the game's own
// Game.log record). With order_error set, everything else is empty and nothing
// is said about the order.
export interface ClashReport {
  connected: SlotStatus[];
  missing: MissingSlot[];
  unseen: UnseenDevice[];
  // When the game last listed its joysticks (the Game.log's own time).
  log_timestamp: string | null;
  // Why there is no order (no readable log, environment not loaded, …).
  order_error: string | null;
  has_clash: boolean;
  resort: ResortMove[];
  // In-game equivalent of `resort`: pp_resortdevices swaps, in order.
  resort_commands: string[];
}

// Last Input card colour: "unseen" (SC does not see the device), "noorder"
// (a joystick while the game's device order is unknown), "bound", "none".
export type LiveState = "noorder" | "bound" | "none";

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

// --- Bindings and Config mode: profiles, backups, compare -------------------

// One exported SC keybinding layout in the binding profiles folder.
export interface BindingProfileSummary {
  file: string;
  name: string;
  bindings: number;
  // File mtime, unix seconds.
  modified: number;
}

// One backup under the app data dir: actionmaps.xml, plus attributes.xml
// where the live one existed.
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
  // The <options> joystick devices of each side (empty for Current); the
  // Apply dialog names the source's devices per slot from a_joysticks.
  a_joysticks: JoystickDevice[];
  b_joysticks: JoystickDevice[];
}

// One device to take over when applying a profile or backup (apply.rs); a
// joystick's bindings and settings can land on another slot (`target`).
export interface DeviceSel {
  kind: DeviceKind;
  instance: number;
  target?: number;
}

// --- Config mode: device settings (devconfig.rs) ------------------------------

// One node of the game's option tree (defaultProfile.xml `<optiongroup>`).
// `show_invert` / `show_curve`: 1 = the node has that control, -1 = a group
// header without one, 0 = not shown.
export interface OptionNode {
  name: string;
  // "" when the game has no label for it.
  label: string;
  show_invert: number;
  show_curve: number;
  default_invert: boolean | null;
  default_exponent: number | null;
  default_points: [number, number][] | null;
  children: OptionNode[];
}

// One option tree per device kind; `nodes` are the visible roots.
export interface OptionTree {
  kind: DeviceKind;
  nodes: OptionNode[];
}

// A node's own values in the file; a curve is an exponent or a point list.
export interface NodeValue {
  invert: boolean | null;
  exponent: number | null;
  points: [number, number][] | null;
}

// One `<options type instance>` element of the file.
export interface OptionsBlock {
  kind: DeviceKind;
  instance: number;
  // The raw Product attribute, byte for byte.
  product: string | null;
  values: Record<string, NodeValue>;
}

// Deadzone / saturation of one input, in the game's display units.
export interface AxisOptionView {
  input: string;
  deadzone: number | null;
  saturation: number | null;
}

// One `<deviceoptions>` element: a joystick's raw Product, "Controller
// (Gamepad)" or "Mouse".
export interface DeviceOptionsView {
  name: string;
  axes: AxisOptionView[];
}

// The mouse settings, display units; null = not set.
export interface MouseView {
  sensitivity: number | null;
  ads_percent: number | null;
  zoom_scaling_enabled: boolean | null;
  zoom_scaling_percent: number | null;
  acceleration: number | null;
  smoothing: number | null;
}

// The device settings of one source (`get_device_config`).
export interface DeviceConfigView {
  options: OptionsBlock[];
  device_options: DeviceOptionsView[];
  mouse: MouseView;
  gamepad_sensitivity: number | null;
  // The game's settings file was read (false for profiles and old backups).
  has_attributes: boolean;
}

// The game's labels for the rows that are not in the option tree.
export interface ConfigLabels {
  joystick_deadzone: Record<string, string>;
  joystick_saturation: Record<string, string>;
  gamepad_deadzone: Record<string, string>;
  gamepad_sensitivity: string;
  mouse_sensitivity: string;
  mouse_ads_percent: string;
  mouse_zoom_scaling_enabled: string;
  mouse_zoom_scaling_percent: string;
  mouse_acceleration: string;
  mouse_smoothing: string;
}

export type CurveValue =
  | { kind: "exponent"; value: number }
  | { kind: "points"; points: [number, number][] }
  | { kind: "default" };

// The settings kept in the game's settings file.
export type ManagedAttribute =
  | "mouse_sensitivity"
  | "ads_percent"
  | "zoom_scaling_enabled"
  | "zoom_scaling_percent"
  | "gamepad_sensitivity";

// One settings change to write (`save_device_config`); values in display
// units. `Invert` with null = Set Default.
export type ConfigChange =
  | { type: "curve"; kind: DeviceKind; instance: number; node: string; value: CurveValue }
  | { type: "invert"; kind: DeviceKind; instance: number; node: string; value: boolean | null }
  | { type: "deadzone"; device: string; input: string; value: number }
  | { type: "saturation"; device: string; input: string; value: number }
  | { type: "mouse_acceleration"; value: number }
  | { type: "mouse_smoothing"; value: number }
  | { type: "attribute"; setting: ManagedAttribute; value: number };

// One device with a chosen image-map, shown on the image stage.
export interface ImageMapView {
  device: DeviceInfo;
  map: ImageMap;
}
