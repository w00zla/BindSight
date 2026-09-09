# BindSight — Claude project guide

Star Citizen joystick binding visualizer. Answers "what does each button do, and
where does each action live?" by joining SC's config with live joystick input.
See `HANDOFF.md` for the current working state.

## Conventions

- **English only** — all code, comments, commit messages and GUI text. (Chat with
  the user is German; the code is not.)
- **Commits**: imperative subject, body explaining the why. Commit only when the
  user asks.
- **Repo content policy**: `starbreaker` and other third-party tools are never
  committed (license). Extracted SC data (`data/extracted/`) is gitignored
  (staleness), but the *derived* `scdata.json` / `tokens.json` resources are
  committed (SC's localization strings are freely usable; regenerate per patch).

## Stack

- **Shell/backend**: Tauri v2 + Rust (`src-tauri/`).
- **Frontend**: Vue 3 + TypeScript + Vite (`src/`). `App.vue` is the
  orchestrator (all state, invokes, listeners) and composes presentational
  components: `TopBar`, `DeviceTile`, `StatusPanel`, `ImageStage` (+
  `Splitter`), `LiveCard`, `BindingsDeck`, `SettingsDialog`, `Toasts`, `Icon`
  (inline stroke SVGs by name — never emoji). `components/ProfileEditor.vue`
  is the image-map editor (Konva via `vue-konva`, still the pre-redesign UI),
  `components/DeviceImage.vue` the plain-SVG viewer; `hwprofile.ts` the
  shared image-map types/helpers, `types.ts` all device/input/binding types.
  Design tokens live in `src/styles/tokens.css` (the only place colours are
  defined), fonts are bundled under `src/assets/fonts/` (OFL). GUI text is
  terse: one-word states, no explanatory sentences.
- **Input**: SDL2 raw joystick API (`sdl2` crate) for buttons/axes/hats;
  `hidapi` for the HID product string (SC's device name) and later axis usages.
- **XML/INI**: `quick-xml` + hand-rolled parsing.

## Backend modules (`src-tauri/src/`)

- `guid.rs` — convert an SDL joystick GUID to SC's `options/@Product` GUID
  (byte-swap vendor/product); `sdl_guid_vendor_product`.
- `input.rs` — a single background thread owns the one SDL context (rust-sdl2
  allows only one), keeps devices open, emits `joy-input` events, maintains the
  shared device list, emits `devices-changed` on hot-plug. `DeviceInfo` carries
  `sc_name` (from hidapi) + `sdl_name` (debug).
- `scdata.rs` — parse `defaultProfile.xml` (action master list), `global.ini`
  (labels), `keybinding_localization.xml` (input token labels), and the user's
  `actionmaps.xml` (rebinds + `<options>` device map).
- `bindings.rs` — `BindingIndex` (token -> bound actions), `button_token`/
  `hat_token` (with the +1 offset), `instance_for_guid`, `resolve_bindings`,
  `analyze_clash` (saved `<options>` vs. `Game.log` order — the only order
  source, no SDL-derived fallback), `plan_resort` / `resort_commands`.
- `gamelog.rs` — parse SC's `Connected joystickN: <Product {GUID}>` lines.
- `hid.rs` — HID report descriptor -> SC axis name per SDL axis index
  (`x y z rotx roty rotz slider1 slider2`); `input.rs` reads the descriptor
  via hidapi per device and stores `DeviceInfo::axes` / `axes_error`.
- `resort.rs` — textual `actionmaps.xml` rewrite applying a resort (joystick
  `<options>` instances + `jsN_` prefixes in `input="..."`), out-of-game
  counterpart of `pp_resortdevices`.
- `config.rs` — persist the SC base path, the ignore list and the HW profile
  choice per device as JSON in the app config dir.
- `hwprofile.rs` — HW profiles: one folder per profile (`profile.json` +
  images) under `<app_data_dir>/profiles/`, bundled ones under
  `resources/profiles/` (user shadows bundled by id, editing forks). Zip
  export/import, image add/remove/read (data URL), validation. Pure logic
  takes `&Path` roots; the `#[tauri::command]` wrappers only resolve dirs.
- `lib.rs` — Tauri commands, state wiring, the input thread spawn, the Wayland
  DMABUF workaround.

## HW profile data model (`profile.json`, format 3)

- Keyed by `hardware_id` = SC Product GUID (vendor/product, platform-stable);
  several profiles per id are normal (told apart by `name`).
- **Exactly one image per profile** (`image: {file, label}`, mandatory — a
  profile is created around its image file, areas belong to it implicitly).
  Format 1 (`images[]`, areas tied to an image id) is not read.
- Areas map an **SDL-level** input key (`button:N`, `hat:N:<dir>`, `axis:N`,
  no axis sign — SC has none) to a shape on the image: `rect`, `ellipse`,
  `polygon`, or `symbol` (`arrow`, `cw`, `ccw`: the 100x100 path stretched
  into a `w` x `h` box, rotatable). Several areas per input are fine.
- All coordinates are normalized 0..1 to the image's natural size, rotation
  in degrees around the shape's center. The model is ours, never Konva's JSON —
  the canvas lib is only the editor's interaction layer.
- Token -> input key undoes the +1 offset (`js2_button5` -> `button:4`); axes
  have no mapping yet.

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

# Regenerate the bundled SC data (after an SC patch / re-extract)
cd src-tauri
cargo run --example convert_scdata -- ../data/extracted/defaultProfile.xml ../data/extracted/global.ini resources/scdata.json
cargo run --example convert_tokens -- ../data/extracted/keybinding_localization.xml ../data/extracted/global.ini resources/tokens.json

# Re-extract raw SC data from the game (needs the starbreaker binary in data/)
cd data && ./extract_sc_datafiles.sh <path/to/Data.p4k>
```

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
