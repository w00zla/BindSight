// Shared frontend types for data coming from the Rust backend.

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
