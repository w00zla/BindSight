# BindSight

Star Citizen binding visualizer and mapper. Answers "what does each button
do, and where does each action live?" by joining SC's own config with live
input from your joysticks, gamepad, keyboard and mouse.

- **Monitor**: press a key or button, move an axis, flip a hat — see the
  bound action(s), their category and the SC input token. Image-maps light
  up the control on a picture of your device (bundled: US / DE keyboards,
  full and TKL, Xbox and PlayStation pads; joystick maps you draw yourself).
  Below it, every binding of your connected devices resolved to a label,
  sortable and filterable per device, including the shipped defaults you
  never rebound. Joysticks get their `jsN` from the order the game itself
  logged, never from a guess.
- **Bindings**: the game's keybinding screen as a table, one column per
  device, editable without starting the game — Save writes `actionmaps.xml`
  after a verified backup. Binding profiles (save the live file as one,
  import / export), backups with restore, Apply of a profile or backup per
  device, and a Compare view that diffs a profile or backup against the
  current bindings per input.
- **Device order**: detects the `jsN` shift SC applies when devices enumerate
  in a different order (from `Game.log`) and fixes it — via
  `pp_resortdevices` console commands or by rewriting `actionmaps.xml`.
- **Devices**: an editor for image-maps (rectangles, ellipses, polygons,
  arcs, wedges, arrow / rotation symbols and images per input, own colours),
  with zip export / import, plus Device Info: every fact about your devices
  and a live input-event log, each savable as text.
- **Game-file safety**: every write to a live game file takes a byte-exact
  verified backup first (unless you switch that off), every rewrite is
  re-parsed before it touches the disk, and writes are atomic.

Game data (action list, labels, input token names) is pulled out of your own
install's `Data.p4k` at runtime with the bundled [StarBreaker](https://github.com/diogotr7/StarBreaker)
CLI and cached per game version, so a new SC patch needs no app update. Nothing
leaves your machine.

## Running

Settings holds one folder per SC environment (LIVE, HOTFIX, PTU, EPTU) —
the one containing `Data.p4k`, e.g.
`C:\Program Files\Roberts Space Industries\StarCitizen\LIVE` (the default),
or the equivalent under your Wine prefix. The chip in the top bar switches
which environment the app reads. Per environment you can point the action
labels at your own `global.ini` (e.g. a community translation) instead of
the install's. Everything else is derived from the active folder.

## Troubleshooting

BindSight writes a log (rotated at 2 MB, three files kept) with the
environment, the detected devices, the SC install and everything it loads —
attach it when reporting a problem:

- Linux: `~/.local/share/com.w00zla.bindsight/logs/bindsight.log`
- Windows: `%LOCALAPPDATA%\com.w00zla.bindsight\logs\bindsight.log`
- macOS: `~/Library/Logs/com.w00zla.bindsight/bindsight.log`

For more detail, switch on **Enable debug logging** in Settings → Logging;
**Open log folder** next to it opens the folder.

## Building

Tauri v2 + Rust backend, Vue 3 + TypeScript frontend.

1. Fetch the StarBreaker sidecar binaries (pinned release, SHA256-verified):
   ```sh
   scripts/fetch-starbreaker.sh      # Linux/macOS, or Git Bash on Windows
   scripts\fetch-starbreaker.ps1     # PowerShell
   ```
2. Install the toolchain:
   - **Linux (Fedora/Nobara)**: `rustup`, `pnpm`, and `webkit2gtk4.1-devel
     openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel
     libxdo-devel SDL2-devel` plus the `c-development` group.
   - **Windows**: `rustup` with the MSVC toolchain, Visual Studio Build Tools
     2022 or a newer Visual Studio (2026 works; *Desktop development with
     C++*), CMake on `PATH` (SDL2 is built
     from source and statically linked), `pnpm`, WebView2 runtime.
3. Build and run:
   ```sh
   pnpm install
   pnpm tauri dev       # development
   pnpm tauri build     # release bundle
   ```

`cd src-tauri && cargo test --lib` runs the backend tests; no hardware needed.

## License

MIT — see [LICENSE](LICENSE). StarBreaker is MIT-licensed by its author and
is downloaded, not vendored. Star Citizen and its data are property of Cloud
Imperium Games; this tool reads your local install only.
