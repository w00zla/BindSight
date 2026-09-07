# BindSight — Concept

**Star Citizen Joystick Binding Visualizer & Mapper.** Answers: "What does each button do, and where does each action live?" Live input highlight ↔ binding list, linked both ways. Runs on Windows and Linux (primary environment: Linux + SC under Wine/Proton).

---

## Decisions (locked in)

| Topic | Decision |
|---|---|
| Shell | **Tauri** (not Electron) |
| Backend | **Rust** |
| Frontend | Web (Vue can be reused from the prototype) |
| Input source | **SDL2 raw joystick API** via the `sdl2` crate — *not* the GameController abstraction, *not* `gilrs` |
| HID usages | `hidapi` crate (for deterministic axis mapping) |
| XML | `quick-xml` + `serde` |
| Visualization | **2D first** (image/SVG + hotspot overlay), 3D optional later (three.js in the same frontend) |
| Profile identity | **GUID-based** (stable), not order-based |

**Why Tauri instead of Electron:** Electron would be faster to a first running build (the prototype is already Vue), but Tauri was chosen for the leaner/more robust artifact **and** the explicit goal of "learning Rust". The extra cost (porting the XML pipeline to Rust, Tauri IPC) is manageable; the input layer is new in either variant anyway.

---

## The core problem (why the prototype won't work as-is)

The prototype reads input via the **browser Gamepad API** (`navigator.getGamepads()`). Its button/axis indices are **not guaranteed to match** SC's DirectInput numbering (`js1_buttonN`) — DirectInput mandates no button order, and Chromium enumerates HID devices through its own path. On top of that, the 32-button limit of the Gamepad API (VKB/Virpil blow past it). This makes the central "physical button → SC action" mapping unreliable — a total loss for a tool whose sole purpose is exactly that.

**Solution:** the SDL raw joystick API delivers DirectInput-native indices. On Windows, SDL *and* SC both use DirectInput → indices match 1:1.

---

## SC input model (important for mapping)

SC only knows a **fixed DirectInput vocabulary**, identical across devices:
- **Axes:** `x, y, z, rotx, roty, rotz, slider1, slider2` (HID usages 0x30–0x37)
- **Hats:** `hat1..4` with `_up/_right/_down/_left` (HID usage 0x39)
- **Buttons:** `button1..128`
- **Modifiers:** e.g. `lctrl+js1_button1`

**Which physical control lands in which slot is decided by the device via its HID usages** — not by SC. DirectInput slots axes deterministically by usage (X→`js_x`, Rz→`js_rotz`, Slider→`js_slider1` …).

**Firmware wildcard (VKB/Virpil):** the same physical control can appear differently depending on firmware — a hat as a true POV (`hat1_up`) *or* as 4 discrete buttons. Encoders/rotaries: axis *or* buttons. → **Even the same model is not guaranteed to be identical.** That is why calibration ("learning") is the source of truth, not a hardcoded per-model table.

---

## Data model

**Device profile** (per device, GUID-based): a mapping table with three columns

```
SDL index (button N / axis M / hat H)  →  SC token (js_button7 / js_x / js_hat1_up)  →  image hotspot (view, x, y)
```

- **Vocabulary** = universal (no per-device code)
- **Images** = new per stick type (they only supply the hotspot column)
- **SDL→SC column** = the actual learning core, per device *and* firmware profile

**SC config sources (reusable from the prototype):**
- `defaultProfile.xml` → all actions (master list)
- `actionmaps.xml` (`USER/Client/0/Profiles/default/`) → user rebinds, overlaid
- `global.ini` + `keybindings_localization.xml` → human-readable labels
- `<options>` block in actionmaps.xml → **`instance` ↔ `Product` GUID** (basis for the remap feature)

---

## Learn flow ("learn a device")

1. SDL opens the device → GUID + enumeration order known; match against SC `options/@Product`.
2. **Axes:** automatable — read each axis's HID usage via `hidapi` → replicate DirectInput slotting → `js_x/rotz/slider1` falls out. Calibration only as verification.
3. **Buttons:** `button i → button(i+1)`; verify once by pressing (because of Wine ordering, see below).
4. **Hats:** index → `hat(H+1)`, directions from the POV value. If firmware exposes the hat as buttons → it shows up as buttons, and the flow detects that automatically.
5. Result = profile JSON (GUID + mapping + optional hotspots).

No extra GUI for now; in-app dev tools are enough.

---

## Cross-platform / Wine pitfall (the spot where it blows up)

SC runs under Wine/Proton with its **own dinput stack** (maps evdev→DirectInput). Our tool reads **natively** via SDL/evdev on Linux. Whether **Wine's** button order == **native SDL** order is **not guaranteed** — even if the sticks "run perfectly in SC" (that only means Wine is internally consistent).

→ **Do not blindly assume `sdl_button i == js_button(i+1)`.** Empirically checkable in 30s; the learn flow makes the verification a feature. On Windows (both DirectInput) it matches 1:1. The `pp_` console commands are engine-internal → they work the same under Wine.

---

## Instance remap feature (wanted, later)

**Problem:** SC binds to order (`js1/js2/…`), assigned by OS enumeration. Re-plug/reboot → `js1` becomes `js3`, bindings point nowhere.

**Why we're predestined for it:** we have both sides — SDL delivers live GUID + order, `actionmaps.xml` delivers GUID↔instance. Diff them → on mismatch generate a corrected `actionmaps.xml` (rewrite `jsX_` prefixes + `instance`) → user loads it via `pp_rebindkeys <file>`.

**Hard design rule:** `actionmaps.xml` is live game config = quasi PROD. **Never write silently** — first back up, write to a copy, show it, user decides/loads.

**Pitfall:** the `Product` GUID (`{0201231D-…-504944564944}` = product+vendor+fixed "PIDVID") is **not per-device unique**. Two identical sticks collide → fall back to order + manual assignment. (VKB L/R have different product IDs, so that is fine.)

---

## Reusable from the prototype

- SC data pipeline (`convert-scdata.js`, defaultProfile/localization → JSON) — logic is good, but the **data was from Dec 2023**; re-export for the current SC version (4.x).
- Data models & parsing logic in `sc-config.ts` (actions, rebinds, joystick options/GUID).
- Vue frontend structure (Quasar optional).
- Profile JSONs (vendor/product schema).

**Known prototype TODOs:** cleanly handle multi-binding per action + modifier (`lctrl+js1_button1`); axis deadzones (`deviceoptions` in the XML).

---

## Open items / next steps

- [ ] New git folder + Tauri scaffold (Rust backend, Vue frontend)
- [ ] SDL joystick read as the first runnable thing (log buttons/axes/hats live) — also confirms the Wine index question
- [ ] Port the SC XML pipeline to Rust
- [ ] Re-export current SC data (4.x)
- [ ] 2D image + hotspot overlay (start with VKB Gladiator EVO as reference)
- [ ] Learn flow (dev tool)
- [ ] Instance remap feature (after core)

**IDE recommendation:** RustRover (JetBrains, free non-commercial, best hand-holding for learning) or VS Code + rust-analyzer.
