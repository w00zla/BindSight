import { readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { test } from 'node:test';
import ts from 'typescript';

// These helpers have no runtime imports. Compile them with the project's
// TypeScript, so the regression suite needs no additional test dependencies.
const source = await readFile(new URL('../src/devices.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
const { deviceKey, liveDeviceKey, eventDeviceKey, inputIdentity, joystickSlot, assignJoystickSlot, mapChoice } =
  await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

const device = (id, path, hats = 1) => ({
  kind: 'joystick', sdl_guid: 'same-guid', sc_product_guid: 'PRODUCT',
  sdl_instance_id: id, index: id, sdl_path: path, sdl_serial: null, num_hats: hats,
});
const a = device(10, 'unit-one', 4), b = device(11, 'unit-two');
const devices = [a, b];
const slots = [1, 2].map(effective_instance => ({ effective_instance, sc_product_guid: 'product' }));
const event = (d, pressed) => ({ guid: d.sdl_guid, instance_id: d.sdl_instance_id, kind: 'button', index: 0, pressed });

test('same-model input events and release tracking stay on their own unit', () => {
  assert.notEqual(liveDeviceKey(a), liveDeviceKey(b));
  assert.equal(liveDeviceKey(a), eventDeviceKey(event(a, true)));
  assert.equal(inputIdentity(event(a, true)), inputIdentity(event(a, false)));
  assert.notEqual(inputIdentity(event(a, true)), inputIdentity(event(b, false)));
});

test('persistent unit identity survives index changes and ignores capabilities', () => {
  assert.notEqual(deviceKey(a), deviceKey(b));
  assert.equal(deviceKey(a), deviceKey({ ...a, index: 0, sdl_instance_id: 99, num_hats: 1 }));
  assert.notEqual(deviceKey(a), deviceKey({ ...a, sdl_path: 'new-path' }));
});

test('duplicate product GUIDs require explicit one-to-one slot choices', () => {
  assert.equal(joystickSlot(a, devices, slots, {}), null);
  let choices = assignJoystickSlot(a, devices, slots, {}, 2);
  choices = assignJoystickSlot(b, devices, slots, choices, 1);
  assert.equal(joystickSlot(a, devices, slots, choices)?.effective_instance, 2);
  assert.equal(joystickSlot(b, devices, slots, choices)?.effective_instance, 1);
  choices = assignJoystickSlot(b, devices, slots, choices, 2);
  assert.equal(joystickSlot(a, devices, slots, choices), null);
  assert.equal(joystickSlot(b, devices, slots, choices)?.effective_instance, 2);
});

test('choices follow unit paths across restarts and reordered enumeration', () => {
  const choices = assignJoystickSlot(a, devices, slots, {}, 2);
  const restartedA = { ...a, index: 1, sdl_instance_id: 101 };
  const restartedB = { ...b, index: 0, sdl_instance_id: 100 };
  assert.equal(joystickSlot(restartedA, [restartedB, restartedA], slots, choices)?.effective_instance, 2);
  assert.equal(joystickSlot(restartedA, [restartedA], slots, choices), null);
  assert.equal(joystickSlot(a, devices, [{ ...slots[0], effective_instance: 3 }, slots[1]], choices), null);
});

test('ambiguous serials can be assigned separately for the current session', () => {
  const x = { ...a, sdl_path: null, sdl_serial: 'duplicate' };
  const y = { ...b, sdl_path: null, sdl_serial: 'duplicate' };
  let choices = assignJoystickSlot(x, [x, y], slots, {}, 1);
  choices = assignJoystickSlot(y, [x, y], slots, choices, 2);
  assert.equal(joystickSlot(x, [x, y], slots, choices)?.effective_instance, 1);
  assert.equal(joystickSlot(y, [x, y], slots, choices)?.effective_instance, 2);
  assert.notEqual(deviceKey(x), deviceKey(y));
});

test('unique serials can remember choices without an OS path', () => {
  const x = { ...a, sdl_path: null, sdl_serial: 'one' };
  const y = { ...b, sdl_path: null, sdl_serial: 'two' };
  const choices = assignJoystickSlot(x, [x, y], slots, {}, 2);
  const restarted = { ...x, sdl_instance_id: 101 };
  assert.equal(joystickSlot(restarted, [restarted, { ...y, sdl_instance_id: 102 }], slots, choices)?.effective_instance, 2);
});

test('existing unique devices resolve automatically and unknown products do not', () => {
  assert.equal(joystickSlot(a, [a], [slots[0]], {})?.effective_instance, 1);
  assert.equal(joystickSlot(a, [a], [{ ...slots[0], sc_product_guid: 'other' }], {}), null);
});

test('image-map choices are per unit and retain legacy model choices as fallbacks', () => {
  const x = { ...a, hardware_id: 'PRODUCT' }, y = { ...b, hardware_id: 'PRODUCT' };
  const choices = { product: 'legacy-map', [deviceKey(x)]: 'joystick-map' };
  assert.equal(mapChoice(x, choices), 'joystick-map');
  assert.equal(mapChoice(y, choices), 'legacy-map');
  choices[deviceKey(y)] = 'throttle-map';
  assert.equal(mapChoice(x, choices), 'joystick-map');
  assert.equal(mapChoice(y, choices), 'throttle-map');
  assert.equal(mapChoice({ ...x, index: 0, sdl_instance_id: 99 }, choices), 'joystick-map');
});
