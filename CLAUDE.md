# BindSight — Claude project guide

Star Citizen binding visualizer and mapper. Answers "what does each button
do, and where does each action live?" by joining SC's config with live input
from joysticks, gamepads, keyboard and mouse.
Secondary: the tools the game lacks — editing bindings without starting it,
binding profiles, backups, applying a profile or backup per device, comparing
two binding sets, and fixing the joystick order.

## Design philosophy (the maintainer's, and the top rule)

An app is worthless if it does not follow its intention as well as it can.
BindSight's intention: **as simple and powerful as possible, filling the
gaps SC leaves** (missing features, bad UX). It is an **addon to the game**
and replicates the game's data and behaviour as closely as it can, so the
user never has to translate between the two.

- **Labels, descriptions, tokens: 1:1 with the game.** Only what SC itself
  shows is shown; nothing synthesized. If SC has no label (a `kb1_lalt+x`
  combo), the raw token is the correct display, not an invented "Left Alt +
  X".
- **Behaviour: what SC does.** A combo resolves only while the modifier is
  held, a rebind replaces the kind's binding, defaults come from
  `defaultProfile.xml` — because that is what the game does. Where SC's
  behaviour is unknown, say so; do not guess a nicer one.
- **Before proposing a nicer name or a "helpful" behaviour, check whether
  SC has it.** If it does not, the game's way is the answer. Ask when
  unsure.

## Game-file safety (the second top rule)

The app edits the game's **live** data (`actionmaps.xml`, the profiles
folder), whose structure and processing can change with any game patch.
**BACKUP, BACKUP, BACKUPS**: the user must be able to undo any change at
any time. Concretely:

- **A verified backup before every write** to a live game file; restore is
  byte-exact. Never write without one — except when the user switched
  auto-backups off in Settings, their explicit choice made against a
  warning dialog.
- **Watertight sanitizing** of every user input that reaches a file, a path
  or an XML attribute (names, tokens, ids, paths: no traversal, nothing
  that needs escaping).
- **Rock-solid XML**: parsing tolerates whatever SC or the user may produce
  (comments, CDATA, CRLF, tabs, BOM, self-closing elements, entities,
  single quotes) and never panics; a textual rewrite is re-parsed before it
  touches the disk; writes are atomic (temp file + rename).
- **Tests and safeguards first**: the highest test coverage goes to
  game-file handling (`rebind.rs`, `resort.rs`, `apply.rs`, `backups.rs`,
  `binding_profiles.rs`, `diff.rs`, `scdata.rs` parsing, every writing
  command); every edge case above has a test, added with the change.

## Conventions

- **English only** — all code, comments, commit messages and GUI text.
- **GUI text speaks the player's language**: no file names (`actionmaps.xml`,
  `Game.log`, …) or other internals in labels, titles, toasts and dialogs.
  Only where an element is specifically about that file (the `global.ini`
  override, a missing-/broken-file error detail). Say "game", never "SC"
  ("Star Citizen" is fine where it reads better). Terse: one-word states,
  explanatory sentences only where a dialog guides an action.
- **Repo content policy**: no SC game data in the repo — the app extracts what
  it needs from the user's install at runtime. The StarBreaker sidecar
  binaries (MIT) are not committed either: `scripts/fetch-starbreaker.sh`
  (or `.ps1`) downloads the pinned release into `src-tauri/binaries/`
  (gitignored) and verifies SHA256; keep version + hashes in both scripts in
  sync. Without them every `cargo build` fails in the Tauri build script.

## Stack

- **Shell/backend**: Tauri v2 + Rust (`src-tauri/`).
- **Frontend**: Vue 3 + TypeScript + Vite (`src/`).
- **Input**: SDL2 raw joystick API (`sdl2` crate) for buttons/axes/hats, its
  GameController API for gamepads, the webview for keyboard keys; `hidapi`
  for the HID product string (SC's device name) and the axis usages.
- **XML/INI**: `quick-xml` + hand-rolled parsing.

## Frontend (`src/`)

`App.vue` is the orchestrator (all state, invokes, listeners) and composes
presentational components:

- `TopBar` — modes **Monitor / Bindings / Devices**, environment chip +
  dropdown, version chip, Refresh, gear. It is also the title bar (see the
  undecorated-window gotcha).
- `StartupTile` — covers every mode while the first game-data load after
  start runs, whatever its outcome.
- Monitor: `DeviceTile`, `StatusPanel`, `ImageStage` (+ `Splitter`),
  `LastInputCard`, `BindingsDeck` (flat rows or one bucket per input, same head
  as the Bindings List).
- `BindingsView` — the whole Bindings mode: Game Bindings (the live file as
  the one item "Current"), binding profiles (Save Profile, Import, Export),
  backups (Create Backup), and on the right either the Bindings List
  (Current: the game's keybinding screen as a table, one toggleable column
  per device the file names, categories collapsible, "Set binding" button
  / double-click = rebind dialog editing one input at a time, pending
  rebinds kept until Save / Discard in the action tile above it, which
  also holds Save Profile / Create Backup) or Compare (a profile or backup
  picked on the left, always against Current, every token as a row, the
  diff chips filter; its action tile: Open Folder | Apply (dialog picking
  the devices to take over) / Delete). Left column and the profiles panel
  are resizable.
- `ImageMapEditor` — the Devices mode (Konva via `vue-konva`, three columns:
  devices + image-maps, canvas, live input + shapes); also hosts Device Info
  (Device List and Device Events tiles, each with its own Save).
  `DeviceImage.vue` is the plain-SVG viewer.
- `SettingsDialog` (own-styled checkboxes, WebKitGTK would paint GTK's),
  `ConfirmDialog` (title, optional subtitle, required icon, buttons —
  each may be `disabled` or parked `side: "left"` — optional body slot,
  `captureKeys` keeps the keyboard capture on; behind every
  unsaved-changes / delete / game-file-write question, the Fix via config /
  Fix via console dialogs and the rebind dialog), `Toasts` (`ok` / `error`
  / `hint` — the accent tells them apart: green done, red broken, blue
  guidance; a hint is logged as info;
  every error toast goes to the app log), `Icon` (inline
  stroke SVGs by name — never emoji), `WindowEdges`.
- Shared modules: `imagemap.ts` (image-map types/helpers incl. token <->
  input key), `keyboard.ts` (webview keyboard capture, `KeyboardEvent.code`
  -> SC key name, feeds the same handler as `joy-input`; also drives mouse
  capture, armed only while `recording` is on), `devices.ts`
  (`deviceName` / `deviceKey` / `deviceIcon`; remembered GUI state is keyed
  by hardware id, never by SDL's index), `types.ts` (all device/input/
  binding types), `logging.ts` (console forwarding, see Logging),
  `persist.ts` (`persistedRef`: a ref mirrored into localStorage — every
  filter chip choice goes through it).
- **Tables** (bindings deck, Compare, Bindings List): `tableColumns.ts`
  (`useTableColumns`: sort state, widths, grid template, localStorage
  persistence; the column list may be reactive — the Bindings List's device
  columns come from the file) + `ColumnHead.vue` (sortable headers with an
  icon per column, resize grips). The last column is the `1fr` filler, the
  others carry px defaults, every cell truncates with an ellipsis.
- Design tokens in `src/styles/tokens.css` (the only place colours are
  defined), fonts bundled under `src/assets/fonts/` (OFL). The name is
  two-tone: BIND in `--text`, SIGHT in `--accent`.

## Backend modules (`src-tauri/src/`)

- `input.rs` — one background thread owns the single SDL context, keeps
  devices open, emits `joy-input`, maintains the shared device list, emits
  `devices-changed` on hot-plug (payload `DevicesChanged`: `added` /
  `removed` by SDL instance id, both empty for the startup enumeration and
  SDL's initial arrival events — the frontend toasts only real hot-plugs and
  announces a clash appearing / going away only after those or a new
  `Game.log`). `DeviceInfo` carries `kind` (`joystick` /
  `gamepad` / `keyboard`), `hardware_id` (image-map key: SC Product GUID,
  `gamepad`, or `keyboard`), `sc_name` (hidapi) + `sdl_name` (debug), `axes`
  / `axes_error`. A gamepad is what SDL's GameController API recognises; its
  raw `Joy*` events are dropped in favour of `padbutton` / `padaxis` with
  SC's names (`a`, `shoulderl`, `thumblx`, …). Only the first pad (SDL
  index order) holds the slot `gp1`, further pads get `gamepad_slot: None`.
  The keyboard is one synthetic entry appended last (`sdl_guid` `keyboard`).
  **Axis rules, all here so every consumer sees one stream**: a fixed
  resting zone per axis (joysticks 4000, pad sticks 8000, pad triggers
  4000; the game's `<deviceoptions>` deadzones are a 1.0 goal), a delta
  filter plus at most one event per axis per 20 ms (the centre always
  passes), and of a stick's two axes (pad sticks, joystick `x`/`y`) only
  the further deflected one is forwarded. **Derived pad buttons**
  (`triggerl_btn`, `thumbl_left` …, `triggerl_r_btn` = both triggers) press
  once the axis is past 30000 — a trigger at once, like a shoulder button,
  a stick direction only after 500 ms there (the stick is an axis first) — and
  release below 24000; the loop ticks every 25 ms for the stick hold. **A trigger is only ever its derived
  button**: SC labels the axis token like the button and ships no default
  on it, so the trigger axis is never emitted.
- `guid.rs` — SDL joystick GUID -> SC `options/@Product` GUID (byte-swap
  vendor/product); `sdl_guid_vendor_product`.
- `hid.rs` — HID report descriptor -> SC axis name per SDL axis index
  (`x y z rotx roty rotz slider1 slider2`).
- `scdata.rs` — parse `defaultProfile.xml` (action master list with the
  `joystick=` / `keyboard=` / `gamepad=` defaults, attribute or child-element
  form, child wins), `global.ini` (labels), `keybinding_localization.xml`
  (input token labels `jsN_` / `kb1_` / `gp1_`; the mouse device lands
  under `kb1_` too, see `Action.mouse_default`), and the
  user's `actionmaps.xml` (rebinds + `<options>` device map). `DeviceKind`
  and `parse_rebind` (prefix -> kind; kb/gp tokens keep their `+` modifiers).
- `scinstall.rs` — the configured install. `validate_install` first
  (`REQUIRED_FILES` = `Data.p4k`, `build_manifest.id`, the live
  `actionmaps.xml`; anything missing aborts the whole load with one message
  and `ScState::invalid_install` keeps `reload_bindings` from reading
  anything). Version from `build_manifest.id` (`ScVersion`, label
  `<branch minus sc-alpha->.<P4 changelist>`, e.g. `4.10.0-hotfix.12572603`).
  Game data (`ScData`: action master list + token labels): StarBreaker
  sidecar `p4k extract --regex` (~1 s), `scdata::parse_*` (unlabeled actions
  dropped), cached as JSON under `<app_cache_dir>/<label>/`. `scdata.json`
  carries a `format` stamp (`CACHE_FORMAT`): bump it whenever the cached
  shape changes meaning, the cache is then re-extracted once. Loaded in a
  background thread at start and on environment change
  (`lib.rs::spawn_sc_load`; an active `global.ini` override bypasses the
  cache): steps via `scdata-progress` (`LOAD_STEPS` = 4), result via
  `scdata-changed`.
- `bindings.rs` — `BindingIndex` (token -> bound actions, all device kinds,
  defaults per kind with a per-kind "touched" rule), `button_token` /
  `hat_token` (+1 offset), `instance_for_guid` (the saved `<options>` slot,
  used by the clash analysis and the Bindings List only), `resolve_bindings`,
  `analyze_clash` (saved `<options>` vs. SC's joystick order, a
  `DeviceOrder`; an empty order is an order, every saved slot then
  "missing"; `ClashReport::logged_order` = the game's logged order when it
  ranks differently from the live one), `plan_resort` / `resort_commands`.
  A joystick SDL lists that the order lacks is `unseen`: the GUI drops it
  from Monitor, deck and image-map list without a word — only the Device
  List ("seen by game") and the clash line in the app log name it.
- `order.rs` — `DeviceOrder` (joysticks in SC's order + timestamp), the one
  type every order source yields: `instance_for_guid` is what the Monitor
  resolves against (`resolve_input`; without an order no joystick input
  resolves, keyboard and pad still do), `same_ranking` the log-vs-live
  check. `order::live()` is the platform switch: DirectInput on Windows,
  the Wine replication (`wineorder.rs`) on Linux; `Game.log` is never the
  source, only the second opinion.
- `dinput.rs` — Windows: DirectInput 8 `EnumDevices(DI8DEVCLASS_GAMECTRL,
  DIEDFL_ATTACHEDONLY)` via `windows-sys` with hand-rolled COM vtables
  (windows-sys ships none). The rank is `jsN`, `guidProduct` is byte for
  byte SC's `options/@Product` GUID; XInput devices (path carrying `ig_`,
  lower-case) are skipped. `DIPROP_GUIDANDPATH` is the pointer value 12,
  not a GUID in memory. `to_order` is the pure part with tests.
- `wineorder.rs` — Linux: Wine's DirectInput enumeration replicated. Wine
  answers `EnumDevices` from the registry, and wineserver keeps subkeys
  sorted, so the order is the alphabetical order of the HID interface keys
  `HID#VID_xxxx&PID_yyyy[&MI_nn]#<version>&<serial>&0&<index>&<gp>`: by
  (vendor, product), identical devices by winebus's `index` (creation
  order = udev enumeration, sorted by sysfs path). Visibility as winebus
  decides it: HID usage joystick / gamepad only; one backend per device —
  hidraw for `hidraw_preferred` vendors (all VKB / VPC, some TM / Fanatec /
  Simucube, DualShock / DualSense), which must open read-write, SDL for the
  rest; an SDL device that SDL maps as a controller (unless wheel / flight
  stick) or has exactly 6 axes and 14+ buttons is a gamepad (`&IG_00`, SC's
  `xinput`, no slot). `rank` is the pure part with tests, `enumerate` reads
  hidapi + sysfs. Not replicated: registry overrides, the evdev backend,
  multi-collection devices (`&ColNN`).
- `gamelog.rs` — parse `Connected joystickN: <Product {GUID}>` into a
  `DeviceOrder` (gamepad lines are not read: no slot). The second opinion
  on both platforms, never the source — the game keeps the order it
  started with, so a live order that ranks differently means "restart the
  game" (Status panel tile).
- `xmltext.rs` — helpers for the textual editors: `mask_markup` (a copy of
  the text with comments, CDATA and processing instructions blanked, same
  byte offsets — every search runs on the mask, every edit on the
  original), `tag_end` (quote-aware), `find_attr` / `attr` / `set_attr`
  (double or single quotes, whitespace around `=`).
- `gamefile.rs` — the one road every live-file write takes:
  `write_atomic` (temp file next to the target, `sync_all`, rename, read-back
  compare) and `replace_live_file` (new text must parse, verified backup
  first unless the user switched auto-backups off, then the atomic
  write). Used by
  `save_rebinds`, `apply_resort`, `apply.rs`, `backups.rs` (copy + restore),
  `binding_profiles.rs` (save / export) and `write_text_file`.
- `resort.rs` — textual `actionmaps.xml` rewrite applying a resort (joystick
  `<options>` instances + `jsN_` prefixes in `input="..."`), the out-of-game
  counterpart of `pp_resortdevices`. Re-parses its output and checks it
  against the intent (`verify_applied`: every rebind in place with mapped
  tokens, devices on their new slots, nothing else changed).
- `rebind.rs` — textual `actionmaps.xml` rewrite writing rebinds (one
  binding per action and device kind, like SC: every `<rebind>` of that kind
  under the action is replaced, the first one keeping its other attributes
  such as `activationMode` unless the change carries its own `attrs` list;
  missing `<action>` / `<actionmap>` elements are created in SC's layout),
  the out-of-game counterpart of the keybinding screen. Every change passes
  `validate` (SC identifiers only, the input must be an SC token of the
  change's kind or a blank of it, attributes whitelisted) and the result
  `verify_applied` (the parsed before/after differ exactly by the changes).
- `config.rs` — JSON in the app config dir: the SC environments
  (`ENVIRONMENTS` = LIVE / HOTFIX / PTU / EPTU, each a base path + optional
  `global.ini` override; Windows default paths), the active one
  (`Config::base_path()` / `global_ini_override()`), the image-map choice
  per device, the auto-backup and debug-logging switches. An older file's
  `ignored_devices` (the Exclude feature of 0.10 – 0.12) is ignored.
  `load` fills missing environments with defaults; no migration of older
  shapes.
- `imagemap.rs` — one folder per image-map (`imagemap.json` + image) under
  `<app_data_dir>/imagemaps/`, bundled ones under `resources/imagemaps/`
  (read-only; clone into the user root with a fresh id, import assigns a
  fresh id too). Zip export/import, image add/remove/read (data URL),
  validation. Pure logic takes `&Path` roots; the `#[tauri::command]`
  wrappers only resolve dirs.
- `binding_profiles.rs` — SC's exported keybinding layouts
  (`controls/mappings/*.xml`, same content as `actionmaps.xml`, different
  root): list / import / export / delete, and "Save Profile" writes the
  live file in that layout (`to_profile_xml`, textual, in-game import
  unverified).
- `apply.rs` — applies a profile or backup to the live file per device
  (`plan_apply`: the source's rebinds for the chosen `kb1` / `gp1` / `jsN`
  are written, live rebinds the source lacks are removed via an empty
  `RebindChange::input`, the source's rebind attributes carried along,
  everything else stays), backup "before apply"; a backup with every device
  chosen is restored byte for byte.
- `names.rs` — the name rule (see Image-map data model).
- `backups.rs` — backups of the live `actionmaps.xml`, one folder per backup
  (`meta.json` with reason + game version, `actionmaps.xml`) under
  `<app_data_dir>/backups/<id>/`, id = `YYYYMMDD-HHMMSS` (UTC) with a `-2`,
  `-3`, … suffix on collision (folders are created, not checked, so
  concurrent backups cannot collide). Taken manually and, always, before
  every write to the live file (rebind, apply, order fix, restore); a
  backup counts only once its copy compares byte for byte with the source,
  a restore refuses a backup that no longer parses. `Config::auto_backup`
  (default on) gates the backups before a write and before a restore —
  the one exception to the safety rule, the user's explicit choice: the
  Settings checkbox asks with a warning before it goes off.
- `diff.rs` — compares the joystick bindings of two sources (live file,
  binding profile, or backup) by SC token: the `(actionmap, action)` set per
  token in A vs B (label-only differences are not changes); added / removed /
  changed rows, sorted `jsN` then numeric-aware by input.
- `logwatch.rs` — the `Game.log` watch (pure state machine; the thread is
  `lib.rs::spawn_game_log_watch`, **the only reader of the log**, always
  outside the `AppData` lock): a metadata poll every 2 s; the first poll and
  an environment change `Adopt` the file (one full read); a *new* file
  (shorter, other creation time, appeared / vanished — NTFS name tunneling
  keeps the creation time on a quick recreate) is read incrementally
  (`Tail`: only the bytes appended since the last read) until the order is
  in or 90 s pass; a changed outcome replaces the log snapshot (and the
  order source, where the log is it) and emits `gamelog-changed`
  (`{started}`: true for a new file — the frontend toasts and announces a
  clash flip only then). Writes to a running log are ignored on purpose.
- `kblayout.rs` — `keyboard_layout` command: xkb code (`de`, `us`, …) via
  `localectl` / `vconsole.conf` on Linux, `GetKeyboardLayoutNameW` on Windows.
- `lib.rs` — Tauri commands, state wiring (`AppData`: config, game data +
  load status, the bindings file + its load error, binding index, the
  joystick order (`device_order`, re-taken from the live source on every
  clash report) + the Game.log snapshot, the last logged clash summary.
  **Nothing slow under the `AppData` lock**: `resolve_input` runs per input
  event on the main thread and needs it, so `order::live()` (DirectInput)
  is taken *before* locking (`get_clash_report`, `apply_resort`) and the
  log is read only by the watch thread without the lock; the write
  commands hold it for their backup + write + reload on purpose (user
  actions, consistency);
  `get_load_status` hands the last
  load outcome to a frontend that mounts after the first load already
  finished), `system_info` (app version, OS, toolkit versions for the
  Device Info dumps), the window-close guard (`CloseGuard`: every
  `CloseRequested` is prevented and sent to the frontend as our own
  `close-requested` event, never Tauri's — with a JS listener on
  `tauri://close-requested` Tauri waits for the webview forever once it is
  dead; the frontend acks via `ack_close`, settles unsaved changes and
  calls `destroy`; no ack within 2 s = dead webview, the window is
  destroyed here and the app exits if even that leaves it running), thread
  spawns, the Wayland DMABUF workaround, logging
  setup.

## Logging

`log` crate everywhere (never `println!` / `eprintln!`); `tauri-plugin-log`
writes to stdout and `<app_log_dir>/bindsight.log` (2 MB, 3 files; Linux
`~/.local/share/com.w00zla.bindsight/logs/`). Our crate logs down to DEBUG
only while Settings "Enable debug logging" is on (`Config::debug_logging`,
default off, applied via `log::set_max_level`), dependencies from WARN. The
webview console, uncaught errors and unhandled rejections are forwarded by
`src/logging.ts` via the plugin's `log` command with the plain `webview`
target (the JS package would tag a source location the level filter cannot
match); the plugin's `log` command bypasses the level cap, so `logging.ts`
drops webview debug records itself (`setDebugLogging`). `main.ts` sets
`app.config.errorHandler` so a component error names its component. Every
error toast (`notify`) and every failed startup / refresh step logs; a
silent `catch` is a bug.

Severities: ERROR = a feature is broken (config not saved, SC data failed,
input thread died, panic); WARN = degraded but running (bindings not loaded,
Game.log missing, unreadable cache); INFO = state changes and facts (startup
environment, SC version, extraction, bindings / Game.log contents, user
actions); DEBUG = detail (command lines, load steps). **Device runtime detail
never goes to the app log** — no enumeration lines, no axis derivation, no
input events, only real failures (hidapi init, joystick open, thread death).
All of it lives in the Devices mode's Device Info view instead.

## Image-map data model (`imagemap.json`, format 4)

- Keyed by `hardware_id`: SC Product GUID (vendor/product, platform-stable)
  for joysticks, the literal `gamepad` for gamepads (SC treats every pad as
  the same XInput device), `keyboard` for the keyboard. Several image-maps
  per id are normal (told apart by `name`).
- **Exactly one device image per image-map** (`image: {file, label}`,
  mandatory — an image-map is created around its image file). Older formats
  (1: `images[]`, 2, 3: `areas` with a nested `shape`) are not read.
- `shapes[]` map an input key to a `geometry` plus optional `stroke` / `fill`
  (`#rrggbb` or `#rrggbbaa`; unset = the `--shape-stroke` / `--shape-fill`
  tokens). Joysticks use **SDL-level** keys (`button:N`, `hat:N:<dir>`,
  `axis:N`, no axis sign); keyboard and gamepad use SC's own names
  (`key:lshift`, `key:oem_102`, `pad:a`, `pad:thumblx`, `pad:triggerl_btn`).
  Geometry kinds: `rect` (+ `radius`, a fraction of the shorter side),
  `ellipse`, `polygon`, `symbol` (`arrow`, `arrow2`, `rotate`: a 100x100
  path stretched into a `w` x `h` box, rotatable), `arc` (outer `r`, `inner`
  as a fraction of it, `angle` of sweep from `rotation`), `wedge` (`r`,
  `angle`, `rotation`) and `image` (its own file in the map folder, no
  colours, box like a symbol). Several shapes per input are fine.
- **Names** (image-map now, device names later): letters, digits, space,
  `_`, `-` and brackets `()[]{}` only, trimmed, at most 64 characters —
  nothing that needs escaping in a file name or URL. `imagemap::sanitize_name` / `src/names.ts` hold the
  rule; the backend validates, the editor strips as you type.
- **A shape is drawn only while its input is active** (Monitor); the image
  itself is the resting look — anything permanent belongs in the image.
- Coordinates are normalized 0..1 to the image's natural size (radii to the
  width), rotation in degrees around the shape's center. The model is ours,
  never Konva's JSON. `save` prunes image files in the folder that nothing
  references any more (an image shape deleted, the device image swapped).
- Token -> input key undoes the +1 offset (`js2_button5` -> `button:4`) and
  maps axes through `DeviceInfo::axes`; `kb1_lalt+x` -> `key:x`, `gp1_a` ->
  `pad:a` (a combo pins the part after the last `+`). The editor shows keys
  by their native name only (`button 3`, `hat 0 up`, `axis 2 (rotz)`): an
  image-map does not know which `jsN` its device is.
- **Bundled image-maps** (`src-tauri/resources/imagemaps/`): Keyboard US,
  Keyboard DE, Xbox controller, PlayStation controller, Keyboard US (TKL),
  Keyboard DE (TKL) — generated, never hand-edited (a generator outside the
  repo holds the geometry and writes `image.png` + `imagemap.json`; fixed
  ids `4b7a2c1e-…-000000000001` to `…0006`).
- **Default map per device** (`App.vue`, `chosenMapId`): the user's choice,
  else the first fitting hard-coded `BUNDLED_RULES` entry (not part of the
  model; gamepads by case-insensitive `*`/`?` wildcards on the controller
  name, keyboards by the OS layout from `kblayout.rs`), else `DEFAULT_MAPS`
  (US keyboard, Xbox pad), else the first map. Joysticks match by hardware
  id only.

## Commands

```sh
pnpm tauri dev                                  # run the app (needs a display)
pnpm build                                      # frontend typecheck + build
cd src-tauri && cargo test --lib                # backend tests, no hardware
cargo run --example enum_joysticks              # list devices + GUIDs
cargo run --example log_joystick_events         # live event log
cargo run --example hid_names                   # HID product strings
cargo run --example hid_axes [-- --hex]         # HID axis usages + SC axes
cargo run --example dinput_order                # SC's joystick order (Windows)
cargo run --example wine_order                  # SC's joystick order under Wine (Linux)
cargo run --example pad_events                  # pad mapping + controller-level events
scripts/fetch-starbreaker.sh                    # sidecar binaries (once)
```

The game data cache lives per version under `~/.cache/com.w00zla.bindsight/
<label>/` on Linux; delete it to force a re-extract.

## Prerequisites

- **All platforms**: the StarBreaker sidecar binaries (see Repo content
  policy; `fetch-starbreaker.sh` needs `curl`, `tar`, `unzip`).
- **Fedora/Nobara**: `rustup`, `pnpm`, `webkit2gtk4.1-devel openssl-devel
  curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel
  SDL2-devel` plus the `c-development` group.
- **Windows**: `rustup` with the MSVC toolchain (`stable-x86_64-pc-windows-
  msvc`, not GNU); Visual Studio Build Tools 2022 with *Desktop development
  with C++* (linker plus `hid` / `setupapi` for `hidapi`); CMake on `PATH`
  (SDL2 is built from source and static-linked, no `SDL2.dll` shipped —
  `winget install Kitware.CMake`, or append the C++ workload's copy under
  `<VS>\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin`); `pnpm`;
  WebView2 runtime.

## Gotchas (learned the hard way)

- **SC label resolution** (`global.ini` is the only source): match keys
  case-insensitively (`@ui_CIboost` vs `ui_CIBoost`) and alias the `,P` (PC
  platform) suffix to the bare key. ~350 actions are genuinely unlabeled
  (SC-internal, hidden in SC too) and are dropped.
- **Input token labels are per-instance**: `js1_button1` = "Button 1",
  `js2_button1` = "Button 1 (Input 2)". Axis/hat keys differ from the bind
  token (`x`->`xAxis`, `rotz`->`zRotation`, `hat1_up`->`hat1Up`), so use
  `keybinding_localization.xml` as the token->key map, not string surgery.
- **actionmaps.xml lives at** `<base>/user/client/0/Profiles/default/`, NOT
  `controls/mappings/` (exported layouts only). Base path = the parent of
  `Data.p4k` (e.g. `.../StarCitizen/LIVE`).
- **Device identity is GUID, never name**: SDL's name (evdev) differs from
  SC's (HID product string).
- **SC's joystick order is DirectInput's `EnumDevices` order** (verified
  2026-09-12 against `Game.log` and a saved `actionmaps.xml`: rank and
  `guidProduct` identical). **SDL order is not SC order**: SDL's Windows
  DirectInput backend prepends every device to its list (the start
  enumeration comes out reversed, a hot-plug lands on index 0); on Linux the
  two are unrelated. `jsN` therefore comes from `order.rs` only — DirectInput
  live on Windows, Wine's registry key order replicated on Linux
  (`wineorder.rs`, verified 2026-09-13 against the prefix's `system.reg`,
  sysfs and `Game.log`) — never from SDL's enumeration, and
  never from the saved `<options>` slot either (that is what the file says,
  the order is what the game does). The Monitor, the deck and the Bindings
  List's column names follow it; without an order the joysticks show "no
  joystick order" and resolve nothing. A device plugged in or out shifts
  every slot behind it — inherent to SC, that is what Apply / the order fix
  are for.
- **Joystick modifiers do not exist in SC** (no `modifier+jsN_` token in
  the data or a real file); keyboard and gamepad combos do (`kb1_lalt+x`,
  `gp1_shoulderl+thumbl_left`, `gp1_shoulderl+thumblx`), always with a real
  button as the modifier — derived pad buttons are never modifiers.
- **+1 button offset**: SC `js_button1` == SDL button 0 (verified under Wine).
- **Windows RawInput pads need `SDL_JOYSTICK_THREAD=1`** (`input::init_sdl`):
  SDL's RawInput driver (e.g. an Xbox pad over Bluetooth, SDL GUID ending
  `72`) gets arrival/removal and input only as messages to SDL's hidden
  window, which nothing pumps without the video subsystem. DirectInput
  sticks were fine (`CM_Register_Notification` callback).
- **SC knows exactly one keyboard (`kb1`) and one gamepad (`gp1`)**: no
  `kb2_` / `gp2_`, no instance logic, no order clash. Key names are
  DirectInput scancode names (physical, layout independent): on a German
  keyboard the cap labelled `Y` is `kb1_z`; `KeyboardEvent.code` is physical
  too, hence the static table in `keyboard.ts`. The mouse is part of the
  keyboard device (`kb1_mouse1`, `kb1_mwheel_up`), captured only while a
  Record button is armed (rebind dialog, image-map editor); `maxis_*` is
  labeled but not recordable.
- **Keyboard capture needs the BindSight window focused** (webview keydown;
  SDL2 delivers key events only to its own window). It runs in every mode
  and `preventDefault`s every mapped key (deliberate: a rebind flow must own
  the keyboard); only text fields and open dialogs (`[role="dialog"]`) are
  skipped — except a dialog carrying `data-capture-keys` (`ConfirmDialog`
  `captureKeys`, the rebind dialog). PrintScreen / Meta never arrive, and
  Escape is deliberately not an input: it cancels a recording (and the
  rebind dialog), like Meta it is dimmed on the keyboard maps.
- **Axes**: SC names axes by HID usage (X->`x` … Rz->`rotz`, Slider/Dial->
  `slider1`/`slider2`); SDL numbers them in canonical usage order (Linux:
  evdev ABS code order, Windows: DirectInput offset order), NOT report
  order. Verified on a VKB EVO whose report order is X Y Rz Z Rx Ry: SDL 5 =
  twist = `rotz`, SDL 2 = `z`. `hid.rs` derives it and cross-checks the
  count against SDL; anything it cannot place is an `axes_error`, never a
  guess.
- **The window is undecorated** (`decorations: false`): the top bar is the
  title bar (`data-tauri-drag-region`, double-click toggles maximize) with
  its own minimize / maximize / close buttons; `WindowEdges.vue` draws the
  eight invisible resize strips (`startResizeDragging`) because an
  undecorated window has no edge resize on Linux. Window permissions live in
  `capabilities/default.json`. The app runs native Wayland on Linux: a dev
  build then shows no taskbar icon (Plasma looks it up by app id +
  `.desktop` file, which only an installed bundle has).
- **Wayland**: `run()` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` on Linux
  (fixes WebKitGTK "Error 71") unless the user overrode it.
- **Dark mode + native controls**: `:root { color-scheme: dark }` in the
  dark media block, or WebKitGTK paints `<select>` popups white with
  inherited white text.
- **Konva rect rotation**: rect nodes sit at their center with
  `offsetX/Y = half size`, so `rotation` turns around the center and the
  stored `x/y` stay the unrotated top-left. On `transformend` read
  `width*scaleX`, reset scale to 1 imperatively (vue-konva diffs configs and
  would not re-apply an unchanged `scaleX: 1`).
- **CMake 4 vs. vendored SDL2**: `sdl2-sys` ships an SDL2 whose
  `CMakeLists.txt` says `cmake_minimum_required(VERSION 3.0)`, which CMake
  4.x refuses (VS 2026 ships 4.3). `src-tauri/.cargo/config.toml` sets
  `CMAKE_POLICY_VERSION_MINIMUM=3.5`; drop it once `sdl2-sys` vendors an SDL2
  requiring >= 3.5.
