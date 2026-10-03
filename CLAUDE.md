# BindSight — Claude project guide

Star Citizen binding visualizer and mapper. Answers "what does each button
do, and where does each action live?" by joining SC's config with live input
from joysticks, gamepads, keyboard and mouse.
Secondary: the tools the game lacks — editing bindings and device settings
(inversion, sensitivity curves, deadzone / saturation, mouse and gamepad
sensitivity) without starting it, profiles, backups, applying a profile or
backup per device, comparing two binding or settings sets, and fixing the
joystick order.

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

The app edits the game's **live** data (`actionmaps.xml`, `attributes.xml`,
the profiles folder), whose structure and processing can change with any
game patch. **BACKUP, BACKUP, BACKUPS**: the user must be able to undo any
change at any time. Concretely:

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
  `binding_profiles.rs`, `diff.rs`, `devconfig.rs`, `scdata.rs` parsing,
  every writing command); every edge case above has a test, added with the
  change.

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

- `TopBar` — modes **Monitor / Bindings / Config** in one segmented bar,
  a thin separator, then **Devices** as a button of its own in the same
  look; environment chip + dropdown, version chip, Refresh, gear. It is
  also the title bar (see the undecorated-window gotcha).
- `StartupTile` — covers every mode while the first game-data load after
  start runs, whatever its outcome, and until the startup image-map load
  is done (`endStartup` waits for `mapsReady`, so the maps never pop in
  after the stage is already showing).
- Monitor: `DeviceTile`, `StatusPanel`, `ImageStage` (+ `Splitter`),
  `LastInputCard`, `BindingsDeck` (flat rows or one bucket per input, same
  head as the Bindings List).
- `BindingsView` — the whole Bindings mode. Left: Game Bindings (the live
  file as the one item "Current"), `ProfilesPanel`, `BackupsPanel`. Right:
  the **Bindings List** for Current (the game's keybinding screen as a
  table, one toggleable column per device the file names, categories
  collapsible, "Set binding" / double-click = rebind dialog editing one
  input at a time, pending rebinds kept until
  Save / Discard in the action tile above it, which also holds Resort —
  swap two joystick slots the game ranks now, one swap at a time, either
  Apply to config = `apply_reorder` or the `pp_resortdevices` line in
  `ConsoleCommandDialog` — plus Save Profile / Create Backup) or **Compare**
  (a profile or backup picked on the left, always against Current, every
  token as a row, the diff chips filter; action tile: Apply — "Restore" for
  a backup — via `ApplyDialog` with the devices the two files bind and a
  joystick's target slot, then `apply_source` / Delete). Left column and
  the profiles panel are resizable.
- `ConfigView` — the whole Config mode: the game's device settings edited
  without starting it. Left (column and both panel heights resizable,
  remembered): Devices (kb1, the slotted pad gp1, the joysticks the game
  sees in its order — one without a slot shows the warning icon and "no
  joystick order", a log-only one is listed with "no input" and stays
  configurable, unseen ones are left out), `ProfilesPanel`, `BackupsPanel`.
  Right, for the picked device: a header tile like the Monitor's device
  tile (slot chip, info line; Create Backup, the pending count, Discard /
  Save), then for the keyboard a **Mouse** tile (six rows: label column,
  slider or `YesNo`, value; a value the files lack shows dimmed — the game
  screen's default for ADS 100 / Zoom Scaling Yes / 75, 0.00 for
  acceleration / smoothing, else a dash), for the gamepad **Thumbsticks**
  (the GamePad Sensitvity row, then one `AxisCard` per stick), for a
  joystick **Deadzone & Saturation** (one `AxisCard` per `x y z rotx roty
  rotz`, an orange "no input" badge on a log-only one), then the
  `OptionTable`. **Edits stay pending** as `ConfigChange`s in order: the
  working state is the saved one with them applied
  (`configModel::workingState`, so a reload underneath keeps them on top),
  Save sends the smallest set that gets there (`diffChanges`, in tree
  pre-order) via `save_device_config`; leaving the mode or opening Compare
  asks first (`requestLeave`, part of `App.vue`'s settle chain). A joystick
  without a slot gets one panel "No joystick order" / "Settings
  unavailable", no live file one "No bindings"; a slot without an
  `<options>` element leaves the option table read-only (the backend never
  creates a slot); without an `attributes.xml` its rows are disabled. Live
  bars and the curve dialog's dot come from `axis-raw`, held while the mode
  is mounted (a pad stick = max(|x|, |y|) of its pair). A profile or backup
  picked on the left replaces the editor with **Compare** (`ConfigCompare`;
  header: name, file · date or reason · version, Apply / Delete; table
  Device | Setting (+ group path) | Type | Current | <source>, chips Added /
  Removed / Changed with counts): computed in the frontend from two
  `get_device_config` views, keyboard and pad by kind, a joystick by its
  Product (spelled the same, else the same GUID), the settings-file rows
  only where both sides have that file. A click on a device returns to
  editing. Apply offers the current devices the source has a slot for, a
  joystick's source slot landing on its current one.
- `OptionTable` — one option tree as the table **Setting | Curve | Invert**
  ("Inversion & Sensitivity Curves": the game's two views of the tree as
  two columns, a cell empty where the node has no control): Expand all /
  Collapse all / Find like the Bindings List, `useTableColumns` +
  `ColumnHead` without sorting (Invert is the filler), groups collapsible.
  Curve cell: the exponent slider (sent on release), its number (none for
  a custom curve), a `CurveThumb` opening the `CurveDialog`, Set Default
  (only with an own value; without one it keeps its place, invisible, so
  the cell never shifts) and an amber chip counting the descendants' own
  curves a change here replaces; Invert cell: `YesNo`, Set Default (same
  rule), the same chip for inverts. Inherited values are dimmed
  (`configModel::effectiveCurve`: own › tree default › nearest group ›
  exponent 1).
- `CurveDialog` — the game's curve dialog (a `ConfirmDialog`, Cancel /
  Apply): grid, diagonal, the curve (a point list drawn as a Catmull-Rom
  spline — how the game smooths is unknown), the live dot. The slider and a
  number field (comma accepted, clamped and rounded to 0.1 on Enter / blur,
  "—" while points are shown) set the exponent and drop the points; a click
  on the grid turns the curve into the game's grid points (in = 0, 0.1 … 1
  on x^exp) plus the new one; points drag in both directions, the ends
  stay, none is deleted (the game cannot either), at most 64.
- `AxisCard` — one axis (joystick axis, pad stick): title = the game's
  deadzone label minus the words all axes' labels share ("X Axis"; the full
  label when nothing sensible is left — works for a translated
  `global.ini`), a live bar with deadzone / saturation laid over raw input
  and output, the Deadzone (and for a joystick Saturation) slider, and up
  to two bound-action chips plus "+N"; hovering (or focusing) the chip line
  opens a small overlay listing every bound action as vertical badges,
  upwards inside the card. Fixed height.
- `ApplyDialog` — the Apply dialog of both modes (a `ConfirmDialog`):
  checkboxes **Bindings** and **Settings** (both on), the caller's device
  list in the slot, Apply; the caller sends `apply_source`.
- `ProfilesPanel` / `BackupsPanel` — the left column's **Profiles** (the
  game's exported layouts; Save Profile in a name dialog, Import, Export)
  and **Backups** (Create Backup with a description dialog) of both modes;
  list and busy flag are the caller's `v-model`, so are the pick and what
  it means.
- `ImageMapEditor` — the Devices mode (Konva via `vue-konva`, three columns:
  devices + image-maps, canvas, live input + shapes); also hosts Device Info
  (Device List and Device Events tiles, each with its own Save, and the
  Axis Test). The Device List's Save appends what the tile does not show,
  read fresh from disk
  (`game_files_report`): the device lines of `Game.log`, the device part of
  `actionmaps.xml` verbatim (`scdata::device_section`) and every bound input
  per device (`scdata::bound_inputs`, numbered runs compacted).
  `AxisTest` is the diagnostics tile for external testers: they play while
  the app records in the background (SDL input arrives without focus) and
  press a marker button — picked Record-style (`recordEdge`, never a
  derived pad button) — the moment the game reacts. Start / Stop hold the
  raw stream; each marker press adds an entry (time since Start, per
  joystick / pad axis the raw value and `axisOutput` with the settings read
  at Start, the three axes that moved most since the previous marker;
  newest first, constant height). Save writes the head (`systemLine`, game
  environment + version), the devices (slot, name, hardware id, SC axis
  names), `device_config_text` verbatim, the deadzone / saturation each
  axis was computed with (display -> stored, or the fixed zone) and the
  entries. It exists to find out which deadzone the game applies and
  whether it rescales.
  `DeviceImage.vue` is the plain-SVG viewer.
- Dialogs and widgets: `SettingsDialog` (own-styled checkboxes, WebKitGTK
  would paint GTK's), `ConfirmDialog` (title, optional subtitle, required
  icon, buttons — each may be `disabled` or parked `side: "left"` —
  optional body slot, `captureKeys` keeps the keyboard capture on; behind
  every unsaved-changes / delete / game-file-write question, the Fix via
  config / Fix via console dialogs and the rebind dialog. **Dialogs close only
  via a button or Escape (the first `outline` button), never a backdrop click**
  — same for `SettingsDialog`),
  `ConsoleCommandDialog` (how to open the game console, the command line, a
  Copy button; used by the order fix and Resort), `Dropdown` (a select in
  the app's look — WebKitGTK paints a native popup no CSS reaches),
  `ScrollRail` (one non-wrapping row that scrolls sideways, wheel or end
  arrows, scrollbar hidden), `Toasts` (`ok` / `error` / `hint`: green done,
  red broken, blue guidance; a hint is logged as info, every error toast
  goes to the app log), `Icon` (inline stroke SVGs by name — never emoji),
  `YesNo` (the game's No | Yes toggle, `dim` for a value not set),
  `CurveThumb` (a curve's shape in a small box, for custom curves),
  `WindowEdges`, `AppFooter` (credits, a `mark` slot before the logo; the
  logo and the link-styled version button open the **App Update dialog**),
  `VersionDialog` (a `ConfirmDialog`: the channel dropdown at the head's
  right edge, `ConfirmDialog`'s `head` slot — the only
  place the channel is picked, filled by `update_channels` at every open,
  a pick saved via `set_update_channel` and checked at once with
  `check(true)`; a channel shows its id —
  Current chip = the running version, Update chip = the found version /
  "Checking…" / a dash, then download progress, an error line, the dev
  toggle; the updater's buttons after Close).
- **Updater** (`update.ts`, `UpdateMark.vue`; backend `update.rs`):
  loaded by `App.vue` via dynamic import in every install; it installs
  only when `system_info.updater` is true (see Releases), elsewhere
  (`selfUpdate` false) it checks the same way and a found update gets
  "Open Webpage" (the project's GitHub page) instead of Install;
  in a dev build it is the simulated one
  (`createUpdater(true)`: a toggle in the dialog fakes an
  available update and a download, a second one the link-only path, so
  the GUI parts can be looked at; the
  simulation code sits behind `import.meta.env.DEV` and is not in a
  release). `createUpdater()` returns reactive state (`idle` /
  `checking` / `current` / `available` / `downloading` / `installing` /
  `error`), `check(switched)` (`check_update`; the backend reads the
  channel from the config),
  `install()` (`install_update`, progress via `update-progress` events,
  the backend restarts the app), the footer `mark` and the dialog
  `buttons()` (Check Update until one is found, then Install). The startup
  check runs last in `onMounted` unless Settings switched it off
  (`Config::update_check`); a found update opens the dialog and puts the
  mark in the footer — **nothing is downloaded or installed without the
  user's click**. A failed startup check only logs.
- Shared modules: `imagemap.ts` (image-map types/helpers incl. token <->
  input key, `arcArrowPath`), `keyboard.ts` (webview keyboard capture,
  `KeyboardEvent.code` -> SC key name, feeds the same handler as
  `joy-input`; also drives mouse capture, armed only while `recording` is
  on), `devices.ts` (`deviceName` / `deviceKey` / `deviceIcon`,
  `recordEdge`, `DERIVED_PAD_BUTTONS`; remembered GUI state is keyed by
  hardware id, never by SDL's index), `configModel.ts` (the Config mode
  without GUI: curves and their effective value, the tree helpers, the
  pending changes applied the way the game applies them — a group's curve
  wipes the curves below, a group's invert the inverts below — `diffChanges`,
  the Compare rows; display units throughout), `axisStream.ts`
  (`holdAxisStream`: every view showing raw axes holds the `axis-raw`
  stream, `set_axis_stream` follows "anyone holds it"; **the one output
  rule** `axisOutput` for axis cards, the curve dot and the Axis Test — 0
  inside the zone, full travel at or beyond saturation, the raw value in
  between, no rescale; zones are the stored values, display × 0.99 /
  × 0.899, else input.rs's fixed ones), `names.ts` (the name rule, see
  Image-map data model), `colour.ts` (`#rrggbb` / `#rrggbbaa` helpers for
  the pickers),
  `types.ts` (all device/input/binding types), `logging.ts` (console
  forwarding, see Logging), `persist.ts` (`persistedRef`: a ref mirrored
  into localStorage — layout and preferences only, never a filter: a
  filter kept across a restart can match nothing any more, e.g. a device
  filter on a joystick that is gone, and the list shows nothing without a
  hint why).
- **Tables** (bindings deck, Compare, Bindings List, option table, settings
  Compare): `tableColumns.ts` (`useTableColumns`: sort state, widths, grid
  template, localStorage persistence; the column list may be reactive —
  the Bindings List's device columns come from the file) + `ColumnHead.vue`
  (sortable headers with an icon per column, resize grips). The last
  column is the `1fr` filler, the others carry px defaults, every cell
  truncates with an ellipsis.
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
  **Axis rules, all here so every consumer of `joy-input` sees one
  stream**: a resting zone per axis — a joystick axis and a pad stick take
  the game's configured deadzone of that device and axis (the **stored**
  `<deviceoptions>` value × 32767; which of stored and displayed the game
  applies is unverified), the fixed zones are the fallback where none is
  configured and the triggers' only zone (joysticks 4000, pad sticks 8000,
  pad triggers 4000) — and at or beyond a configured saturation the value
  is forwarded as full travel, no other rescaling (`shape_axis`); a delta
  filter plus at most one event per axis per 20 ms (the centre always
  passes), and of a stick's two axes (pad sticks, joystick `x`/`y`) only
  the further deflected one is forwarded. The configured zones arrive as
  `InputControl::zones` (`AxisZones`, an `Arc<RwLock<ZoneMap>>`: hardware
  id -> SC axis -> stored deadzone / saturation, written by
  `reload_bindings` from `devconfig::axis_zones`), read per axis event —
  never through the `AppData` lock. **`axis-raw`**: a second, unfiltered
  stream (`AxisRaw`: SDL instance id + GUID, kind, `values` — a joystick
  one per SDL axis index, `value / 32767`; a gamepad, Wine-rule pads
  translated, exactly six: left x, left y, right x, right y in −1..1,
  triggers 0..1), only while `set_axis_stream` has switched
  `InputControl::stream` on: a full snapshot on switching on (and on
  hot-plug), then each changed device at most every 33 ms, the last value
  of a burst within one loop tick; no cost while off. **Derived pad buttons**
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
  (`x y z rotx roty rotz slider1 slider2`); only the joystick-class
  top-level collections count (a keyboard's interface may carry a mouse
  collection with its own X/Y).
- `kblayout.rs` — `keyboard_layout` command: xkb code (`de`, `us`, …) via
  `localectl` / `vconsole.conf` on Linux, `GetKeyboardLayoutNameW` on Windows.

### Joystick order (see the order gotcha for the facts)

- `order.rs` — `DeviceOrder` (joysticks in SC's order + timestamp), the one
  type every order source yields: `instance_for_guid` is what the Monitor
  resolves against (`resolve_input`; without an order no joystick input
  resolves, keyboard and pad still do), `same_ranking` the log-vs-live
  check. **`assign(enumerated, saved)` is the slot rule** (see the order
  gotcha): the enumeration from `Game.log` (`gamelog.rs` / `logwatch.rs`,
  SC's own record of what it saw at its last start) reconciled with the
  device map in the file's `<options>` — the file's slots while the
  attached set is the saved one (`OrderSource::File`), derived slots after
  a set change (`SetChanged`: gap closed on removal, a new device inserted
  at enumeration index + 1, the latter an assumption from one observed
  case). The log is the game's *last* start, so `attached_only` first drops
  the logged joysticks that are unplugged now (`AppData::attached_joysticks`:
  SDL's joysticks plus the hid-only ones, taken by the clash-report
  commands outside the lock — hidapi enumerates on every call) — the
  next start will not see them, and that is the clash the Monitor
  predicts. `Assignment::describe_source` is the app-log line. A live
  enumeration never was SC's rule; `dinput.rs` / `wineorder.rs` stay only for
  the `dinput_order` / `wine_order` diagnostics (and `wine_keys` /
  `dinput_devices`, the Device List's `wine` / `dinput` rows), no longer
  wired into the order — nothing composes them, only their pieces
  (`dinput::list`/`to_order`, `wineorder::wine_devices`/`rank`) feed the
  examples and those rows. The `dinput` row is matched to a device by its
  path, so identical devices (same Product GUID) stay apart. No usable log
  means no order — there is no live fallback.
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
  decides it: HID usage joystick / gamepad only; one backend per device,
  decided on a hidraw interface's **first** top-level collection
  (`hidraw_taken`): mouse / keyboard / digitizer first never hidraw, a
  joystick / gamepad first by the `hidraw_preferred` vendors (all VKB / VPC,
  some TM / Fanatec / Simucube, DualShock / DualSense), anything else
  always hidraw (the Keychron Link dongle: bar-code page first); the node
  must open read-write; SDL for the rest. hidclass splits a multi-collection
  interface into `&ColNN` children (+ `&NNNN` instance suffix), each a
  device. An SDL device that SDL maps as a controller (unless wheel / flight
  stick) or has exactly 6 axes and 14+ buttons is a gamepad (`&IG_00`, SC's
  `xinput`, no slot). `rank` is the pure part with tests, `enumerate` reads
  hidapi + sysfs. Not replicated: registry overrides, the evdev backend,
  winebus's synthesized serial for devices without one.
- `gamelog.rs` — parse `Connected joystickN: <Product {GUID}>` into a
  `DeviceOrder` (gamepad lines are not read: no slot), the raw `product`
  kept byte for byte (it is the `Product` attribute SC writes). **This is
  the enumeration `order::assign` starts from**: `device_order` is the
  assignment of the last parsed log against the loaded file
  (`lib.rs::refresh_device_order`, logged on every change). Parsing fails
  soft, never panics.
- `logwatch.rs` — the `Game.log` watch (pure state machine; the thread is
  `lib.rs::spawn_game_log_watch`, **the only reader of the log** for the
  order, always outside the `AppData` lock; the one other read is the
  Device List export's `game_files_report`, once per Save): a metadata
  poll every 2 s; the first poll and an environment change `Adopt` the
  file (one full read); a *new* file
  (shorter, other creation time, appeared / vanished — NTFS name tunneling
  keeps the creation time on a quick recreate) is read incrementally
  (`Tail`: only the bytes appended since the last read) for the whole 90 s
  window — the joystick lines land one by one, 200 ms apart, a read may
  fall between them, so the first line found is not the order yet; every
  changed outcome replaces the log snapshot and emits `gamelog-changed`
  (`{started}`: true for the first outcome of a new file — the frontend
  announces a clash flip only then; a start by itself is not toasted).
  Writes to a running log are ignored on purpose.

### Game data and bindings

- `scinstall.rs` — the configured install. `validate_install` first
  (`REQUIRED_FILES` = `Data.p4k`, `build_manifest.id`, the live
  `actionmaps.xml`; anything missing aborts the whole load with one message
  and `ScState::invalid_install` keeps `reload_bindings` from reading
  anything). Version from `build_manifest.id` (`ScVersion`, label
  `<branch minus sc-alpha->.<P4 changelist>`, e.g. `4.10.0-hotfix.12572603`).
  Game data (`ScData`: action master list, token labels, the option trees
  and `ConfigLabels`, the labels of the settings outside them): the three
  files read straight out of `Data.p4k` (`p4k.rs` + `cryxml.rs`, `P4K_FILES`,
  ~150 ms), `scdata::parse_*` (unlabeled actions dropped), cached as JSON
  under `<app_cache_dir>/cache/<label>/` (`lib.rs::sc_cache_root`; on Windows
  LocalAppData also holds the logs and the WebView2 profile, so the version
  folders keep their own `cache` folder — Linux uses the same layout).
  `scdata.json`
  carries a `format` stamp (`CACHE_FORMAT`, currently 7): bump it whenever
  the cached shape changes meaning, the cache is then re-extracted once.
  Loaded in a background thread at start and on environment change
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
  `parse_option_trees`: the `<optiontree type>` per kind of
  `defaultProfile.xml` as `OptionTree` (`nodes` = the children of the
  node named `inversion`, the visible rows) of `OptionNode`s (name, resolved
  `UILabel`, the `UIShowInvert` / `UIShowCurve` flags — `1` control, `-1`
  group header, `0` / missing none; `UIShowSensitivity` is not read — and
  the shipped `invert` / `exponent` / `<nonlinearity_curve>` points,
  `reset="1"` ignored); a tree without `inversion` yields the `*_curves`
  children, without either nothing; never panics.
- `bindings.rs` — `BindingIndex` (token -> bound actions, all device kinds,
  defaults per kind with a per-kind "touched" rule), `button_token` /
  `hat_token` (+1 offset), `instance_for_guid` (the saved `<options>` slot,
  used by the clash analysis and the Bindings List only), `resolve_bindings`,
  `analyze_clash` (saved `<options>` vs. the assigned order from
  `order::assign`, a `DeviceOrder`; a difference only ever comes from a set
  change; an empty order is an order, every saved slot then "missing"),
  `plan_resort` / `resort_commands`.
  A joystick SDL lists that the order lacks is `unseen`: the GUI drops it
  from Monitor, deck, image-map list and the Config mode's device list
  without a word — only the Device List (`game` row) and the clash line in
  the app log name it. The reverse
  — a joystick the order lists that SDL lacks (e.g. one on SDL's joystick
  blacklist that Wine reaches through hidraw, the Keychron Link dongle) —
  is a `LogOnlyJoystick` in the frontend (`App.vue`): a Monitor tile
  without input ("input handling not supported"), a dim "(no input)" note
  on its Bindings List column, a Config mode device with "no input" (no
  live bar, still configurable), and the `game` row of its hid-only entry
  in the Device List.
- `diff.rs` — compares the joystick bindings of two sources (live file,
  binding profile, or backup) by SC token: the `(actionmap, action)` set per
  token in A vs B (label-only differences are not changes); added / removed /
  changed rows, sorted `jsN` then numeric-aware by input.

### Writing game files

- `gamefile.rs` — the one road every live-file write takes:
  `write_atomic` (temp file next to the target, `sync_all`, rename, read-back
  compare), `replace_live_file` (new text must parse, verified backup
  first unless the user switched auto-backups off, then the atomic write)
  and `replace_live_config` (the same for `actionmaps.xml` and / or the
  `attributes.xml` next to it: each new text must parse as its kind of
  file, **one** backup of both, then the writes, `actionmaps.xml` first; a
  failed second write says what is already written). `replace_live_file`:
  `save_rebinds`, the order fix and Resort; `replace_live_config`:
  `save_device_config`, `apply_source`; `write_atomic` also serves
  `backups.rs` (the copies), `binding_profiles.rs` (save / export) and
  `write_text_file`.
- `xmltext.rs` — helpers for the textual editors: `mask_markup` (a copy of
  the text with comments, CDATA and processing instructions blanked, same
  byte offsets — every search runs on the mask, every edit on the
  original), `tag_end` (quote-aware), `find_attr` / `attr` / `set_attr`
  (double or single quotes, whitespace around `=`), `remove_attr` (with
  the whitespace before it) / `insert_attr` (after the last attribute).
- `rebind.rs` — textual `actionmaps.xml` rewrite writing rebinds (one
  binding per action and device kind, like SC: every `<rebind>` of that kind
  under the action is replaced, the first one keeping its other attributes
  such as `activationMode` unless the change carries its own `attrs` list;
  missing `<action>` / `<actionmap>` elements are created in SC's layout),
  the out-of-game counterpart of the keybinding screen. Every change passes
  `validate` (SC identifiers only, the input must be an SC token of the
  change's kind or a blank of it, attributes whitelisted) and the result
  `verify_applied` (the parsed before/after differ exactly by the changes).
- `resort.rs` — textual `actionmaps.xml` rewrite replicating
  `pp_resortdevices joystick A B`, swap for swap in console order
  (`bindings::resort_swaps` turns the clash plan into that chain; Resort in
  the Bindings mode is one swap). **What the command does to the file**
  (byte-exact against the game's own output, 2026-09-20): the *contents* of
  the two slots swap — every `jsA_` / `jsB_` token in a `<rebind input>`
  (blanks and combos alike) and the child elements of the two
  `<options type="joystick">` elements (invert / exponent settings, an
  emptied element written self-closing, a filled one opened) — while the
  `instance` / `Product` attributes and the element order stay: that map is
  the game's per-session record, which the command never touches. Not
  replicated, not understood: the game left two blank rebinds on the
  source slot (`turret_toggle_mouse_mode`, `v_cycle_pitch_ladder_mode`)
  while moving 350 blanks of the same shape. Every swap re-parses its output
  and checks it (`verify_applied`: rebinds in place with swapped tokens,
  device map unchanged; `verify_children_swapped`).
  `rewrite_device_map` records a device map in the `<options>` (raw
  `Product` per slot, byte for byte, the rest emptied; element order,
  `instance` and children stay) — what the game writes at its next save.
  The **order fix** (`apply_resort`) is the swap chain plus that map for the
  assigned order: afterwards the saved set is the attached one, the game
  follows the file whatever its rule for a set change is, and the clash is
  gone; Resort in the Bindings mode (`apply_reorder`) is the bare command.
- `apply.rs` — `apply_source(source, devices, bindings, settings)`, the
  one entry point of both modes' Apply dialog (`plan_source` the pure part,
  nothing written there): **bindings** = `plan_apply` (the source's rebinds
  for the chosen `kb1` / `gp1` / `jsN` are written, live rebinds the source
  lacks are removed via an empty `RebindChange::input`, the source's rebind
  attributes carried along; `DeviceSel.target` lands a source joystick on
  another live slot, each slot taken once); **settings** =
  `devconfig::apply_settings` for the same source -> target pairs, plus,
  only from a backup holding an `attributes.xml`, `copy_attributes` for the
  chosen devices' managed names (one the backup lacks is removed live; a
  profile or an older backup leaves the live game settings alone, a backup
  with them and no live file is refused). Everything else stays. A backup
  with both ticked and `every_device_chosen` (each device on its own slot:
  kb1 and gp1 always, every `jsN` a rebind — a blank one too — or an
  `<options>` element of either file names) is put back **byte for byte**,
  plus its managed game settings — never the whole `attributes.xml`. One
  backup of both files first (`replace_live_config`, reason "before
  restore" for the byte-exact case, else "before apply"), then
  `reload_bindings`.
- `binding_profiles.rs` — SC's exported keybinding layouts
  (`controls/mappings/*.xml`, same content as `actionmaps.xml`, different
  root): list / import / export / delete, and "Save Profile" writes the
  live file in that layout (`to_profile_xml`, textual, in-game import
  unverified).
- `backups.rs` — backups of the live files, one folder per backup
  (`meta.json` with reason + game version, `actionmaps.xml`, and
  `attributes.xml` when the live one exists — older backups have none and
  keep working) under `<app_data_dir>/backups/<id>/`, id =
  `YYYYMMDD-HHMMSS` (UTC) with a `-2`, `-3`, … suffix on collision
  (folders are created, not checked, so concurrent backups cannot
  collide). Taken manually (Create Backup asks
  for a description in a dialog, "manual" by default) and, always, before
  every write to the live files (rebind, apply, restore, order fix, device
  settings); a backup counts only once every copy compares byte for byte
  with its source, so the user can also copy them back by hand. Applying
  or restoring a backup is `apply_source` (`apply.rs`), which refuses one
  that no longer parses. `Config::auto_backup` (default on) gates the
  backups before every write, a restore included — the one exception to
  the safety rule, the user's explicit choice: the Settings checkbox asks
  with a warning before it goes off.

### Device settings

- `devconfig.rs` — the game's settings screens out of game: inversion,
  sensitivity curves, deadzone / saturation, mouse and gamepad
  sensitivity, in `actionmaps.xml` and `attributes.xml` (the game facts:
  see the device-settings gotchas). **Reading**: `parse_device_config`
  (root `<ActionMaps>`, live file and profiles alike, never panics) yields
  the stored model `DeviceConfig` — every `<options type instance
  [Product]>` with its node children (attributes in file order, the
  `<nonlinearity_curve>` points — a curve element without points is no
  custom curve —, a count of other content in the node and in its curve
  so an element holding a comment never counts as empty) and every `<deviceoptions
  name>` (the raw Product string, `Controller (Gamepad)`, `Mouse`) with its
  `<option input …>` entries, raw texts throughout; `parse_attributes`
  reads `<Attributes><Attr name value/>`. `view` turns both into the
  `DeviceConfigView` in display units (rounded to the GUI step, exponents
  to two decimals to drop the game's float noise; of duplicate `<option>`s
  the last wins — an assumption; the mouse's `<deviceoptions>` becomes
  part of `MouseView`; `has_attributes` false for a profile or a backup
  without the file). `config_labels` resolves the rows outside the trees
  (`ui_DeadzoneJoystick<Input>`, `ui_SaturationJoystick<Input>`,
  `ui_DeadzoneXI<Input>`, `ui_GamePadSensitivity`, `pause_OptionsMouse*`)
  against `global.ini`, the game's English label as fallback; cached with
  `ScData`. **Writing** is textual like `rebind.rs` (`xmltext` plus
  `rebind`'s layout helpers), untouched bytes stay: `apply_config` takes
  the `actionmaps.xml` part of a `ConfigChange` list (display units:
  `Curve` exponent / points / `Default`, `Invert` on / off / `None` = Set
  Default, `Deadzone`, `Saturation`, `MouseAcceleration`,
  `MouseSmoothing`); `plan` validates each against the trees and the file
  as it stands and breaks it into edits. **Group wipe**: a curve (not
  `Default`) on a node with children also clears every descendant's curve,
  an invert set on one every descendant's invert; Set Default removes only
  the node's own value. A node element left empty goes, the `<options>`
  element stays (self-closing when empty); a curve replaced or removed
  loses only its `<point>`s, a comment in it stays in place (a removed
  curve holding one keeps its element); a new child lands at its
  tree-order (pre-order) position, `invert` before `exponent`, points
  sorted by `in`; a slot the file lacks is refused, never created.
  `<deviceoptions>`: every duplicate `<option>` gets the new value, a new
  one is appended last, a missing element is created after the last
  `<deviceoptions>` (else before the first `<options>`). Validation: node
  names from the kind's tree, a set value only where its control is shown
  (flag 1); inputs from the fixed lists (`JOYSTICK_AXES`, `GAMEPAD_AXES`;
  the pad has no saturation, the mouse no deadzone); exponent 0.1 – 3.0
  rounded to 0.1, 2 – 64 points in 0..1 incl. 0/0 and 1/1, 0..1 values in
  0.01 steps; the device must be in the file, `Controller (Gamepad)` or a
  joystick `<options>` Product; nothing that needs escaping. Display ->
  stored by the `*_SCALE` factors and `ManagedAttribute::stored`, written
  `%.8g` of the `f32` into `actionmaps.xml` (`format_actionmaps_float`:
  `0.69999999`, `0.13860001`, `3`) and `%g` into `attributes.xml`
  (`format_attributes_float`: `8.88889`, `40`). Every rewrite is re-parsed
  and must equal the parsed original with the edits applied to its model,
  rebinds and joystick map untouched (`check_rewrite`). `apply_attributes`
  patches only the managed names (`MouseSensitivity`,
  `ADSMouseSensitivity`, `ZoomSensitivityMultiplierToggle`,
  `ZoomSensitivityMultiplier`, `Sensitivity`): every duplicate updated, a
  missing one inserted at its sorted position, checked the same way.
  **Apply**: `apply_settings` makes the live file hold a source's settings
  per `SlotPair` (source slot -> target slot): the `<options>` children
  exactly the source slot's (source values taken as stored, validated as
  plain numbers; live ones it lacks removed) and the `<deviceoptions>`
  values exactly the source device's (joystick: the Product of each side's
  slot; pad and mouse: their fixed names; live values the source lacks
  removed — unlike the game's import, which merges; values outside the
  kind's inputs stay); `copy_attributes` does the same for a backup's
  managed attributes (`managed_attributes`: the mouse's four for kb1, the
  sensitivity for gp1). `axis_zones` feeds the input thread (stored values
  by hardware id, `hardware_id_of`); `config_text` is the Axis Test
  report's settings part (every `<deviceoptions>` / `<options>` element and
  the managed `<Attr>` lines verbatim). Commands: `get_option_trees`,
  `get_config_labels`, `get_device_config(source)` (`Current` the live
  files, a profile its file only, a backup its copies; an unreadable
  `attributes.xml` only drops its values, logged), `save_device_config`
  (`apply_config` / `apply_attributes`, `replace_live_config` with reason
  "before config", `reload_bindings`; an attribute change without a live
  `attributes.xml` is refused), `device_config_text`.

### App state, config, image-maps

- `config.rs` — JSON in the app config dir: the SC environments
  (`ENVIRONMENTS` = LIVE / HOTFIX / PTU / EPTU, each a base path + optional
  `global.ini` override; Windows default paths), the active one
  (`Config::base_path()` / `global_ini_override()`), the image-map choice
  per device, the auto-backup, debug-logging and startup update-check
  switches, the update channel (`update_channel`, JSON key
  `update_channel_id`: `stable`, a channel id, or empty = the running
  build's own channel — its own key because 0.16 and older parse
  `update_channel` as a `stable` / `prerelease` enum and a failed parse
  there drops the whole config; this app ignores that old key). An
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
  joystick order (`device_order`: `order::assign` of the Game.log snapshot
  reduced to the joysticks attached now against the file's `<options>`,
  re-taken by `refresh_device_order` on every clash report, reload and log
  change) + that Game.log snapshot + the attached joysticks as of the last
  report, the last logged clash summary and order line, the stamps
  (mtime + length) of the `actionmaps.xml` and `attributes.xml` last read,
  and `axis_zones` (a clone of the input thread's `AxisZones`).
  `reload_bindings` also hands the input thread the configured zones
  (`devconfig::axis_zones` of the file, empty = the fixed zones without a
  readable one) through a short write lock on that `RwLock`; the input
  thread gets the zones and the raw-stream flag as managed `InputControl`,
  never via `AppData`. **`spawn_actionmaps_watch`** polls both stamps
  every 2 s (under the lock, a metadata call) and, once a changed stamp held
  still for one more poll, runs `reload_bindings` and emits
  `bindings-changed` (payload the `LoadStatus`; `settings-changed` when
  only `attributes.xml` changed; the frontend takes either like a Refresh
  and toasts a hint) — the game's console commands, its
  keybinding and options screens land in the GUI without a Refresh (a
  missing `attributes.xml` is a state, no stamp). The app's own writes
  end in `reload_bindings`, which records both stamps, so they never come
  back as a change; idle until the first load and while the environment is
  invalid.
  **Nothing slow under the `AppData` lock**: `resolve_input` runs per input
  event on the main thread and needs it; the order costs nothing there
  (`order::assign` over snapshots — no enumeration; the hidapi scan for the
  attached set runs in the report commands before they take the lock), the
  input thread's zones never need it, and the log is read only by the
  watch thread without the lock; the write commands hold it for their
  backup + write + reload on purpose (user actions, consistency). `get_load_status` hands the last load outcome to
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
  from `target/release/`) and the deb / rpm packages never install: their
  `check_update` reads the channel's `latest.json` by hand
  (`update.rs::check_feed`, `semver` against the running version) and the
  dialog links the project page. No feature flags, no runtime switch.
- **Branches and versions**: `main` is the live line — only validated
  work lands there, live versions `x.y.z` are tagged there. A channel lives
  on `channel/<id>` cut from main, its versions are `x.y.z-<id>.<n>` (semver
  pre-release; id `[a-z][a-z0-9-]{0,31}`, never `stable`). Live fixes the
  tester needs are merged from main into the branch; after a validated
  user test the branch is merged into main and the live version is cut
  and **built again** — nothing is promoted. Semver does the rest:
  `0.18.0-x.3` < `0.18.0`, so the final replaces the channel version, and a live
  `0.17.4` < `0.18.0-x.1` never pulls a tester back. A channel version whose base is
  not above the latest live version is never offered: after a live release
  an active branch merges main and moves to the next base.
- **Channels** (`update.rs`, the plugin's JS commands are not used: only
  the Rust side can pick the endpoint per check): **stable** is built in
  and reads `releases/latest/download/latest.json` (never a pre-release).
  The other channels are `channels.json` on main (`{"channels": [{"id",
  "tag"}]}`, no display names — the id is shown), read from raw.githubusercontent.com
  at every check and dialog open (`parse_channels` drops an entry whose
  tag is not `v<x.y.z>-<id>.<n>`); a channel reads the `latest.json` of
  its tag's release. CI moves the tag when a channel version is published, removing a
  channel after its merge is by hand — its testers fall back to stable and
  get the final. No `channels.json` (HTTP 404) = stable only. The
  channel in effect (`resolve`): the chosen one while listed, else the
  running build's own (`channel_of` its version), else stable. Only a
  newer version is offered, except in the check right after a channel
  pick (`switched`, the plugin's `version_comparator`): then any other,
  so a switch back to stable can go down. **Unverified: whether the NSIS
  and rpm bundlers take a version with a pre-release suffix** — the first
  channel tag's CI run is the test.
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
  follow). `tools/releasectl.sh` (a Textual TUI, `tools/releasectl/`)
  sets all three, commits "Bump version to …", tags `v…` and asks before
  pushing — a live version only on main, a channel version only on its branch — and
  manages the rest of the cycle: new channel branch, bump channel / bump
  live, the CI run of a tag with its steps in a Build tab, a draft's release notes in
  `nano`, publish, sync main into a branch, finalize (merge + bump
  live), remove a channel. Keys and runs in `tools/releasectl/README.md`.
- **CI** (`.github/workflows/build.yml`): `test` (typecheck + build, cargo
  test, clippy `-D warnings`, on a tag also tag == Cargo.toml version),
  then `build` on ubuntu-24.04 (AppImage, deb, rpm; `NO_STRIP`; 22.04 ships SDL
  2.0.20, `input.rs` needs 2.24+) and
  windows-latest (NSIS + the bare exe zipped as `_x64-standalone.zip`),
  bundles as workflow artifacts (14 days). A manual run is a **test
  build**: artifacts only, no release, nothing the updater can see. A
  `v*` tag makes a **draft** release with every bundle, the `.sig` files
  and `latest.json` (a channel tag's draft is marked pre-release) — nothing
  is live until the draft is published by hand: a channel version as a pre-release
  (its channel), a live one as a full release (everyone). Secrets: `TAURI_SIGNING_PRIVATE_KEY`,
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. A release created by a workflow
  token never triggers another workflow (GitHub's loop guard), so the
  channel, feed and README jobs only ever run on a publish by hand.
- **After a publish** (`.github/workflows/release.yml`, events
  `published` + `released`, tags `v*` only — CI drafts a channel version
  already marked pre-release, and publishing such a draft fires only
  `published`, never `prereleased`; a manual run with a `tag` input redoes
  channel and feed for a release already published): `channel`
  (pre-releases with a channel tag) points the channel in `channels.json` at the tag — created on
  its first version, never moved back to an older tag — and
  commits to main as github-actions[bot]; `prerelease-feed`, the legacy
  feed only 0.16 and older read (their "Pre-Release" setting; drop it once
  none is left), copies every published release's `latest.json` onto the
  `prerelease-version` release, which
  **exists once, made by hand** (`gh release create prerelease-version
  --prerelease --title "Prerelease feed" --notes "…" latest.json`; the
  Actions token uploads fine but was refused creating it, HTTP 403,
  2026-09-15) and the job fails with a clear message if it is missing;
  `readme` (full releases only) runs
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
cargo run --example dinput_order                # DirectInput enumeration, diagnostics (Windows)
cargo run --example wine_order                  # Wine's enumeration, diagnostics (Linux)
cargo run --example pad_events                  # pad mapping + controller-level events
cargo run --example parse_scdata -- <defaultProfile.xml> <global.ini>  # parser check
cargo run --release --example p4k_extract -- <Data.p4k> <out dir>      # P4K reader check
scripts/latest-json.sh <version> <assets dir> [notes]   # updater feed for a release
tools/releasectl.sh                             # releases + channels (TUI): versions, tags, channels.json
scripts/readme-updater.sh <x.y.z>             # README release tokens (CI runs it)
```

The game data cache lives per version under `~/.cache/com.w00zla.bindsight/
cache/<label>/` on Linux and `%LOCALAPPDATA%\com.w00zla.bindsight\cache\<label>\`
on Windows; delete it to force a re-extract.

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
- **SC's `jsN` slots follow the saved device map, not the enumeration**
  (test series v2, Windows, SC 4.10, 2026-09-20, in-cockpit verified):
  while the attached joysticks are the ones in the file's
  `<options type="joystick">`, each takes the `instance` saved for its
  Product GUID — swap two `Product` attributes and the sticks swap in the
  cockpit. Only a **set change** derives new slots: a stick gone closes the
  gap (`js4` -> `js3`; the tokens stay, so every stick behind the gap loses
  its bindings — the real clash), a stick new was seen once (enumerated
  second, three saved: it took `js3`, the saved third moved to `js4`;
  `order::assign` replicates that as "insert at enumeration index + 1",
  an assumption). The game writes its map at exit and on a keybinding
  export, never at start, so after a regular exit the file is the game's
  own state. `Game.log`'s `Connected joystickN: <Product {GUID}>` lines
  (`joystickN` -> `js(N+1)`) are the enumeration the rule starts from and
  the only source of "what the game saw"; the live enumeration (`dinput.rs`
  / `wineorder.rs`) is diagnostics only, and SDL's order is nothing (its
  Windows backend even prepends). No usable `Game.log` = no order: the
  joysticks show "no joystick order" and resolve nothing (no live fallback).
  The Monitor, the deck, the Bindings List's column names and the Config
  mode's device slots follow the assigned order.
- **`pp_resortdevices` logs 0-based slots**: `pp_resortdevices joystick 2 3`
  writes `N actions moved from js1 to js2` into `Game.log` (the same quirk
  as `Connected joystick0` = `js1`). The arguments are 1-based `jsN`. A
  blank rebind counts as an action; the settings children of `<options>`
  travel with them, the `Product` map does not (see `resort.rs`).
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
- **Device settings: one option tree, two game views** (game files and
  in-game tests, SC 4.10, 2026-10-03): "Inversion Settings" and "…
  Sensitivity Curves" are the same `<optiontree>` per kind, its visible
  rows the children of the node `inversion`; BindSight shows one table.
  **Groups inherit**: a group's curve shows on its children, and
  **changing a group's curve removes every descendant's own curve**
  (`exponent` and `<nonlinearity_curve>`; their `invert` stays, an emptied
  element goes); a child set after the group keeps both. Inversion has no
  group settings except `mining`, whose toggle likewise removes the
  descendants' own `invert`. A curve is **either** an `exponent` (row
  slider and dialog slider are one value, 0.1 – 3.0 in 0.1 steps) **or** a
  custom point list: the slider drops the points; points can be added and
  moved (in x too, not necessarily monotonic) but never deleted in the
  game, are written sorted by `in` incl. 0/0 and 1/1, and the game smooths
  them into a curve (how is unknown). The `<options>` children are written
  in tree order, only deviations stored.
- **Device settings: stored ≠ displayed**: joystick deadzone / saturation
  stored = display × 0.99 (0.15 -> `0.1485`, 0.92 -> `0.91079998`; no
  saturation = 1.00), pad deadzone × 0.899 (0.65 -> `0.58434999`), mouse
  acceleration × 0.1, smoothing × 0.9 — stored in a `saturation`
  attribute, under the literal inputs `@pause_OptionsMouseAcceleration` /
  `@pause_OptionsMouseSmoothing` of `<deviceoptions name="Mouse">`; Mouse
  Sensitivity stored = 5 + (display − 1) × 35/99 (12 -> `8.88889`), ADS /
  Zoom Scaling % = display / 100. A missing deadzone has an unknown game
  default: not set, never invented. The joystick axes `x y z rotx roty
  rotz` and the pad's `thumbl` / `thumbr` are not in the game data. **The
  mouse is split across two files**: acceleration / smoothing in
  `actionmaps.xml`, sensitivity, ADS and zoom scaling (and the pad's
  "GamePad Sensitvity", sic) in `attributes.xml`, which holds **every**
  game setting (graphics, audio, …; sorted by name, an entry appears once
  changed) — only the managed names are touched, it is never restored
  whole. The game **appends duplicate `<option>` entries** and later
  rewrites them all; which one it reads is unknown. Mouse acceleration /
  smoothing **read back as 0.00 after a game restart** (a game bug); they
  are written anyway. The game's own profile import **merges**
  `<deviceoptions>` by Product and practically ignores `<options>`;
  BindSight's Apply deliberately replaces both per device. **Unverified**:
  whether the game applies the stored or the displayed deadzone, and
  whether it rescales the travel between deadzone and saturation — the
  Monitor uses the stored value without rescaling; the Axis Test exists to
  find out.
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
