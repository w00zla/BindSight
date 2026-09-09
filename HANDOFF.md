# Handoff — BindSight

Snapshot of where the project stands, for the next session. Pair with `CLAUDE.md`
(project guide).

## What works today

The core loop is closed end-to-end:

1. **Devices**: SDL enumerates connected joysticks; `hidapi` gives SC's own
   device name; the GUID bridge maps each to its SC Product GUID.
2. **SC data** (2026-09-09): extracted from the configured install at runtime.
   `build_manifest.id` gives the game version (shown as a chip in the top bar,
   e.g. `4.10.0-hotfix.12572603`); the bundled StarBreaker sidecar pulls
   `defaultProfile.xml`, `keybinding_localization.xml` and `global.ini` out of
   `Data.p4k` (~1 s), the app converts them to the action master list + token
   labels and caches the JSON per version in the app cache dir. Loading runs
   in the background at start and on base-path change; the Status panel shows
   `Reading SC data…` with a four-segment step bar meanwhile (manifest,
   extracted, converted, cached; `scdata-progress` events) and `No SC data`
   with the error text when the manifest, `Data.p4k` or the sidecar is
   missing or StarBreaker fails. Nothing is bundled any more, so a new SC
   patch needs no app update.
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
8. **Image-maps** (2026-09-09): the "Devices" mode with an editor — pick a
   device, create an image-map via an inline form (name, then the mandatory
   image — picking it creates the image-map; Replace image later keeps the
   areas) or pick one, press an input, draw areas
   (rect / ellipse / polygon / arrow / cw / ccw), save; zip export/import; bundled image-maps from
   `resources/imagemaps/` (none shipped yet). The Live view shows one image
   block per connected device that has an image-map (select when several match
   the hardware id) and lights the active inputs: buttons while
   held, hats until centered, axes as a 400 ms pulse; blue when SC has a
   binding, grey otherwise. A bound input the image-map lacks raises a toast and
   a `not in image-map` tag in the bindings list; clicking a binding row pins
   its area(s), or a toast says why it cannot. While the editor is open the
   Live view ignores joystick input. See `CLAUDE.md` for the data model.

9. **Axes** (2026-09-09): `DeviceInfo::axes` holds the SC axis name per SDL
   axis index, derived from the HID report descriptor (`hid.rs`, read via
   hidapi in `input.rs`, count cross-checked against SDL). `resolve_input`
   handles `kind = "axis"`, so moving an axis shows its binding in the live
   tile (resolved at most every 150 ms per axis) and lights its image-map
   area blue; the bindings list can pin axis areas. Devices whose descriptor
   cannot be read or placed carry `axes_error` (shown on the tile, e.g. a
   `/dev/hidraw` without permission) and get no axis tokens.

10. **Logging** (2026-09-09): `tauri-plugin-log` -> stdout + a rotating
    `bindsight.log` in the app log dir (paths in the README). Startup logs
    OS/app/tauri/webview/SDL versions, all app dirs, the session env and the
    config; then SC version + extraction/cache, device enumeration (per
    device incl. GUIDs and derived axes, only when the list changed), the
    profile's `<options>` and what `Game.log` says, plus user actions (base
    path, exclusions, resort). The webview console and uncaught frontend
    errors are forwarded too (`src/logging.ts`, target `webview`). Per-input
    events are never logged. Severity rules in `CLAUDE.md`.

GUI (redesigned 2026-09-09, phases 0-2 of the plan in the design session;
look = RSI Pledge-Store palette + cyan live accent, Bai Jamjuree / Share Tech
Mono bundled locally, tokens in `src/styles/tokens.css`, icons in
`components/Icon.vue`): top bar with modes **Live / Tools / Devices**, install
slug chip, Refresh, gear (Settings dialog). Live = "Connected devices" panel
(status dot, name, `jsN` chip, counts; a `None` tile when empty) next to a
"Status" panel (`No issues`, or one tile per issue: load error, `No device order
found`, `<name> jsN missing`, `Order clash` with `Fix via config` /
`Fix via console` and slot chips); an image stage with one tile per device
(image-map or `No image-map` placeholder, move left/right buttons, draggable
splitters, shares/order remembered in localStorage); a row splitter; the
"Last input" card (label big, device, one row per bound action + category);
the bindings deck (tabs Bindings / Actions / Log, chips All / jsN, search over
label + action, columns DEVICE / INPUT / LABEL / ACTION / CATEGORY, held input
tinted, rows without an image-map area greyed with a tooltip, click to pin;
Log = device dump + raw events). Settings dialog: install path + Browse, action
labels (bundled / global.ini stub), excluded devices as chips; nothing applies
before Save. Tools = empty placeholder. Devices = the old `ProfileEditor`
(unstyled, "IMAGE-MAPS" title only). Vocabulary and the remaining phases: see
the memory `gui-naming-decisions` and the plan below.

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
- Axis mapping (2026-09-09, VKB Gladiator EVO R, Linux): HID report order is
  X Y Rz Z Rx Ry; SDL reported stick X = 0, Y = 1, slider = 2, twist = 5 —
  canonical usage order, not report order. SC's `deviceoptions` for the same
  device list x y z rotx roty rotz. Rx/Ry exist in the descriptor but have no
  physical control on this grip.
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
- **Axes: Windows unverified.** The SDL-index rule is verified on Linux
  only; on Windows it rests on SDL's DirectInput backend sorting objects by
  offset. Check with `enum_joysticks` + moving the twist there.
- **Axes: hidraw permissions.** Reading the descriptor opens the hidraw
  node; the VKBs open fine, the Keychron Link is `Permission denied` on this
  box (udev). Such devices show `axes: …denied` and get no axis tokens.
- **Axes: Slider/Dial edge cases** are refused, not guessed: more than two
  sliders, or a Dial before a Slider (Linux and DirectInput would order them
  differently). No device with either has been seen.
- The Keychron Link's joystick interface (evdev `js1`, 6 ABS axes) is not
  enumerated by SDL at all today — unexplained, not investigated.
- **`onMounted` fragility (frontend)**: the startup `invoke` chain in `App.vue`
  aborts all following calls on the first failure. Consider a per-call
  try/catch.
- Duplicate identical devices (same GUID, empty serial) can't be told apart —
  known limitation, falls back to order/manual.
- `bindingCountFor` on the device tiles counts shipped defaults too.

### Image-maps — open items (found while building, not built; user decides)

- **Untested in the GUI** as of the 2026-09-09 commits: Konva transform math
  (rect rotation most likely to bite), file dialogs, zip import/export, live
  highlighting, the single-image switch (format 3: the New form with its
  image pick, Replace image), and the axis path (live tile, blue axis areas,
  pinning axis bindings). `pnpm build`, `cargo test` and the headless
  `enum_joysticks` (prints `SC axes: x y z rotx roty rotz` for both VKBs).
- **First bundled image-map**: once one exists, copy its folder to
  `src-tauri/resources/imagemaps/<id>/`.
- Mode switch remounts the editor: **unsaved changes are lost without a
  warning**.
- The pinned binding highlight is not cleared on device change / image-map
  reload.
- `validate()` does not check uniqueness of `areas[].id` nor the image file
  extension; import extracts every zip entry, referenced or not
  (path escapes are rejected). Broken image-map folders are logged to stderr
  only.
- Refresh button also shows in Devices mode (only refreshes devices).
- Axis areas can be drawn and pulse on movement, but have no SC token mapping
  (see the axis item above), so they never turn blue and are not clickable in
  the bindings list.

### GUI redesign — remaining phases (plan in the 2026-09-09 design session)

- **Phase 3 — Devices mode**: rename `hwprofile` -> `imagemap` (rs/ts,
  commands, `resources/profiles/` -> `resources/imagemaps/`, app-data folder,
  config key `profile_choices` -> `imagemap_choices`; no migration, nothing
  existed yet) — **rename done (2026-09-09)**; bundled
  image-maps become hard read-only (no edit, no delete, no silent fork) with a
  new `clone_imagemap(id, name)` command — **read-only + clone_imagemap done
  (2026-09-09)**; editor UI per the canvas (device
  list without state, image-map list with lock / clone / trash, shape-tool
  icon bar, SDL-key input card, areas list, Discard / Save, unsaved-changes
  guard). Zero SC data in that mode.
- **Phase 4 — Tools mode**: `mappings.rs` (SC mapping-profile XMLs in
  `<base>/user/client/0/controls/mappings/`, import/export), `backups.rs`
  (`<app_data>/backups/<ts>/` + `meta.json` reason; `apply_resort` writes
  there instead of the `.bak`), `diff.rs` (`diff_bindings`), then the Tools UI
  (profiles list, backups list with restore, Compare A/B with chips).
  Needs a real SC-exported mapping XML to verify the parser first.
- **Phase 5 — cleanup**: dead code, docs, first bundled image-map.
- Open: source for the game version chip (`build_manifest.id` next to
  `Data.p4k`?); `labels_source` in `config.rs` for the global.ini stub.
- **GUI-unverified after the redesign**: splitter dragging and the folder
  dialog under WebKitGTK, image fitting with a real image-map, the Status
  panel with a real clash / missing device / missing Game.log.

## Testing

- `cd src-tauri && cargo test --lib` — 49 tests (+1 ignored). The ignored one
  (`converts_real_hardware_guids`) checks the author's real GUIDs; run with
  `cargo test -- --ignored`.
- Frontend: `pnpm build` (vue-tsc typechecks, `noUnusedLocals` is on).
- The real GUI test is `pnpm tauri dev` with a configured SC base path — Claude
  can't run the GUI headless, so that verification is the user's.
- Evidence for the device-order facts: `temp/joyenumtest/` (gitignored) —
  fresh/rebound actionmaps per platform, `i_DumpDeviceInformation` screenshots,
  `enum_joysticks` output.

## Working data

The extracted SC files live only transiently in the cache dir during a load;
the cached JSON per version is under `<app_cache_dir>/v1/<label>/` (Linux:
`~/.cache/com.w00zla.bindsight/`). For offline parsing (the `parse_*`
examples) extract them by hand:
`src-tauri/binaries/starbreaker-<triple> p4k extract --p4k <Data.p4k> -o <dir> --regex '...' --convert cryxml`
(regex in `scinstall.rs`).

## Gotchas

- `node_modules` on the shared NTFS mount is per-OS: a `pnpm install` on one OS
  purges the other's native bindings (rolldown). Re-run `pnpm install` after
  switching OS. `core.fileMode=false` is set so the mount's +x bits don't show
  up as changes.
