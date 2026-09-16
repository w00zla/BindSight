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
  it needs from the user's install at runtime. The `Data.p4k` reader and the
  CryXmlB decoder (`p4k.rs`, `cryxml.rs`) are ports from StarBreaker (MIT,
  by diogotr7): keep the credit header in both files and the notice in
  `THIRD-PARTY-LICENSES.md`.

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
  start runs, whatever its outcome, and until the startup image-map load
  is done (`endStartup` waits for `mapsReady`, so the maps never pop in
  after the stage is already showing).
- Monitor: `DeviceTile`, `StatusPanel`, `ImageStage` (+ `Splitter`),
  `LastInputCard`, `BindingsDeck` (flat rows or one bucket per input, same
  head as the Bindings List).
- `BindingsView` — the whole Bindings mode. Left: Game Bindings (the live
  file as the one item "Current"), binding profiles (Save Profile, Import,
  Export), backups (Create Backup). Right: the **Bindings List** for Current
  (the game's keybinding screen as a table, one toggleable column per device
  the file names, categories collapsible, "Set binding" / double-click =
  rebind dialog editing one input at a time, pending rebinds kept until
  Save / Discard in the action tile above it, which also holds Reorder —
  swap two joystick slots the game ranks now, one swap at a time, either
  Apply to config = `apply_reorder` or the `pp_resortdevices` line in
  `ConsoleCommandDialog` — plus Save Profile / Create Backup) or **Compare**
  (a profile or backup picked on the left, always against Current, every
  token as a row, the diff chips filter; action tile: Open Folder | Apply
  (dialog picking the devices to take over) / Delete). Left column and the
  profiles panel are resizable.
- `ImageMapEditor` — the Devices mode (Konva via `vue-konva`, three columns:
  devices + image-maps, canvas, live input + shapes); also hosts Device Info
  (Device List and Device Events tiles, each with its own Save).
  `DeviceImage.vue` is the plain-SVG viewer.
- Dialogs and widgets: `SettingsDialog` (own-styled checkboxes, WebKitGTK
  would paint GTK's), `ConfirmDialog` (title, optional subtitle, required
  icon, buttons — each may be `disabled` or parked `side: "left"` —
  optional body slot, `captureKeys` keeps the keyboard capture on; behind
  every unsaved-changes / delete / game-file-write question, the Fix via
  config / Fix via console dialogs and the rebind dialog),
  `ConsoleCommandDialog` (how to open the game console, the command line, a
  Copy button; used by the order fix and Reorder), `Dropdown` (a select in
  the app's look — WebKitGTK paints a native popup no CSS reaches),
  `ScrollRail` (one non-wrapping row that scrolls sideways, wheel or end
  arrows, scrollbar hidden), `Toasts` (`ok` / `error` / `hint`: green done,
  red broken, blue guidance; a hint is logged as info, every error toast
  goes to the app log), `Icon` (inline stroke SVGs by name — never emoji),
  `WindowEdges`, `AppFooter` (credits, a `mark` slot before the logo; the
  logo and the link-styled version button open the **App Update dialog**),
  `VersionDialog` (a `ConfirmDialog`: Current chip = the running version,
  Update chip = the found version / "Checking…" / a dash, then download
  progress, an error line, the dev toggle; without an updater only the
  Current row; the updater's buttons after Close).
- **Updater** (`update.ts`, `UpdateMark.vue`; backend `update.rs`):
  loaded by `App.vue` via dynamic import only when `system_info.updater`
  is true (see Releases), or in a dev build as the simulated one
  (`createUpdater(channel, true)`: a toggle in the dialog fakes an
  available update and a download, so the GUI parts can be looked at; the
  simulation code sits behind `import.meta.env.DEV` and is not in a
  release). `createUpdater(channel)` returns reactive state (`idle` /
  `checking` / `current` / `available` / `downloading` / `installing` /
  `error`), `check()` (`check_update` with the Settings channel),
  `install()` (`install_update`, progress via `update-progress` events,
  the backend restarts the app), the footer `mark` and the dialog
  `buttons()` (Check Update until one is found, then Install). The startup
  check runs last in `onMounted` unless Settings switched it off
  (`Config::update_check`); a found update opens the dialog and puts the
  mark in the footer — **nothing is downloaded or installed without the
  user's click**. A failed startup check only logs. The dialog shows the
  channel as a `ConfirmDialog` `badge` next to the title unless it is
  stable.
- Shared modules: `imagemap.ts` (image-map types/helpers incl. token <->
  input key, `arcArrowPath`), `keyboard.ts` (webview keyboard capture,
  `KeyboardEvent.code` -> SC key name, feeds the same handler as
  `joy-input`; also drives mouse capture, armed only while `recording` is
  on), `devices.ts` (`deviceName` / `deviceKey` / `deviceIcon`,
  `recordEdge`; remembered GUI state is keyed by hardware id, never by
  SDL's index), `names.ts` (the name rule, see Image-map data model),
  `colour.ts` (`#rrggbb` / `#rrggbbaa` helpers for the pickers),
  `types.ts` (all device/input/binding types), `logging.ts` (console
  forwarding, see Logging), `persist.ts` (`persistedRef`: a ref mirrored
  into localStorage — layout and preferences only, never a filter: a
  filter kept across a restart can match nothing any more, e.g. a device
  filter on a joystick that is gone, and the list shows nothing without a
  hint why).
- **Tables** (bindings deck, Compare, Bindings List): `tableColumns.ts`
  (`useTableColumns`: sort state, widths, grid template, localStorage
  persistence; the column list may be reactive — the Bindings List's device
  columns come from the file) + `ColumnHead.vue` (sortable headers with an
  icon per column, resize grips). The last column is the `1fr` filler, the
  others carry px defaults, every cell truncates with an ellipsis.
- Styles: `src/styles/tokens.css` (the only place colours are defined),
  `base.css`, `fonts.css` (fonts bundled under `src/assets/fonts/`, OFL).
  The name is two-tone: BIND in `--text`, SIGHT in `--accent`.

## Backend modules (`src-tauri/src/`)

### Input and devices

- `input.rs` — one background thread owns the single SDL context, keeps
  devices open, emits `joy-input`, maintains the shared device list, emits
  `devices-changed` on hot-plug (payload `DevicesChanged`: `added` /
  `removed` by SDL instance id, both empty for the startup enumeration and
  SDL's initial arrival events — the frontend toasts only real hot-plugs and
  announces a clash appearing / going away only after those or a new
  `Game.log`). `DeviceInfo` carries `kind` (`joystick` / `gamepad` /
  `keyboard`), `hardware_id` (image-map key: SC Product GUID, `gamepad`, or
  `keyboard`), `sc_name` (hidapi) + `sdl_name` (debug), `axes` /
  `axes_error`. A gamepad is what the game takes as its XInput device:
  on Windows what SDL's GameController API recognises, on Linux what
  winebus's rule says (`wineorder::sdl_is_gamepad`: an SDL-fed device —
  not a `hidraw_preferred` one — that SDL maps as a controller unless it
  is a wheel / flight stick, or that has exactly 6 axes and 14+ buttons;
  a Keychron K2 HE keyboard is one). Its raw `Joy*` events are dropped in
  favour of `padbutton` / `padaxis` with SC's names (`a`, `shoulderl`,
  `thumblx`, …); a Linux gamepad without an SDL mapping (`wine_gamepad`)
  has its raw events translated first, the way Wine + xinput map a
  generic report (`translate_wine_pad`: buttons 0–9 = A B X Y LB RB Back
  Start LS RS, the rest dropped; axes 0–5 = left stick, LT, right stick,
  RT with the triggers rescaled to 0..32767; hat 0 = D-pad — derived
  from Wine 11.7's source, not verified in-game). The slot `gp1` goes to
  the first pad in SDL index order on Windows and to the first in Wine's
  key order (XInput user 0) on Linux; further pads get `gamepad_slot:
  None`.
  The keyboard is one synthetic entry appended last (`sdl_guid` `keyboard`).
  **Axis rules, all here so every consumer sees one stream**: a fixed
  resting zone per axis (joysticks 4000, pad sticks 8000, pad triggers
  4000; the game's `<deviceoptions>` deadzones are a 1.0 goal), a delta
  filter plus at most one event per axis per 20 ms (the centre always
  passes), and of a stick's two axes (pad sticks, joystick `x`/`y`) only
  the further deflected one is forwarded. **Derived pad buttons**
  (`triggerl_btn`, `thumbl_left` …, `triggerl_r_btn` = both triggers) press
  once the axis is past 30000 — a trigger at once, like a shoulder button,
  a stick direction only after 500 ms there (the stick is an axis first) —
  and release below 24000; the loop ticks every 25 ms for the stick hold.
  **A trigger is only ever its derived button**: SC labels the axis token
  like the button and ships no default on it, so the trigger axis is never
  emitted.
- `guid.rs` — SDL joystick GUID -> SC `options/@Product` GUID (byte-swap
  vendor/product); `sdl_guid_vendor_product`.
- `hid.rs` — HID report descriptor -> SC axis name per SDL axis index
  (`x y z rotx roty rotz slider1 slider2`).
- `kblayout.rs` — `keyboard_layout` command: xkb code (`de`, `us`, …) via
  `localectl` / `vconsole.conf` on Linux, `GetKeyboardLayoutNameW` on Windows.

### Joystick order (see the order gotcha for the facts)

- `order.rs` — `DeviceOrder` (joysticks in SC's order + timestamp), the one
  type every order source yields: `instance_for_guid` is what the Monitor
  resolves against (`resolve_input`; without an order no joystick input
  resolves, keyboard and pad still do), `same_ranking` the log-vs-live
  check. `order::live()` is the platform switch: `dinput.rs` on Windows,
  `wineorder.rs` on Linux; `Game.log` is never the source, only the second
  opinion.
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
  `DeviceOrder` (gamepad lines are not read: no slot). The game keeps the
  order it started with, so a live order that ranks differently means
  "restart the game" (Status panel tile).
- `logwatch.rs` — the `Game.log` watch (pure state machine; the thread is
  `lib.rs::spawn_game_log_watch`, **the only reader of the log**, always
  outside the `AppData` lock): a metadata poll every 2 s; the first poll and
  an environment change `Adopt` the file (one full read); a *new* file
  (shorter, other creation time, appeared / vanished — NTFS name tunneling
  keeps the creation time on a quick recreate) is read incrementally
  (`Tail`: only the bytes appended since the last read) for the whole 90 s
  window — the joystick lines land one by one, 200 ms apart, a read may
  fall between them, so the first line found is not the order yet; every
  changed outcome replaces the log snapshot and emits `gamelog-changed`
  (`{started}`: true for the first outcome of a new file — the frontend
  toasts and announces a clash flip only then). Writes to a running log are
  ignored on purpose.

### Game data and bindings

- `scinstall.rs` — the configured install. `validate_install` first
  (`REQUIRED_FILES` = `Data.p4k`, `build_manifest.id`, the live
  `actionmaps.xml`; anything missing aborts the whole load with one message
  and `ScState::invalid_install` keeps `reload_bindings` from reading
  anything). Version from `build_manifest.id` (`ScVersion`, label
  `<branch minus sc-alpha->.<P4 changelist>`, e.g. `4.10.0-hotfix.12572603`).
  Game data (`ScData`: action master list + token labels): the three files
  read straight out of `Data.p4k` (`p4k.rs` + `cryxml.rs`, `P4K_FILES`,
  ~150 ms), `scdata::parse_*` (unlabeled actions dropped), cached as JSON
  under `<app_cache_dir>/<label>/` (Windows: `<app_cache_dir>/cache/<label>/`,
  `lib.rs::sc_cache_root` — LocalAppData also holds the logs and the
  WebView2 profile). `scdata.json`
  carries a `format` stamp (`CACHE_FORMAT`): bump it whenever the cached
  shape changes meaning, the cache is then re-extracted once. Loaded in a
  background thread at start and on environment change
  (`lib.rs::spawn_sc_load`; an active `global.ini` override bypasses the
  cache): steps via `scdata-progress` (`LOAD_STEPS` = 4), result via
  `scdata-changed`.
- `p4k.rs` — reader for `Data.p4k`: Zip64 central directory (tail of the
  file only, ~1.4 M entries) with CIG's extra fields (`0x0001`, `0x5000`,
  `0x5002` = encryption flag, `0x5003`, in that order), CIG's local-header
  signature, AES-128-CBC with CIG's fixed key + zero padding, zstd (method
  100) / deflate / stored. `Archive::open` + `entry` (either separator,
  case-insensitive) + `read`; `Cursor` is the bounds-checked LE reader
  `cryxml.rs` shares. Ported from StarBreaker; the synthetic test archives
  cover every method incl. encrypted, `examples/p4k_extract.rs` diffs a
  real install against another extractor.
- `cryxml.rs` — CryXmlB (CryEngine binary XML, the in-archive form of
  `defaultProfile.xml` and `keybinding_localization.xml`) to XML text,
  byte-identical to StarBreaker's output. Validates every index and that
  the nodes form a tree before an iterative walk (no recursion, no panic
  on a broken blob).
- `scdata.rs` — parse `defaultProfile.xml` (action master list with the
  `joystick=` / `keyboard=` / `gamepad=` defaults, attribute or child-element
  form, child wins), `global.ini` (labels), `keybinding_localization.xml`
  (input token labels `jsN_` / `kb1_` / `gp1_`; the mouse device lands
  under `kb1_` too, see `Action.mouse_default`), and the user's
  `actionmaps.xml` (rebinds + `<options>` device map). `DeviceKind` and
  `parse_rebind` (prefix -> kind; kb/gp tokens keep their `+` modifiers).
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
- `diff.rs` — compares the joystick bindings of two sources (live file,
  binding profile, or backup) by SC token: the `(actionmap, action)` set per
  token in A vs B (label-only differences are not changes); added / removed /
  changed rows, sorted `jsN` then numeric-aware by input.

### Writing game files

- `gamefile.rs` — the one road every live-file write takes:
  `write_atomic` (temp file next to the target, `sync_all`, rename, read-back
  compare) and `replace_live_file` (new text must parse, verified backup
  first unless the user switched auto-backups off, then the atomic write).
  Used by `save_rebinds`, `apply_resort`, `apply.rs`, `backups.rs` (copy +
  restore), `binding_profiles.rs` (save / export) and `write_text_file`.
- `xmltext.rs` — helpers for the textual editors: `mask_markup` (a copy of
  the text with comments, CDATA and processing instructions blanked, same
  byte offsets — every search runs on the mask, every edit on the
  original), `tag_end` (quote-aware), `find_attr` / `attr` / `set_attr`
  (double or single quotes, whitespace around `=`).
- `rebind.rs` — textual `actionmaps.xml` rewrite writing rebinds (one
  binding per action and device kind, like SC: every `<rebind>` of that kind
  under the action is replaced, the first one keeping its other attributes
  such as `activationMode` unless the change carries its own `attrs` list;
  missing `<action>` / `<actionmap>` elements are created in SC's layout),
  the out-of-game counterpart of the keybinding screen. Every change passes
  `validate` (SC identifiers only, the input must be an SC token of the
  change's kind or a blank of it, attributes whitelisted) and the result
  `verify_applied` (the parsed before/after differ exactly by the changes).
- `resort.rs` — textual `actionmaps.xml` rewrite applying a resort (joystick
  `<options>` instances + `jsN_` prefixes in `input="..."`), the out-of-game
  counterpart of `pp_resortdevices`. Re-parses its output and checks it
  against the intent (`verify_applied`: every rebind in place with mapped
  tokens, devices on their new slots, nothing else changed).
- `apply.rs` — applies a profile or backup to the live file per device
  (`plan_apply`: the source's rebinds for the chosen `kb1` / `gp1` / `jsN`
  are written, live rebinds the source lacks are removed via an empty
  `RebindChange::input`, the source's rebind attributes carried along,
  everything else stays), backup "before apply"; a backup with every device
  chosen is restored byte for byte.
- `binding_profiles.rs` — SC's exported keybinding layouts
  (`controls/mappings/*.xml`, same content as `actionmaps.xml`, different
  root): list / import / export / delete, and "Save Profile" writes the
  live file in that layout (`to_profile_xml`, textual, in-game import
  unverified).
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

### App state, config, image-maps

- `config.rs` — JSON in the app config dir: the SC environments
  (`ENVIRONMENTS` = LIVE / HOTFIX / PTU / EPTU, each a base path + optional
  `global.ini` override; Windows default paths), the active one
  (`Config::base_path()` / `global_ini_override()`), the image-map choice
  per device, the auto-backup, debug-logging and startup update-check
  switches, the update channel (`UpdateChannel`: `stable` / `prerelease`). An
  older file's
  `ignored_devices` (the Exclude feature of 0.10 – 0.12) is ignored.
  `load` fills missing environments with defaults; no migration of older
  shapes.
- `imagemap.rs` — one folder per image-map, named by its id, under
  `<app_data_dir>/imagemaps/<id>/`: `imagemap.json` plus the image files it
  references by bare file name (png / jpg / jpeg / webp / svg / gif, no
  path). Bundled ones are compiled into the binary from
  `resources/imagemaps/<id>/` (`include_dir`, `BUNDLED`; read-only; clone
  into the user root with a fresh id, import assigns a fresh id too), so no
  folder ships next to the app; `build.rs` re-runs the build when a map
  changes. Zip export/import, image add/remove/read (data URL), validation.
  Pure logic takes a `Bundled` source (embedded dir, or a folder in tests)
  plus the `&Path` user root; the `#[tauri::command]` wrappers only resolve
  them.
- `names.rs` — the name rule (see Image-map data model).
- `textpath.rs` — the text tool's backend: `list_fonts` (the system's
  font families via `fontdb`, scanned once on first use; the app bundles
  no font for this on purpose, the map stores outlines only) and
  `text_path` (text + family + bold -> glyph outlines via `ttf-parser`,
  advance + `kern`, `\n` stacks lines, normalized into the 100x100 symbol
  box, plus the line box's `aspect`). At most 256 characters, control
  characters dropped.
- `lib.rs` — Tauri commands, state wiring and thread spawns, the Wayland
  DMABUF workaround, logging setup. `AppData` holds config, game data +
  load status, the bindings file + its load error, the binding index, the
  joystick order (`device_order`, re-taken from the live source on every
  clash report) + the Game.log snapshot, and the last logged clash summary.
  **Nothing slow under the `AppData` lock**: `resolve_input` runs per input
  event on the main thread and needs it, so `order::live()` (DirectInput)
  is taken *before* locking (`get_clash_report`, `apply_resort`) and the
  log is read only by the watch thread without the lock; the write
  commands hold it for their backup + write + reload on purpose (user
  actions, consistency). `get_load_status` hands the last load outcome to
  a frontend that mounts after the first load already finished;
  `system_info` (app version, OS, toolkit versions, `updater`: this
  install updates itself, see Releases) feeds the Device Info
  dumps. **Window-close guard** (`CloseGuard`): every `CloseRequested` is
  prevented and sent to the frontend as our own `close-requested` event,
  never Tauri's — with a JS listener on `tauri://close-requested` Tauri
  waits for the webview forever once it is dead. The frontend acks via
  `ack_close`, settles unsaved changes and calls `destroy`; no ack within
  2 s = dead webview, the window is destroyed here and the app exits if
  even that leaves it running.

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
  `ellipse`, `polygon`, `symbol` (`arrow`, `arrow2`, `rotate`, `curve`: a
  100x100 path stretched into a `w` x `h` box, rotatable; the two ring
  symbols take an optional `angle` of sweep, `imagemap.ts::arcArrowPath`),
  `arc` (outer `r`, `inner` as a fraction of it, `angle` of sweep from
  `rotation`), `wedge` (`r`, `angle`, `rotation`), `image` (its own file in
  the map folder, no colours, box like a symbol) and `path` (own SVG path
  data in the 100x100 box, box like a symbol; the editor's text tool writes
  these from any font installed on the editing machine, see `textpath.rs`,
  so a map never needs a font and a text is not editable afterwards, only
  replaced). Several shapes per input are fine.
- **Names** (image-map now, device names later): letters, digits, space,
  `_`, `-` and brackets `()[]{}` only, trimmed, at most 64 characters —
  nothing that needs escaping in a file name or URL.
  `imagemap::sanitize_name` / `src/names.ts` hold the rule; the backend
  validates, the editor strips as you type.
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
- **Bundled image-maps** (`src-tauri/resources/imagemaps/`, embedded at
  compile time, fixed ids
  `4b7a2c1e-…-000000000001` onwards): Keyboard US, Keyboard DE, Xbox
  controller, PlayStation controller, Keyboard US (TKL), Keyboard DE (TKL)
  (`…0001` to `…0006`) are generated, never hand-edited — a generator
  outside the repo holds the geometry and writes `image.png` +
  `imagemap.json`. VKB EVO Omni L (`…0007`) and VKB EVO R (`…0008`) are
  hand-made in the editor.
- **Default map per device** (`App.vue`, `chosenMapId`): the user's choice,
  else the first fitting hard-coded `BUNDLED_RULES` entry (not part of the
  model; gamepads by case-insensitive `*`/`?` wildcards on the controller
  name, keyboards by the OS layout from `kblayout.rs`), else `DEFAULT_MAPS`
  (US keyboard, Xbox pad), else the first map. Joysticks match by hardware
  id only.

## Releases and updates

- **One build serves every shape.** `tauri build` stamps the binary it
  packs into each bundle with its bundle type (`__TAURI_BUNDLE_TYPE_VAR_*`,
  `tauri::utils::platform::bundle_type`); the bare `target/release/`
  executable stays unstamped. `lib.rs::updater_available` accepts NSIS and
  AppImage only: the Windows installer and the AppImage update themselves,
  the bare executable (the standalone / portable download, just the file
  from `target/release/`) and the deb / rpm packages (the package manager's
  business) never load the updater module. No feature flags, no runtime
  switch.
- **Channels** (`update.rs`, the plugin's JS commands are not used: only
  the Rust side can pick the endpoint per check): **stable** reads GitHub's
  `releases/latest/download/latest.json` (never a pre-release);
  **prerelease** reads `releases/download/prerelease-version/latest.json`,
  the rolling `prerelease-version` release whose only asset CI replaces
  with the `latest.json` of every published release, pre-release or stable,
  so prerelease users get the pre-releases and the finals. **A pre-release
  is the finished binary under its final version number**: `bump-version.sh 0.14.0`, tag, draft, then publish it ticked as
  pre-release; if it holds, untick the box (GitHub's `released` event) and
  it is the stable 0.14.0 — same files, same signatures, no rebuild. If it
  does not, the next candidate is 0.14.1. No version suffixes anywhere:
  the version is compiled into the binary and compared with the feed, a
  promoted `-beta` build would offer itself forever. The GUI says
  "Pre-Release" (Settings channel, dialog badge).
- **Updater config** (`tauri.conf.json` `plugins.updater`): the minisign
  `pubkey` and the stable endpoint (the plugin's default; `update.rs` sets
  the channel's feed per check).
  `bundle.createUpdaterArtifacts` is on, so `tauri build` needs the private
  key as **content** in `TAURI_SIGNING_PRIVATE_KEY` (the `_PATH` variant is
  not honoured by the CLI, verified 2026-09-14) plus
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if set, and writes a `.sig` next to
  every bundle. **With an empty `pubkey` nothing is signed and nothing
  complains** — a release built that way can never be an update source.
  Keypair once via `pnpm tauri signer generate -w <file>`; the private key
  never enters the repo, and a lost key means no installed copy can ever
  update again.
- **Release feed**: `scripts/latest-json.sh <version> <assets dir> [notes]`
  assembles `latest.json` from the signed installer + AppImage (both
  platforms' files collected into one folder) — it goes to the GitHub
  release next to the assets.
  Windows and Linux updates are keyed `windows-x86_64` / `linux-x86_64`.
- **The version lives in `src-tauri/Cargo.toml` only** (`tauri.conf.json`
  has none, Tauri takes the crate's; `package.json` and `Cargo.lock` just
  follow). `scripts/bump-version.sh <x.y.z>` sets all three, commits
  "Bump version to x.y.z", tags `vx.y.z` and asks before pushing.
- **CI** (`.github/workflows/build.yml`): `test` (typecheck + build, cargo
  test, clippy `-D warnings`, on a tag also tag == Cargo.toml version),
  then `build` on ubuntu-24.04 (AppImage, deb, rpm; `NO_STRIP`; 22.04 ships SDL
  2.0.20, `input.rs` needs 2.24+) and
  windows-latest (NSIS + the bare exe zipped as `_x64-standalone.zip`),
  bundles as workflow artifacts (14 days). A manual run is a **test
  build**: artifacts only, no release, nothing the updater can see. A
  `v*` tag makes a **draft** release with every bundle, the `.sig` files
  and `latest.json` — nothing is live until the draft is published by
  hand: ticked as pre-release it feeds the prerelease channel, as a full release
  everyone. Secrets: `TAURI_SIGNING_PRIVATE_KEY`,
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. A release created by a workflow
  token never triggers another workflow (GitHub's loop guard), so the feed
  and README jobs only ever run on a publish by hand.
- **After a publish** (`.github/workflows/release.yml`, events
  `prereleased` + `released`, tags `v*` only): `prerelease-feed` copies the
  release's `latest.json` onto the `prerelease-version` release, which
  **exists once, made by hand** (`gh release create prerelease-version
  --prerelease --title "Prerelease feed" --notes "…" latest.json`; the
  Actions token uploads fine but was refused creating it, HTTP 403,
  2026-09-15) and the job fails with a clear message if it is missing;
  `readme` (full releases only, promotions included) runs
  `scripts/readme-updater.sh <version>`, which fills the README's tags —
  `<span id="release_v">…</span>` (the version, `v0.13.0`) and
  `<div id="release_dls">` … `</div>` (the download table, blank lines
  inside the div or GitHub renders it raw), both optional, any number of
  times — and commits to main as github-actions[bot]. HTML tags, not
  comments: MarkText strips comments.
- **Testing on Linux**: the dev build and the bare binary have no updater;
  only an AppImage does. The feed URL is fixed, so a manual check against a
  release that has no `latest.json` shows "Check failed".

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
cargo run --example parse_scdata -- <defaultProfile.xml> <global.ini>  # parser check
cargo run --release --example p4k_extract -- <Data.p4k> <out dir>      # P4K reader check
scripts/latest-json.sh <version> <assets dir> [notes]   # updater feed for a release
scripts/bump-version.sh <x.y.z>                 # version everywhere, commit, tag, offer push
scripts/readme-updater.sh <x.y.z>             # README release tokens (CI runs it)
```

The game data cache lives per version under `~/.cache/com.w00zla.bindsight/
<label>/` on Linux and `%LOCALAPPDATA%\com.w00zla.bindsight\cache\<label>\` on
Windows; delete it to force a re-extract.

## Prerequisites

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
  sysfs and `Game.log`) — never from SDL's enumeration, and never from the
  saved `<options>` slot either (that is what the file says, the order is
  what the game does). `Game.log` is the second opinion on both platforms,
  never the source. The Monitor, the deck and the Bindings List's column
  names follow the order; without one the joysticks show "no joystick
  order" and resolve nothing. A device plugged in or out shifts every slot
  behind it — inherent to SC, that is what Apply / the order fix are for.
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
- **Recording: the release decides** (`devices.ts::recordEdge`, rebind
  dialog and image-map editor): every press replaces the candidate, the
  release of that input takes it. A dual-stage trigger pulled through
  records stage 2 (stage 1 fires first and stays held on a VKB by default;
  the game's own screen binds on the first press and can only ever take
  stage 1). Combos take the modifiers held at the press. An axis is the
  candidate past half travel (`AXIS_RECORD`) and taken back at the centre;
  `AXIS_PRESS` (near full travel) only lights rows.
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
