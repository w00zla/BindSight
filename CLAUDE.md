# BindSight — Claude project guide

Star Citizen joystick binding visualizer. Answers "what does each button do, and
where does each action live?" by joining SC's config with live joystick input.
`.claude/HANDOFF.md` (gitignored, local) holds the current working state.

## Conventions

- **English only** — all code, comments, commit messages and GUI text.
- **GUI text speaks the player's language**: no file names (`actionmaps.xml`,
  `Game.log`, …) or other internals in labels, titles, toasts and dialogs.
  Only where an element is specifically about that file (the `global.ini`
  override, a missing-/broken-file error detail). Say "game", never "SC"
  ("Star Citizen" is fine where it reads better). Terse: one-word states,
  explanatory sentences only where a dialog guides an action.
- **Commits**: imperative subject, body explaining the why; one commit per
  logical change.
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

- `TopBar` — modes **Monitor / Bindings / Devices** (code `Mode` ids are
  still `live` / `tools` / `devices`), environment chip + dropdown, version
  chip, Refresh, gear. It is also the title bar (see the undecorated-window
  gotcha).
- `StartupTile` — covers every mode while the first game-data load after
  start runs, whatever its outcome.
- Monitor: `DeviceTile`, `StatusPanel`, `ImageStage` (+ `Splitter`),
  `LiveCard`, `BindingsDeck`.
- `ToolsView` — the whole Bindings mode: Game Bindings (the live file as
  the one item "Current"), binding profiles, backups, and on the right
  either the Bindings List (Current: the game's keybinding screen as a
  table, one toggleable column per device the file names, categories
  collapsible, double-click = rebind dialog, pending rebinds kept until
  Save / Discard in the action tile above it) or Compare (any other source
  picked on the left). Filter chips persist via `persist.ts`.
- `ImageMapEditor` — the Devices mode (Konva via `vue-konva`, three columns:
  devices + image-maps, canvas, live input + areas); also hosts Device Info
  (Device List and Device Events tiles, each with its own Save).
  `DeviceImage.vue` is the plain-SVG viewer.
- `SettingsDialog`, `ConfirmDialog` (title, required icon, buttons, optional
  body slot — behind every unsaved-changes / delete / game-file-write
  question and the Fix via config / Fix via console dialogs), `Toasts`,
  `Icon` (inline stroke SVGs by name — never emoji), `WindowEdges`.
- Shared modules: `imagemap.ts` (image-map types/helpers incl. token <->
  input key), `keyboard.ts` (webview keyboard capture, `KeyboardEvent.code`
  -> SC key name, feeds the same handler as `joy-input`), `devices.ts`
  (`deviceName` / `deviceKey` / `deviceIcon`; remembered GUI state is keyed
  by hardware id, never by SDL's index), `types.ts` (all device/input/
  binding types), `logging.ts` (console forwarding, see Logging).
- **Tables** (bindings deck, Compare): `tableColumns.ts` (`useTableColumns`:
  sort state, widths, grid template, localStorage persistence) +
  `ColumnHead.vue` (sortable headers, resize grips). The last column is the
  `1fr` filler, the others carry px defaults, every cell truncates with an
  ellipsis.
- Design tokens in `src/styles/tokens.css` (the only place colours are
  defined), fonts bundled under `src/assets/fonts/` (OFL). The name is
  two-tone: BIND in `--text`, SIGHT in `--accent`.

## Backend modules (`src-tauri/src/`)

- `input.rs` — one background thread owns the single SDL context, keeps
  devices open, emits `joy-input`, maintains the shared device list, emits
  `devices-changed` on hot-plug. `DeviceInfo` carries `kind` (`joystick` /
  `gamepad` / `keyboard`), `hardware_id` (image-map key: SC Product GUID,
  `gamepad`, or `keyboard`), `sc_name` (hidapi) + `sdl_name` (debug), `axes`
  / `axes_error`. A gamepad is what SDL's GameController API recognises; its
  raw `Joy*` events are dropped in favour of `padbutton` / `padaxis` with
  SC's names (`a`, `shoulderl`, `thumblx`, … plus the derived `triggerl_btn`
  / `thumbl_left` … at 50 %). Only the first pad (SDL index order) holds the
  slot `gp1`, further pads get `gamepad_slot: None`. The keyboard is one
  synthetic entry appended last (`sdl_guid` `keyboard`).
- `guid.rs` — SDL joystick GUID -> SC `options/@Product` GUID (byte-swap
  vendor/product); `sdl_guid_vendor_product`.
- `hid.rs` — HID report descriptor -> SC axis name per SDL axis index
  (`x y z rotx roty rotz slider1 slider2`).
- `scdata.rs` — parse `defaultProfile.xml` (action master list with the
  `joystick=` / `keyboard=` / `gamepad=` defaults, attribute or child-element
  form, child wins), `global.ini` (labels), `keybinding_localization.xml`
  (input token labels `jsN_` / `kb1_` / `gp1_`; mouse skipped), and the
  user's `actionmaps.xml` (rebinds + `<options>` device map). `DeviceKind`
  and `parse_rebind` (prefix -> kind; kb/gp tokens keep their `+` modifiers).
- `scinstall.rs` — the configured install. `validate_install` first
  (`REQUIRED_FILES` = `Data.p4k`, `build_manifest.id`, the live
  `actionmaps.xml`; anything missing aborts the whole load with one message
  and `ScState::invalid_install` keeps `reload_profile` from reading
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
  `hat_token` (+1 offset), `instance_for_guid`, `resolve_bindings`,
  `analyze_clash` (saved `<options>` vs. `Game.log` order — the only order
  source), `plan_resort` / `resort_commands`.
- `gamelog.rs` — parse `Connected joystickN: <Product {GUID}>` and
  `Connected xinputN: <name>` (the latter only says SC saw a gamepad).
- `resort.rs` — textual `actionmaps.xml` rewrite applying a resort (joystick
  `<options>` instances + `jsN_` prefixes in `input="..."`), the out-of-game
  counterpart of `pp_resortdevices`.
- `rebind.rs` — textual `actionmaps.xml` rewrite writing rebinds (one
  binding per action and device kind, like SC: every `<rebind>` of that kind
  under the action is replaced; missing `<action>` / `<actionmap>` elements
  are created in SC's layout), the out-of-game counterpart of the keybinding
  screen. `lib.rs::save_rebinds` backs up first (reason "before rebind").
- `config.rs` — JSON in the app config dir: the SC environments
  (`ENVIRONMENTS` = LIVE / HOTFIX / PTU / EPTU, each a base path + optional
  `global.ini` override; Windows default paths), the active one
  (`Config::base_path()` / `global_ini_override()`), the ignore list, the
  image-map choice per device, the auto-backup and debug-logging switches.
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
  root): list/import/export only; applying one is not built.
- `backups.rs` — backups of the live `actionmaps.xml`, one folder per backup
  (`meta.json` with reason + game version, `actionmaps.xml`) under
  `<app_data_dir>/backups/<id>/`, id = `YYYYMMDD-HHMMSS` (UTC) with a `-2`,
  `-3`, … suffix on collision. Taken manually, and while `Config::auto_backup`
  is on (default) before a resort and before a restore.
- `diff.rs` — compares the joystick bindings of two sources (live file,
  binding profile, or backup) by SC token: the `(actionmap, action)` set per
  token in A vs B (label-only differences are not changes); added / removed /
  changed rows, sorted `jsN` then numeric-aware by input.
- `kblayout.rs` — `keyboard_layout` command: xkb code (`de`, `us`, …) via
  `localectl` / `vconsole.conf` on Linux, `GetKeyboardLayoutNameW` on Windows.
- `lib.rs` — Tauri commands, state wiring (`AppData`: config, game data +
  load status, profile, binding index, Game.log snapshot), thread spawns, the
  Wayland DMABUF workaround, logging setup.

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
drops webview debug records itself (`setDebugLogging`).

Severities: ERROR = a feature is broken (config not saved, SC data failed,
input thread died, panic); WARN = degraded but running (profile not loaded,
Game.log missing, unreadable cache); INFO = state changes and facts (startup
environment, SC version, extraction, profile / Game.log contents, user
actions); DEBUG = detail (command lines, load steps). **Device runtime detail
never goes to the app log** — no enumeration lines, no axis derivation, no
input events, only real failures (hidapi init, joystick open, thread death).
All of it lives in the Devices mode's Device Info view instead.

## Image-map data model (`imagemap.json`, format 3)

- Keyed by `hardware_id`: SC Product GUID (vendor/product, platform-stable)
  for joysticks, the literal `gamepad` for gamepads (SC treats every pad as
  the same XInput device), `keyboard` for the keyboard. Several image-maps
  per id are normal (told apart by `name`).
- **Exactly one image per image-map** (`image: {file, label}`, mandatory —
  an image-map is created around its image file). Format 1 (`images[]`) is
  not read.
- Areas map an input key to a shape. Joysticks use **SDL-level** keys
  (`button:N`, `hat:N:<dir>`, `axis:N`, no axis sign); keyboard and gamepad
  use SC's own names (`key:lshift`, `key:oem_102`, `pad:a`, `pad:thumblx`,
  `pad:triggerl_btn`). Shapes: `rect`, `ellipse`, `polygon`, `symbol`
  (`arrow`, `cw`, `ccw`: a 100x100 path stretched into a `w` x `h` box,
  rotatable). Several areas per input are fine.
- Coordinates are normalized 0..1 to the image's natural size, rotation in
  degrees around the shape's center. The model is ours, never Konva's JSON.
- Token -> input key undoes the +1 offset (`js2_button5` -> `button:4`) and
  maps axes through `DeviceInfo::axes`; `kb1_lalt+x` -> `key:x`, `gp1_a` ->
  `pad:a` (a combo pins the part after the last `+`).
- **Bundled image-maps** (`src-tauri/resources/imagemaps/`): Keyboard US,
  Keyboard DE, Xbox controller, PlayStation controller — generated, never
  hand-edited: `scripts/gen-imagemaps.py` holds the geometry and writes
  `image.png` + `imagemap.json` (fixed ids `4b7a2c1e-…-000000000001` to
  `…0004`).
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
- **SDL order is not SC order** (Windows: reversed, Linux: unrelated). `jsN`
  comes from `Game.log` only; never derive it from SDL's enumeration.
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
  too, hence the static table in `keyboard.ts`. `kb1_` also carries mouse
  buttons/wheel; mouse is not supported.
- **Keyboard capture needs the BindSight window focused** (webview keydown;
  SDL2 delivers key events only to its own window). It runs in every mode
  and `preventDefault`s every mapped key (deliberate: a rebind flow must own
  the keyboard); only text fields and open dialogs (`[role="dialog"]`) are
  skipped — except a dialog carrying `data-capture-keys` (`ConfirmDialog`
  `captureKeys`, the rebind dialog). PrintScreen / Meta never arrive.
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
