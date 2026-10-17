# Identical joystick models

SDL and Star Citizen Product GUIDs identify models, not individual units.
For identical models, verify each unit's jsN in Star Citizen and select it
on its Input Devices tile. This changes only BindSight's local GUI state.
Until selected, that unit's inputs do not resolve to bindings. Selecting an
occupied slot clears the other unit's assignment. Unique models resolve automatically.

Live input uses SDL instance IDs. Slot choices use OS device paths, or unique
serials when no path exists; missing or shared identities allow session-only
choices. Choices are scoped to the game's slot map and connected units of that
model. Changed sets or maps require verification; returning to the same set
restores its choices. A model known to have duplicates keeps requiring selection
when only one unit remains. Image-map compatibility remains model-based, but
each unit remembers its own selected map. Existing model-level map choices
remain fallbacks until a unit-specific map is selected.

Paths can change after driver, USB port, or Linux enumeration changes.
Product-only game records cannot reveal a swap between identical units, so
recheck choices after changing controller configuration or game order.
Automatic order fixes are disabled for duplicate models. HID axis lookup uses
the exact SDL collection path, or reports ambiguity when several interfaces
share a VID/PID and no path matches.

Run pnpm test, pnpm build, and, in src-tauri, cargo test --lib.
Hardware verification should check each unit's game slot; simultaneous presses
and releases of the same button number; axes and hats; image-map and binding
recording; app restart; and reconnect/enumeration changes. Cancel recording
dialogs to leave game bindings untouched.
