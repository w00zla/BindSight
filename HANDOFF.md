# Handoff — BindSight

Snapshot of where the project stands, for the next session. Pair with `CLAUDE.md`
(project guide).

## What works today

The core loop is closed end-to-end:

1. **Devices**: SDL enumerates connected joysticks; `hidapi` gives SC's own
   device name; the GUID bridge maps each to its SC Product GUID.
2. **SC data**: `defaultProfile.xml` + `global.ini` are pre-converted to the
   bundled `scdata.json` (action master list + labels, per-instance token labels
   in `tokens.json`).
3. **User bindings**: the app reads the configured install's `actionmaps.xml`,
   overlays the rebinds, and lists every joystick binding with its SC label.
4. **Live**: pressing a button/hat highlights the resolved action(s) in the live
   tile, with the localized token, `jsN` badge and localized category.
5. **Controller match**: each binding shows a green check + device when the
   recorded device is connected (by GUID), else an orange "not connected".

GUI: config (SC base path), device tiles (name + green `✓ jsN` + counts), live
tile, bindings list, actions list (with category tag), live event log, toasts.

## Verified facts (this hardware / Wine)

- SDL button `i` == SC `button(i+1)` (the +1 offset holds for buttons).
- SDL enumeration order != SC `jsN` order — GUID is the only reliable link.
- SC device name == HID product string (same on Win10 and Wine).
- The live event pump works on a background thread on Linux.

## Open items / next steps

- **Index/slot clash (the "manageable" part the user wants)**: the current match
  is presence-only (is the recorded device connected?). Detecting that a device
  is present but SC would assign it the *wrong* `jsN` (enum-order clash) needs
  SC's live enumeration order and is the **instance-remap feature**. Not started.
- **Axis highlight**: `resolve_input` handles buttons + hats only. Axes need a
  token mapping from HID usages (X->js_x, Rz->js_rotz, Slider->js_slider1) — the
  planned `hidapi` usage work.
- **`onMounted` fragility (frontend)**: the startup `invoke` chain in `App.vue`
  aborts all following calls on the first failure (this already caused a
  "base path disappeared" scare when `get_tokens` was unregistered). Consider a
  per-call try/catch.
- Duplicate identical devices (same GUID, empty serial) can't be told apart —
  known limitation, falls back to order/manual.

## Testing

- `cd src-tauri && cargo test --lib` — 13 tests (+1 ignored). The ignored one
  (`converts_real_hardware_guids`) checks the author's real GUIDs; run with
  `cargo test -- --ignored`.
- Frontend: `pnpm build` (vue-tsc typechecks).
- The real GUI test is `pnpm tauri dev` with a configured SC base path — Claude
  can't run the GUI headless, so that verification is the user's.

## Working data

`data/extracted/` (gitignored) holds the user's extracted `defaultProfile.xml`,
`global.ini`, `keybinding_localization.xml` and a working copy of `actionmaps.xml`
for offline parsing/converting.
