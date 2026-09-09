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
3. **Bindings**: the app reads the configured install's `actionmaps.xml`
   (rebinds + `<options>` device map) and folds in the shipped `js1` defaults
   from `defaultProfile.xml` for every action the user never rebound (tagged
   `is_default`). A blank js rebind (`js1_ `) counts as "deliberately unbound";
   a keyboard/mouse/gamepad-only rebind leaves the joystick default in place.
4. **Live**: pressing a button/hat shows the resolved action(s) in the live tile
   — blue = bound, grey = nothing bound, yellow = SC doesn't see the device.
   Without an SC token the SDL input name (`button 5`, `hat 0 up`) is shown.
5. **Device order / clash**: `Game.log` (`Connected joystickN: <Product {GUID}>`,
   written by SC at every start) is the primary source for SC's `jsN` order and
   for which devices SC sees at all. It is compared against the saved
   `<options>`: rank mismatch = clash, saved device not in SC's list = missing
   (the cause of the shift), SDL device not in SC's list = unseen (hidden by
   Wine, or plugged in after SC started). Without a usable log the order is
   derived from SDL (Windows: reversed; Linux: unknown, no rank clash asserted)
   and the GUI says why (`Game.log not found: <path>` / `lists no joysticks`).
6. **Exclude**: a per-device "Exclude always" (`ignored_devices` in the per-OS
   `config.json`) declares a device SC never sees (e.g. a keyboard Wine hides);
   it then counts as unplugged.

GUI: config (SC base path), device tiles (name, `✓ jsN` / clash / `not seen by
SC` / `excluded`, counts, Exclude toggle), Game.log line with local timestamp,
clash banner, live tile, bindings list (with `default` tags), actions list,
live event log, toasts.

## Verified facts (this hardware)

- SDL button `i` == SC `button(i+1)` (the +1 offset holds for buttons).
- SC assigns `jsN` purely by enumeration position and ignores name/GUID — the
  reason `pp_resortdevices` is ever needed. `jsN` == `i_DumpDeviceInformation`
  / `Game.log` `joystickN` + 1.
- SDL enumeration order != SC `jsN` order on both platforms. Windows: the exact
  reverse (one sample). Linux/Wine: no relation — SC's order stayed put across
  a reboot that reshuffled the evdev order. GUID is the only identity link;
  `Game.log` is the only order source.
- Under Wine, SC does not see a Keychron K2 HE keyboard at all (SDL/evdev does);
  under Windows it is a full joystick. Why Wine hides it is unknown — and
  irrelevant thanks to Game.log + Exclude.
- `actionmaps.xml` can be a cross-platform hybrid (profile imports carry
  `<options>`/`<deviceoptions>` written on the other OS). Never assume the saved
  `<options>` were produced on the current platform.
- `defaultProfile.xml` joystick defaults are unnumbered (`button1`, `x`, …) and
  apply to `js1`.
- SC device name == HID product string (same on Win10 and Wine).
- The live event pump works on a background thread on Linux.

## Open items / next steps

- **Stufe 2 — the remap fix**: compute the permutation from Game.log order vs.
  the saved `<options>` and show the ready-made `pp_resortdevices` command.
  No write to actionmaps.xml (live game config — the user applies it in-game).
- **Windows reverse rule is n=1** and unproven across boots; only a fallback
  behind Game.log, but a second sample is cheap (`enum_joysticks` +
  `i_DumpDeviceInformation` after a replug).
- **Game.log staleness**: it reflects the last game start; an SDL device not in
  it is either hidden or plugged in later — Exclude disambiguates by hand.
- **Axis highlight**: `resolve_input` handles buttons + hats only. Axes need a
  token mapping from HID usages (X->js_x, Rz->js_rotz, Slider->js_slider1) — the
  planned `hidapi` usage work.
- **`onMounted` fragility (frontend)**: the startup `invoke` chain in `App.vue`
  aborts all following calls on the first failure. Consider a per-call
  try/catch.
- Duplicate identical devices (same GUID, empty serial) can't be told apart —
  known limitation, falls back to order/manual.
- `bindingCountFor` on the device tiles counts shipped defaults too.

## Testing

- `cd src-tauri && cargo test --lib` — 30 tests (+1 ignored). The ignored one
  (`converts_real_hardware_guids`) checks the author's real GUIDs; run with
  `cargo test -- --ignored`.
- Frontend: `pnpm build` (vue-tsc typechecks).
- The real GUI test is `pnpm tauri dev` with a configured SC base path — Claude
  can't run the GUI headless, so that verification is the user's.
- Evidence for the device-order facts: `temp/joyenumtest/` (gitignored) —
  fresh/rebound actionmaps per platform, `i_DumpDeviceInformation` screenshots,
  `enum_joysticks` output.

## Working data

`data/extracted/` (gitignored) holds the user's extracted `defaultProfile.xml`,
`global.ini`, `keybinding_localization.xml` and a working copy of `actionmaps.xml`
for offline parsing/converting.

## Gotchas

- `node_modules` on the shared NTFS mount is per-OS: a `pnpm install` on one OS
  purges the other's native bindings (rolldown). Re-run `pnpm install` after
  switching OS. `core.fileMode=false` is set so the mount's +x bits don't show
  up as changes.
