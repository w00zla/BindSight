# BindSight — Claude project guide

Star Citizen joystick binding visualizer. Answers "what does each button do, and
where does each action live?" by joining SC's config with live joystick input.
See `HANDOFF.md` for the current working state.

## Conventions

- **English only** — all code, comments, commit messages and GUI text.
- **Commits**: imperative subject, body explaining the why; one commit per
  logical change.
- **Repo content policy**: no SC game data in the repo — the app extracts what
  it needs from the user's install at runtime (see `scinstall.rs`). The
  StarBreaker sidecar binaries (MIT) are not committed either:
  `scripts/fetch-starbreaker.sh` downloads the pinned release into
  `src-tauri/binaries/` (gitignored) and verifies SHA256.

## Stack

- **Shell/backend**: Tauri v2 + Rust (`src-tauri/`).
- **Frontend**: Vue 3 + TypeScript + Vite (`src/`). `App.vue` is the
  orchestrator (all state, invokes, listeners) and composes presentational
  components: `TopBar` (modes **Monitor / Bindings / Devices**; the code's
  `Mode` ids are still `live` / `tools` / `devices`), `DeviceTile`,
  `StatusPanel`, `ImageStage` (+ `Splitter`), `LiveCard`, `BindingsDeck`,
  `ToolsView` (the whole Bindings mode: game bindings, binding profiles,
  backups, Compare), `SettingsDialog`, `Toasts`, `Icon` (inline stroke SVGs
  by name — never emoji). `components/ImageMapEditor.vue` is the image-map
  editor (Konva via `vue-konva`, the three-column Devices mode: devices +
  their image-maps, canvas, live input + areas; also hosts the raw device /
  event log with Save) and uses `ConfirmDialog` for the unsaved-changes and
  delete questions; `components/DeviceImage.vue` the plain-SVG viewer;
  `imagemap.ts` the shared image-map types/helpers (incl. token <-> input
  key), `keyboard.ts` the webview keyboard capture (`KeyboardEvent.code` ->
  SC key name, emits `key` inputs into the same handler as `joy-input`),
  `devices.ts` the shared `deviceName` / `deviceKey` helpers (pads show
  SDL's controller name; remembered GUI state is keyed by hardware id, never
  by SDL's index), `types.ts` all device/input/binding types. **Tables** (bindings deck, Compare) are built
  on `tableColumns.ts` (`useTableColumns`: sort state, widths, grid
  template, localStorage persistence) + `components/ColumnHead.vue`
  (sortable headers, resize grips): the last column is the `1fr` filler,
  the others carry px defaults, every cell truncates with an ellipsis.
  Design tokens live in `src/styles/tokens.css` (the only place colours are
  defined), fonts are bundled under `src/assets/fonts/` (OFL). GUI text is
  terse: one-word states, no explanatory sentences.
- **Input**: SDL2 raw joystick API (`sdl2` crate) for buttons/axes/hats,
  its GameController API for gamepads, the webview for keyboard keys;
  `hidapi` for the HID product string (SC's device name) and later axis usages.
- **XML/INI**: `quick-xml` + hand-rolled parsing.

## Backend modules (`src-tauri/src/`)

- `guid.rs` — convert an SDL joystick GUID to SC's `options/@Product` GUID
  (byte-swap vendor/product); `sdl_guid_vendor_product`.
- `input.rs` — a single background thread owns the one SDL context (rust-sdl2
  allows only one), keeps devices open, emits `joy-input` events, maintains the
  shared device list, emits `devices-changed` on hot-plug. `DeviceInfo` carries
  `kind` (`joystick` / `gamepad` / `keyboard`), `hardware_id` (the image-map
  key: SC Product GUID, `gamepad`, or `keyboard`), `sc_name` (from hidapi) +
  `sdl_name` (debug). A gamepad is what SDL's GameController API recognises;
  it is opened as controller too and its raw `Joy*` events are dropped in
  favour of `padbutton` / `padaxis` events carrying SC's names (`a`,
  `shoulderl`, `thumblx`, …, plus the derived `triggerl_btn` / `thumbl_left`
  … at 50 %). Only the first pad (SDL index order) holds the slot `gp1`,
  further pads are listed with `gamepad_slot: None`. The keyboard is one
  synthetic entry appended last (`sdl_guid` `keyboard`); its keys are captured
  in the webview (`src/keyboard.ts`), never by SDL (no window).
- `scdata.rs` — parse `defaultProfile.xml` (action master list with the
  `joystick=` / `keyboard=` / `gamepad=` defaults, attribute or child-element
  form, child wins), `global.ini` (labels), `keybinding_localization.xml`
  (input token labels: `jsN_`, `kb1_`, `gp1_`; mouse skipped), and the user's
  `actionmaps.xml` (rebinds + `<options>` device map). `DeviceKind` and
  `parse_rebind` (prefix -> kind, kb/gp tokens keep their `+` modifiers) live
  here.
- `scinstall.rs` — the configured install: `validate_install` first
  (`REQUIRED_FILES` = `Data.p4k`, `build_manifest.id`, the live
  `actionmaps.xml`; a missing folder or file aborts the whole load with one
  message, `ScState::invalid_install` then keeps `reload_profile` from
  reading anything), version from `build_manifest.id`
  (`ScVersion`, label `<branch minus sc-alpha->.<P4 changelist>`, e.g.
  `4.10.0-hotfix.12572603`), and the game data (`ScData`: action master list
  + token labels). Runs the StarBreaker sidecar (`p4k extract --regex` for
  `defaultProfile.xml`, `keybinding_localization.xml`, `global.ini`, ~1 s),
  converts via `scdata::parse_*` (unlabeled actions dropped) and caches the
  JSON under `<app_cache_dir>/<label>/`; `scdata.json` carries a `format`
  stamp (`CACHE_FORMAT`), bump it whenever the cached shape changes meaning
  and the cache is re-extracted once instead of loading with silently
  missing fields. Loaded in a background
  thread at start and on environment change (`lib.rs::spawn_sc_load`; with
  an active `global.ini` override the cache is bypassed and the labels come
  from that file): steps via `scdata-progress` (`LOAD_STEPS` = 4), result
  via `scdata-changed`.
- `bindings.rs` — `BindingIndex` (token -> bound actions, all device kinds,
  defaults per kind with a per-kind "touched" rule), `button_token`/
  `hat_token` (with the +1 offset), `instance_for_guid`, `resolve_bindings`,
  `analyze_clash` (saved `<options>` vs. `Game.log` order — the only order
  source, no SDL-derived fallback), `plan_resort` / `resort_commands`.
- `gamelog.rs` — parse SC's `Connected joystickN: <Product {GUID}>` and
  `Connected xinputN: <name>` lines (the latter only says whether SC saw a
  gamepad).
- `hid.rs` — HID report descriptor -> SC axis name per SDL axis index
  (`x y z rotx roty rotz slider1 slider2`); `input.rs` reads the descriptor
  via hidapi per device and stores `DeviceInfo::axes` / `axes_error`.
- `resort.rs` — textual `actionmaps.xml` rewrite applying a resort (joystick
  `<options>` instances + `jsN_` prefixes in `input="..."`), out-of-game
  counterpart of `pp_resortdevices`.
- `config.rs` — persist the SC environments (`ENVIRONMENTS` = LIVE / HOTFIX
  / PTU / EPTU, each a base path + optional `global.ini` override; Windows
  default paths), the active one (`Config::base_path()` /
  `global_ini_override()` read it), the ignore list and the image-map choice
  per device as JSON in the app config dir. `load` fills missing
  environments with defaults; no migration of older shapes.
- `imagemap.rs` — image-maps: one folder per image-map (`imagemap.json` +
  images) under `<app_data_dir>/imagemaps/`, bundled ones under
  `resources/imagemaps/` (bundled are read-only, clone into the user root
  with a fresh id; import assigns a fresh id too). Zip export/import,
  image add/remove/read (data URL), validation. Pure logic takes `&Path`
  roots; the `#[tauri::command]` wrappers only resolve dirs.
- `binding_profiles.rs` — SC's exported keybinding layouts (binding profiles;
  `controls/mappings/*.xml`, same content as `actionmaps.xml`, different
  root): list/import/export only, applying one is not built.
- `backups.rs` — backups of the live `actionmaps.xml`, one folder per backup
  (`meta.json` + `actionmaps.xml`) under `<app_data_dir>/backups/<id>/`, id =
  `YYYYMMDD-HHMMSS` (UTC) with a `-2`, `-3`, … suffix on collision. Taken
  manually, before a resort (`apply_resort` in `lib.rs`), and before a restore.
- `diff.rs` — compares the joystick bindings of two sources (live
  `actionmaps.xml`, a binding profile, or a backup) by SC token: per token, the
  `(actionmap, action)` set bound to it in A vs B (a label-only difference is
  not a change); reports added/removed/changed rows, sorted `jsN` then
  numeric-aware by input.
- `lib.rs` — Tauri commands, state wiring (`AppData`: config, the install's
  game data + load status, profile, binding index, Game.log snapshot), the
  input thread spawn, the SC data loader thread, the Wayland DMABUF
  workaround, logging setup.
- **Logging**: the `log` crate everywhere (never `println!`/`eprintln!`),
  `tauri-plugin-log` writes to stdout and `<app_log_dir>/bindsight.log`
  (2 MB, 3 files kept; Linux `~/.local/share/com.w00zla.bindsight/logs/`).
  Our crate logs down to DEBUG, dependencies from WARN. The webview console
  (`console.*`, uncaught errors, unhandled rejections) is forwarded by
  `src/logging.ts` via the plugin's `log` command with the plain `webview`
  target (the JS package would tag a source location the level filter
  cannot match), so frontend messages land in the same file. Severities: ERROR =
  a feature is broken (config not saved, SC data failed, input thread died,
  panic), WARN = degraded but running (profile not loaded, Game.log missing,
  unreadable cache), INFO = state changes and facts (startup environment, SC
  version, extraction, profile/Game.log contents, user actions), DEBUG =
  detail (command lines, load steps). **Device runtime detail never goes to
  the app log**: no enumeration lines, no axis
  derivation results, no input events — only real failures (hidapi init,
  joystick open, thread death). All of it lives in the Devices mode's
  device log instead (`DeviceInfo` carries every SDL/HID fact, events carry
  SDL timestamp + instance id).

## Image-map data model (`imagemap.json`, format 3)

- Keyed by `hardware_id` = SC Product GUID (vendor/product, platform-stable)
  for joysticks, the literal `gamepad` for gamepads (SC treats every pad as
  the same XInput device) and `keyboard` for the keyboard; several image-maps
  per id are normal (told apart by `name`).
- **Exactly one image per image-map** (`image: {file, label}`, mandatory — an
  image-map is created around its image file, areas belong to it implicitly).
  Format 1 (`images[]`, areas tied to an image id) is not read.
- Areas map an input key to a shape on the image. Joysticks use **SDL-level**
  keys (`button:N`, `hat:N:<dir>`, `axis:N`, no axis sign — SC has none);
  keyboard and gamepad use SC's own names (`key:lshift`, `key:oem_102`,
  `pad:a`, `pad:thumblx`, `pad:triggerl_btn`). Shapes: `rect`, `ellipse`,
  `polygon`, or `symbol` (`arrow`, `cw`, `ccw`: the 100x100 path stretched
  into a `w` x `h` box, rotatable). Several areas per input are fine.
- All coordinates are normalized 0..1 to the image's natural size, rotation
  in degrees around the shape's center. The model is ours, never Konva's JSON —
  the canvas lib is only the editor's interaction layer.
- Token -> input key undoes the +1 offset (`js2_button5` -> `button:4`) and
  maps axes through `DeviceInfo::axes`; `kb1_lalt+x` -> `key:x`, `gp1_a` ->
  `pad:a` (a combo pins the part after the last `+`).
- **Bundled image-maps** (`src-tauri/resources/imagemaps/`): Keyboard US,
  Keyboard DE, Xbox controller, PlayStation controller — generated, never
  hand-edited: `scripts/gen-imagemaps.py` holds the geometry once and writes
  both `image.png` and `imagemap.json` (fixed ids `4b7a2c1e-…-000000000001`
  to `…0004`). `App.vue`'s `DEFAULT_MAPS` picks the US keyboard and the Xbox
  pad by default (PlayStation for Sony's vendor id); without a match the
  first map wins.

## Commands / how to work

```sh
# Run the app (needs a display)
pnpm tauri dev

# Frontend typecheck + build
pnpm build

# Backend tests (fast, no hardware needed)
cd src-tauri && cargo test --lib

# Diagnostics (headless)
cd src-tauri && cargo run --example enum_joysticks       # list devices + GUIDs
cargo run --example log_joystick_events                  # live event log
cargo run --example hid_names                            # HID product strings
cargo run --example hid_axes [-- --hex]                  # HID axis usages + derived SC axes

# Fetch the StarBreaker sidecar binaries (once, and after bumping the pinned version)
scripts/fetch-starbreaker.sh
```

The SC game data is extracted from the configured install at runtime and
cached per game version (`~/.cache/com.w00zla.bindsight/<label>/` on
Linux); delete that dir to force a re-extract.

## Prerequisites (all platforms)

The StarBreaker sidecar binaries in `src-tauri/binaries/` — run
`scripts/fetch-starbreaker.sh` (needs `curl`, `tar`, `unzip`) or
`scripts/fetch-starbreaker.ps1` (PowerShell); keep version + hashes in both
in sync. Without them every `cargo build` fails in the Tauri build script
(`resource path binaries/starbreaker-<triple> doesn't exist`).

## Prerequisites (Fedora/Nobara)

`rustup`, `pnpm`, and system deps: `webkit2gtk4.1-devel openssl-devel curl wget
file libappindicator-gtk3-devel librsvg2-devel libxdo-devel SDL2-devel`, plus the
`c-development` group.

## Prerequisites (Windows)

- **`rustup` with the MSVC toolchain** (`stable-x86_64-pc-windows-msvc`) — not GNU;
  the VC libs won't link against a `-gnu` target.
- **Visual Studio Build Tools 2022**, workload *Desktop development with C++*
  (MSVC + Windows SDK) — provides the linker plus `hid`/`setupapi` that `hidapi`
  links against.
- **CMake** on `PATH` — SDL2 is built from source and static-linked on Windows
  (see the `sdl2` dep in `src-tauri/Cargo.toml`), so no `SDL2.dll` is shipped and
  no system SDL2 is needed. Without CMake the SDL2 build script aborts. Either
  `winget install Kitware.CMake`, or reuse the copy the C++ workload already
  installs — that one is *not* on the `PATH`, so append
  `<VS>\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin` yourself.
- **`pnpm`** and the WebView2 runtime (preinstalled on Win10/11) for the Tauri app.

## Gotchas (learned the hard way)

- **SC label resolution** (`global.ini` is the only source): match keys
  **case-insensitively** (`@ui_CIboost` vs `ui_CIBoost`) and alias the **`,P`**
  (PC platform) suffix to the bare key. ~350 actions are genuinely unlabeled
  (SC-internal) — hidden in SC too. Actions without a label are dropped from the
  bundle (a label is mandatory, description optional).
- **Input token labels are per-instance**: `js1_button1` = "Button 1", but
  `js2_button1` = "Button 1 (Input 2)". Axis/hat keys differ from the bind token
  (`x`->`xAxis`, `rotz`->`zRotation`, `hat1_up`->`hat1Up`), so use
  `keybinding_localization.xml` as the token->key map, not string surgery.
- **actionmaps.xml lives at** `<base>/user/client/0/Profiles/default/` — NOT
  `controls/mappings/` (that holds only exported layouts). The base path is the
  parent of `Data.p4k` (e.g. `.../StarCitizen/LIVE`).
- **Device identity is GUID, never name**: SDL's name (evdev) differs from SC's
  (HID product string). Match devices by vendor/product GUID only.
- **SDL order is not SC order** (Windows: reversed, Linux: unrelated). `jsN`
  comes from `Game.log` only; never derive it from SDL's enumeration.
- **+1 button offset**: SC `js_button1` == SDL button 0 (verified under Wine).
- **SC knows exactly one keyboard (`kb1`) and one gamepad (`gp1`)**: no
  `kb2_`/`gp2_` anywhere, no instance logic, no order clash. Key names are
  DirectInput scancode names (physical, layout independent): on a German
  keyboard the cap labelled `Y` is `kb1_z`. `KeyboardEvent.code` is physical
  too, hence the static table in `keyboard.ts`. `kb1_` also carries mouse
  buttons/wheel (`kb1_mouse1`, `kb1_mwheel_up`); mouse is not supported.
- **Keyboard capture needs the BindSight window focused** (webview keydown;
  SDL2 delivers key events only to its own window). It runs in every mode
  and `preventDefault`s every mapped key (deliberate: a rebind flow must own
  the keyboard); only text fields and open dialogs (`[role="dialog"]`) are
  skipped. PrintScreen / Meta never arrive.
- **Axes**: SC names axes by HID usage (X->`x` … Rz->`rotz`, Slider/Dial->
  `slider1`/`slider2`); SDL numbers them in canonical usage order (Linux:
  evdev ABS code order, Windows: DirectInput offset order), NOT report order.
  Verified on a VKB EVO whose report order is X Y Rz Z Rx Ry: SDL 5 = twist =
  `rotz`, SDL 2 = `z`. `hid.rs` derives it and cross-checks the count against
  SDL; anything it cannot place is an `axes_error`, never a guess.
- **Wayland**: `run()` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` on Linux (fixes
  WebKitGTK "Error 71"), unless the user overrode it.
- **Dark mode + native controls**: `:root { color-scheme: dark }` in the dark
  media block, or WebKitGTK paints `<select>` popups white with inherited white
  text.
- **Konva rect rotation**: rect nodes sit at their center with
  `offsetX/Y = half size`, so `rotation` turns around the center and the stored
  `x/y` stay the unrotated top-left. On `transformend` read `width*scaleX`,
  reset scale to 1 imperatively (vue-konva diffs configs and would not re-apply
  an unchanged `scaleX: 1`).
- **CMake 4 vs. vendored SDL2**: `sdl2-sys` ships an SDL2 whose `CMakeLists.txt`
  still says `cmake_minimum_required(VERSION 3.0)`, which CMake 4.x refuses
  ("Compatibility with CMake < 3.5 has been removed"). VS 2026 ships CMake 4.3,
  so the build dies in the `sdl2-sys` build script. `src-tauri/.cargo/config.toml`
  sets `CMAKE_POLICY_VERSION_MINIMUM=3.5` to work around it; drop that once
  `sdl2-sys` vendors an SDL2 requiring >= 3.5.
