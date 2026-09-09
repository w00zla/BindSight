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
   written by SC at every start) is the **only** source for SC's `jsN` order and
   for which devices SC sees at all. It is compared against the saved
   `<options>`: rank mismatch = clash, saved device not in SC's list = missing
   (the cause of the shift), SDL device not in SC's list = unseen (hidden by
   Wine, or plugged in after SC started). Without a usable log there is no
   order at all (2026-09-09: the SDL-derived fallback was removed — no guessed
   `jsN`, no "order unknown"); the GUI says why (`Game.log not found: <path>` /
   `lists no joysticks`) and shows nothing else.
6. **Resort** (2026-09-09): the clash report carries the slot permutation
   that puts every listed device's bindings on the `jsN` SC now assigns it
   (`plan_resort`: saved slot -> SC slot per device; leftover slots — saved
   ones of devices SC does not list, SC ones holding unsaved devices — are
   paired off ascending, so dangling bindings are renumbered, never dropped).
   Two ways to apply it, both in the clash banner: the in-game
   `pp_resortdevices joystick A B` commands (one per swap, cycle-decomposed,
   Copy button) and the out-of-game "Rewrite actionmaps.xml" button
   (`resort.rs`: textual rewrite of joystick `<options>` instances and `jsN_`
   prefixes in `input="..."`, re-emitting the joystick blocks in slot order;
   backup `actionmaps.xml.<unix time>.bak` next to it; profile reloaded).
7. **Exclude**: a per-device "Exclude always" (`ignored_devices` in the per-OS
   `config.json`) declares a device SC never sees (e.g. a keyboard Wine hides);
   it then counts as unplugged.
8. **HW profiles** (2026-09-09): a "HW profiles" mode with an editor — pick a
   device, create a profile via an inline form (name, variant, and the
   mandatory image the areas are drawn on; Replace image later keeps the
   areas) or pick one, press an input, draw areas
   (rect / ellipse / polygon / arrow / cw / ccw), save; zip export/import; bundled profiles from
   `resources/profiles/` (none shipped yet). The Live view shows one image
   block per connected device that has a profile (select when several match
   the hardware id) and lights the active inputs: buttons while
   held, hats until centered, axes as a 400 ms pulse; blue when SC has a
   binding, grey otherwise. A bound input the profile lacks raises a toast and
   a `not in HW profile` tag in the bindings list; clicking a binding row pins
   its area(s), or a toast says why it cannot. While the editor is open the
   Live view ignores joystick input. See `CLAUDE.md` for the data model.

GUI: mode switch (Live / HW profiles), config (SC base path), device tiles
(name, `✓ jsN` / clash / `not seen by SC` / `excluded`, counts, Exclude
toggle), Game.log line with local timestamp, clash banner (missing slots,
resort moves, commands + Copy, Rewrite button), live tile, HW
profile images, bindings list (with `default` / `not in HW profile` tags,
click to pin), actions list, live event log, toasts.

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

- **Resort is GUI-untested** (2026-09-09): backend covered by tests plus a
  round-trip smoke test on the real `temp/joyenumtest/actionmaps_live.xml`
  (swap 1<->2 and back = byte-identical). The banner, the Copy button
  (`navigator.clipboard` under WebKitGTK/WebView2 — may need the Tauri
  clipboard plugin) and the Rewrite button are unverified in the app.
- **`pp_resortdevices` semantics beyond a 2-swap are unverified**: the user
  states it moves all bindings of `jsA` to `jsB`; the command list assumes
  B's bindings come back to A (a swap, as NOMAN's guide says), so a 3+-cycle
  is emitted as a chain of swaps anchored on the cycle's first slot. Test
  in-game with 3 devices before trusting a multi-command list.
- **After an in-game resort** SC rewrites `actionmaps.xml`; the app reloads
  the profile only via Load (base path) or its own Rewrite — hit Load.
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

### HW profiles — open items (found while building, not built; user decides)

- **Untested in the GUI** as of the 2026-09-09 commits: Konva transform math
  (rect rotation most likely to bite), file dialogs, zip import/export, live
  highlighting, and the single-image switch (format 2: the New form with
  its image pick, Replace image). `pnpm build` and `cargo test` only.
- **First bundled profile**: once one exists, copy its folder to
  `src-tauri/resources/profiles/<id>/`.
- Mode switch remounts the editor: **unsaved changes are lost without a
  warning**.
- The pinned binding highlight is not cleared on device change / profile
  reload.
- `validate()` does not check uniqueness of `areas[].id` nor the image file
  extension; import extracts every zip entry, referenced or not
  (path escapes are rejected). Broken profile folders are logged to stderr
  only.
- Refresh button also shows in HW profiles mode (only refreshes devices).
- The device tile's `not in profile` means SC's binding profile
  (`actionmaps.xml`) — consider `not in actionmaps` to keep it apart from HW
  profiles.
- Symbol transformer keeps symbols square (x scale wins).
- Axis areas can be drawn and pulse on movement, but have no SC token mapping
  (see the axis item above), so they never turn blue and are not clickable in
  the bindings list.

## Testing

- `cd src-tauri && cargo test --lib` — 44 tests (+1 ignored). The ignored one
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
