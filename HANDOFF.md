# Handoff — BindSight

Snapshot of where the project stands, for whoever picks it up next. Pair
with `CLAUDE.md` (project guide).

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
   in the background at start and on environment change; the Status panel shows
   `Reading SC data…` with a four-segment step bar meanwhile (manifest,
   extracted, converted, cached; `scdata-progress` events) and `No SC data`
   with the error text when the environment fails validation (folder
   missing, or any of `Data.p4k`, `build_manifest.id`, `actionmaps.xml`
   missing — listed in one message; nothing else is read then, also not on
   Refresh), or when the sidecar is missing or StarBreaker fails. Nothing is bundled any more, so a new SC
   patch needs no app update.
3. **Bindings**: the app reads the configured install's `actionmaps.xml`
   (rebinds + `<options>` device map) and folds in the shipped `js1` defaults
   from `defaultProfile.xml` for every action the user never rebound (tagged
   `is_default`). A blank js rebind (`js1_ `) counts as "deliberately unbound";
   a keyboard/mouse/gamepad-only rebind leaves the joystick default in place.
4. **Monitor**: pressing a button/hat shows the resolved action(s) in the live tile
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
   backed up via `backups.rs` first, reason "before Fix via config"; profile
   reloaded).
7. **Exclude**: a per-device "Exclude always" (`ignored_devices` in the per-OS
   `config.json`) declares a device SC never sees (e.g. a keyboard Wine hides);
   it then counts as unplugged.
8. **Image-maps** (2026-09-09): the "Devices" mode with an editor — pick a
   device, create an image-map via an inline form (name, then the mandatory
   image — picking it creates the image-map; Replace image later keeps the
   areas) or pick one, press an input, draw areas
   (rect / ellipse / polygon / arrow / cw / ccw), save; zip export/import; bundled image-maps from
   `resources/imagemaps/` (none shipped yet). The Monitor view shows one image
   block per connected device that has an image-map (the map is picked in Devices
   mode via the check icon per map, since 2026-09-10; the stage caption shows
   only the device name) and lights the active inputs: buttons while
   held, hats until centered, axes as a 400 ms pulse; blue when SC has a
   binding, grey otherwise. A bound input the image-map lacks raises a toast and
   a `not in image-map` tag in the bindings list; clicking a binding row pins
   its area(s), or a toast says why it cannot. While the editor is open the
   Monitor view ignores joystick input. See `CLAUDE.md` for the data model.

9. **Axes** (2026-09-09): `DeviceInfo::axes` holds the SC axis name per SDL
   axis index, derived from the HID report descriptor (`hid.rs`, read via
   hidapi in `input.rs`, count cross-checked against SDL). `resolve_input`
   handles `kind = "axis"`, so moving an axis shows its binding in the live
   tile (resolved at most every 150 ms per axis) and lights its image-map
   area blue; the bindings list can pin axis areas. Devices whose descriptor
   cannot be read or placed carry `axes_error` (Devices log and stderr only, never
   on the device tiles, e.g. a `/dev/hidraw` without permission) and get no
   axis tokens.

10. **Logging** (2026-09-09): `tauri-plugin-log` -> stdout + a rotating
    `bindsight.log` in the app log dir (paths in the README). Startup logs
    OS/app/tauri/webview/SDL versions, all app dirs, the session env and the
    config; then SC version + extraction/cache, the profile's `<options>`
    and what `Game.log` says, plus user actions (environments, exclusions,
    resort). Device runtime detail is deliberately absent (only hidapi /
    joystick-open failures): the Devices mode's device log has it all. The
    webview console and uncaught frontend errors are forwarded too
    (`src/logging.ts`, target `webview`). Severity rules in `CLAUDE.md`.

11. **Keyboard + gamepad** (2026-09-10): every feature now covers SC's one
    keyboard (`kb1`) and one gamepad (`gp1`) next to the joysticks.
    `DeviceInfo.kind` / `hardware_id` / `gamepad_slot`; the first SDL game
    controller is `gp1` (further pads: `no slot`), the keyboard is a
    synthetic device whose keys are captured in the webview
    (`src/keyboard.ts`, `KeyboardEvent.code` -> SC scancode name, in every
    mode with `preventDefault` on every mapped key — the coming rebind flow
    relies on that — never in text fields or over a dialog). Pads emit `padbutton` / `padaxis` with SC names incl. the
    derived `triggerl_btn` / `thumbl_left` … buttons (backend, 50 %
    threshold). Defaults come from `keyboard=` / `gamepad=` (attribute or
    child form), token labels from the `keyboard` / `control_pad` sections,
    rebinds keep their modifiers (`kb1_lalt+x`); the live tile resolves
    kb/gp through `resolve_tokens` (held-name combos first, plain token
    last). Image-map keys `key:<name>` / `pad:<name>`, `hardware_id`
    `keyboard` / `gamepad`; four bundled image-maps (Keyboard US / DE, Xbox /
    PlayStation) generated by `scripts/gen-imagemaps.py`. Deck and Compare
    chips `kb1` / `gp1`, Game.log's `Connected xinputN` line feeds
    `gamepad_seen`. Each device tile has an eye button (bottom left) hiding its stage tile
    (`bindsight.stage.hidden`, hardware ids). Device lists and the stage's
    default tile order follow `deviceRank` in `App.vue`: keyboard, slotted
    pad, joysticks SC sees, then everything unseen / excluded / without a
    slot. **The SC-data cache shape
    changed** (`Action` gained two fields, `tokens.json` the `kb1_` / `gp1_`
    labels); serde would load an old cache with the new fields silently
    `None`, so `scdata.json` now carries a `format` stamp (`CACHE_FORMAT` =
    2, 2026-09-10) and an older or stampless cache is re-extracted once.

GUI (redesigned 2026-09-09, phases 0-2 of the plan in the design session;
look = RSI Pledge-Store palette + cyan live accent, Bai Jamjuree / Share Tech
Mono bundled locally, tokens in `src/styles/tokens.css`, icons in
`components/Icon.vue`): top bar with modes **Monitor / Bindings / Devices**
(renamed 2026-09-09 from Live / Tools; code ids `live` / `tools` unchanged),
environment chip (active slug, dropdown to switch), version chip, Refresh,
gear (Settings dialog). Monitor = "Connected devices" panel (status dot,
name, `jsN` chip, counts; a `None` tile when empty) next to a
"Status" panel (`No issues`, or one tile per issue: load error, `No device order
found`, `<name> jsN missing`, `Order clash` with `Fix via config` /
`Fix via console` and slot chips); an image stage with one tile per device
(image-map or `No image-map` placeholder, move left/right buttons, draggable
splitters, shares/order remembered in localStorage); a row splitter; the
"Last input" card (label big, device, one row per bound action + category);
the bindings deck ("Bindings" panel title, only bindings of connected
devices (2026-09-10), toggle chips kb1 / gp1 / jsN (none = all), search over
input + action, columns DEVICE / INPUT (SC's label, else the bare token in
mono) / ACTION / CATEGORY, held input
tinted, rows without an image-map area greyed with a tooltip, click to pin).
Settings dialog (2026-09-09): one block per environment (LIVE / HOTFIX /
PTU / EPTU: path + Browse, "Override global.ini" checkbox with file path +
Browse), excluded devices as chips; nothing applies before Save
(`set_environments`, reload only when the active one changed). The top-bar
chip shows the active environment and opens a dropdown to switch it
(`set_active_env`, reload). Bindings =
`ToolsView`: on the left the "Game bindings" panel (SC's action list grouped
by category, capped at 40 % of the column), binding profiles and backups
(Import / Export / New, Backup now, restore and delete behind
`ConfirmDialog`), the Compare panel on the right (A/B source chips, kind and
diff-kind and device toggle chips with counts (none toggled = all, like
the deck),
search, one tinted row per differing token). Devices =
`ImageMapEditor` (reworked 2026-09-10): left the "Image-maps" panel —
devices + their image-maps sorted by name, one row = check (the map the
Monitor shows, live colour) / name / lock (bundled), New under the rows,
Import / Export in the foot — and the "System" panel with the "Device log"
toggle; an action tile spanning the centre and right columns with what
can be done with the open map (view: Use for device | Edit (disabled for
bundled), Clone, Delete; edit: Use for device | Cancel, Save | Choose image
(replaces the image, keeps the areas) — Cancel drops the changes, Save
writes them, both return to view mode; a new not-yet-created map: only
Choose image); the Konva canvas
with the name (plain text in view mode, input in edit mode), the six shape
tools and zoom (both edit only, zoom resets on leaving); the live SDL-key
card with Add area / Delete (edit mode only) and the areas list with
filter. A map opens in view mode
(canvas inert, areas still light up for the live key); Edit, a fresh clone
or a fresh map enter edit mode, Cancel / Save leave it (the Save / Discard
question only comes when switching map or device while dirty). "New image-map" opens the empty state with a yellow `No image-map`
chip and only Choose image, which creates the map (name = device name) and
opens it in edit mode. Unsaved changes are guarded by `ConfirmDialog`. The Device log
toggle swaps the canvas (and the right column) for the raw log with Clear
and Save (text file via the `write_text_file` command): per device every
SDL fact (names, GUIDs, index / instance / type / path, vendor / product /
version, power, counts, rumble / led), the derived SC axes or the error,
the HID input fields in report order, every hidapi interface of the
vendor/product (usage, bus, release, strings, path) and the raw report
descriptor; then the last 500 input events from every mode with wall-clock
time, SDL timestamp, device, SC axis name, raw and normalised axis value,
raw hat state.

**Tables** (2026-09-09): the bindings deck and Compare share
`tableColumns.ts` + `ColumnHead.vue` — click a header to sort (default:
the deck by ACTION, Compare by INPUT, ascending, natural order, tie-break
by token), drag the grip in the
column gap to resize, double-click to reset; the last column is the `1fr`
filler, the others have px defaults, the user's widths and the sort persist
in localStorage (`bindsight.columns.<table>`). Rows keep a min-width so a
narrow panel scrolls horizontally under a sticky header; every cell
truncates with an ellipsis. Vocabulary and the remaining phases: see the
memory `gui-naming-decisions` and the plan below.

## Verified facts (VKB Gladiator EVO L/R + Keychron K2 HE, Linux/Wine and Windows)

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

- **Keyboard + gamepad (2026-09-10), partly GUI-verified**: the app starts,
  lists the `Keyboard` tile with its `kb1` chip, shows the eye buttons and
  the bundled DE keyboard map in the stage (screenshot). **Not verified**:
  any key press (no input-injection tool on the dev box), everything
  gamepad (no pad was connected: `is_game_controller`, the double open, the
  `Joy*` drop, `padbutton`/`padaxis`, derived buttons, `gp1` tile, the
  PlayStation default by vendor `054C`), the hide toggle, kb/gp chips in
  Deck and Compare, the editor with the keyboard/pad selected. Known
  rough edges: the capture `preventDefault`s webview shortcuts in every
  mode, by design (the editor has no key shortcuts any more — polygon by
  double-click, delete by button — so a keyboard map can be edited without
  side effects; only the top bar's Escape still closes the environment
  dropdown, harmless); the bundled defaults per kind
  (`DEFAULT_MAPS` in `App.vue`: US keyboard, Xbox pad, PlayStation pad for
  vendor `054C`, else the first matching map) are unverified; the keyboard cannot be
  excluded, only hidden; the Status panel says nothing about an unseen pad
  (tile + live card do); a `Game.log` with only `xinput` lines now yields
  `missing` for every saved joystick instead of "no device order"; extra
  pads still resolve nothing but do highlight and log; under real Windows
  DirectInput may list an XInput pad as a joystick too — unchecked.
- **Session 2026-09-09 (evening), all GUI-unverified**: sortable/resizable
  tables, the Monitor / Bindings rename, the Devices log with Save, the Game
  bindings panel, "Open log folder", the environments (Settings blocks,
  chip dropdown, a real global.ini override load), install validation
  (folder / missing-files messages, Refresh staying quiet afterwards), the
  Image-maps / System panels in Devices, and the Current row vanishing
  without an actionmaps.xml. `write_text_file` writes to any path the
  frontend hands it — deliberately without a path check (only the save
  dialog feeds it).
- **Resort is GUI-untested** (2026-09-09): backend covered by tests plus a
  round-trip smoke test on the real `temp/joyenumtest/actionmaps_live.xml`
  (swap 1<->2 and back = byte-identical). The banner, the Copy button
  (`navigator.clipboard` under WebKitGTK/WebView2 — may need the Tauri
  clipboard plugin) and the Rewrite button are unverified in the app.
- **`pp_resortdevices` semantics beyond a 2-swap are unverified**: in-game
  experience says it moves all bindings of `jsA` to `jsB`; the command list assumes
  B's bindings come back to A (a swap, as NOMAN's guide says), so a 3+-cycle
  is emitted as a chain of swaps anchored on the cycle's first slot. Test
  in-game with 3 devices before trusting a multi-command list.
- **After an in-game resort** SC rewrites `actionmaps.xml`; the app reloads
  the profile via Refresh (full: devices, actionmaps.xml, Game.log), on a
  base-path change, or its own Rewrite. Hot-plug (`devices-changed`) and
  startup only re-list devices (2026-09-09; SDL raises one event per device
  at start, the install has not changed).
- **SC data (2026-09-09)**: `scripts/fetch-starbreaker.ps1` is untested
  (no PowerShell here; `-MaximumRetryCount` needs PowerShell 6+, drop it for
  5.1). Old cache versions under `<app_cache_dir>/` are never cleaned
  (~100 KB per SC patch; the `v1/` folder layer was dropped on 2026-09-09 in
  favour of the `format` stamp inside `scdata.json`, an existing `v1/`
  folder is just dead weight). The StarBreaker version is
  pinned in both fetch scripts; bump version + SHA256 there when updating.
  No re-extract button: delete the version's cache folder to force one.
- **Logging (2026-09-09)**: Settings has "Open log folder" (`open_log_dir`
  command via the opener plugin, GUI-unverified); the README lists the
  paths. Frontend `console.*` is forwarded, but the frontend itself only
  logs `frontend mounted` so far.
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

### Image-maps — open items (found while building, not built)

- **Untested in the GUI** as of the 2026-09-09 commits: Konva transform math
  (rect rotation most likely to bite), file dialogs, zip import/export, live
  highlighting, the single-image switch (format 3: the New form with its
  image pick, Replace image), and the axis path (live tile, blue axis areas,
  pinning axis bindings). `pnpm build`, `cargo test` and the headless
  `enum_joysticks` (prints `SC axes: x y z rotx roty rotz` for both VKBs).
  The **whole Devices UI is GUI-unverified** — the rebuilt three-column
  editor (zoom/fit, tool toggles, area list, guards) has only been through
  `pnpm build`.
- **Bundled image-maps** (2026-09-10): four generated ones ship; joystick
  maps are still user-made only. Regenerate with `scripts/gen-imagemaps.py`
  (`--overlays DIR` renders review overlays). The PS map shows only the PS
  glyphs, not SC's `a b x y` names.
- The pinned binding highlight is not cleared on device change / image-map
  reload.
- `validate()` does not check uniqueness of `areas[].id` nor the image file
  extension; import extracts every zip entry, referenced or not
  (path escapes are rejected). Broken image-map folders are logged to stderr
  only.
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
  guard) — **done (2026-09-09)** as `ImageMapEditor` (renamed from the old
  editor) plus a new `ConfirmDialog`. Zero SC data in that mode.
- **Phase 4 — Bindings mode** (was "Tools mode"): `binding_profiles.rs` (SC binding-profile XMLs in
  `<base>/user/client/0/controls/mappings/`, import/export) — **done
  (2026-09-09)**: `BindingProfileSummary` (file, name from `profileName` else the
  file stem, joystick binding count via `resolve_bindings`, mtime), `list`/
  `import`/`export` as plain file-copy in/out of `controls/mappings/` —
  applying a layout into the live `actionmaps.xml` is explicitly not this
  module's job; verified against a real export (since deleted with the old
  `data/` folder). `backups.rs` —
  **done (2026-09-09)**: one folder per backup (`meta.json` + `actionmaps.xml`)
  under `<app_data>/backups/<id>/`, id = `YYYYMMDD-HHMMSS` (UTC) with a
  collision suffix; `create`/`list`/`path_of`/`delete`/`restore` (restore
  takes a safety backup first, reason "before restore") plus the four Tauri
  commands; `apply_resort` now backs up through it (reason "before Fix via
  config") instead of the old `actionmaps.xml.<ts>.bak` copy. `diff.rs` —
  **done (2026-09-09)**: `diff_bindings(a, b)` compares two resolved binding
  sets by SC token, reporting per token whether it is bound in A only
  (added), B only (removed), or both with a different `(actionmap, action)`
  set (changed) — a label-only difference never counts; plus the
  `compare_bindings` command with a `Source` (Current/Profile/Backup) per
  side. The Bindings-mode UI — **done (2026-09-09)** as `ToolsView`: binding-profile
  list (Current row + one row per layout, Import / Export, New disabled),
  backup list (Backup now, restore / delete via `ConfirmDialog`, restore hands
  the `LoadStatus` back to `App.vue`), and the Compare panel (A/B source
  selects, kind and `jsN` filter chips, search, tinted diff rows).
- **Phase 5 — cleanup**: dead code, docs, first bundled image-map.
- Open: source for the game version chip — done (`build_manifest.id`); the
  global.ini stub became the per-environment override (2026-09-09).
- **GUI-unverified after the redesign**: splitter dragging and the folder
  dialog under WebKitGTK, image fitting with a real image-map, the Status
  panel with a real clash / missing device / missing Game.log, and the whole
  Bindings-mode UI (import/export file dialogs, backup create / restore /
  delete, Compare against a real layout). Also GUI-unverified (2026-09-09):
  the Game bindings panel's 40 % cap, the Devices log view and its Save
  dialog, the environment dropdown and the Settings environment blocks
  (incl. a real global.ini override load).

## Testing

- `cd src-tauri && cargo test --lib` — 90 tests (+1 ignored). The ignored one
  (`converts_real_hardware_guids`) checks GUIDs of one specific setup; run with
  `cargo test -- --ignored`.
- Frontend: `pnpm build` (vue-tsc typechecks, `noUnusedLocals` is on).
- The real GUI test is `pnpm tauri dev` with a configured SC environment.
  When another session already holds port 1420, build a self-contained
  binary instead: `pnpm tauri build --debug --no-bundle`, run
  `src-tauri/target/debug/bindsight`, read `bindsight.log`, and screenshot
  under KDE/Wayland by raising the window with a KWin script
  (`workspace.activeWindow = w` for the caption `BindSight`, loaded via
  `qdbus org.kde.KWin /Scripting loadScript`) and `spectacle -a -b -n -o`.
- The SC-data failure paths (step bar, red tile, empty bindings) are
  testable without touching the config: replace `target/debug/starbreaker`
  (Tauri re-copies it on every build) with a shell wrapper that sleeps or
  exits 1, and delete `~/.cache/com.w00zla.bindsight` to force a load.
- Evidence for the device-order facts: `temp/joyenumtest/` (gitignored) —
  fresh/rebound actionmaps per platform, `i_DumpDeviceInformation` screenshots,
  `enum_joysticks` output.

## Working data

The extracted SC files live only transiently in the cache dir during a load;
the cached JSON per version is under `<app_cache_dir>/<label>/` (Linux:
`~/.cache/com.w00zla.bindsight/`). For offline parsing (the `parse_*`
examples) extract them by hand:
`src-tauri/binaries/starbreaker-<triple> p4k extract --p4k <Data.p4k> -o <dir> --regex '...' --convert cryxml`
(regex in `scinstall.rs`).

## Gotchas

- `node_modules` on the shared NTFS mount is per-OS: a `pnpm install` on one OS
  purges the other's native bindings (rolldown). Re-run `pnpm install` after
  switching OS. `core.fileMode=false` is set so the mount's +x bits don't show
  up as changes.
