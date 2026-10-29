import { test, expect } from 'vitest';

// Addons compiled by tests/napi_host.rs next to this file (sources in tests/napi_support/).
test('an init returning a value replaces the exports object', () => {
  const m = require('./init_custom.node');
  expect(m).toEqual({ kind: 'custom' });
  expect(m.onExports).toBe(undefined);
});

test('a JS exception thrown during init is a catchable require() error', () => {
  expect(() => require('./init_throw.node')).toThrow(
    /^native addon \/.*\/init_throw\.node: native addon threw during module init: Error: init exploded$/,
  );
  // not cached as a success: requiring again fails the same way
  expect(() => require('./init_throw.node')).toThrow(/init exploded/);
});


// host-only: Node binds the import lazily and crashes on the call
test('host-only: an addon importing a Node-API function the host does not provide fails require()', () => {
  expect(() => require('./unknown.node')).toThrow(
    /^native addon \/.*\/unknown\.node: it imports napi_turbo_test_unknown_entrypoint, which turbo-test's Node-API host does not provide \(dlopen .*napi_turbo_test_unknown_entrypoint/,
  );
  // the run goes on: other addons still load
  expect(require('./second.node').who()).toBe('second');
});

test('a shared library without napi_register_module_v1 is rejected', () => {
  expect(() => require('./noreg.node')).toThrow(
    /^native addon \/.*\/noreg\.node: no napi_register_module_v1: /,
  );
});

test('a file that fails to dlopen is rejected', () => {
  expect(() => require('./garbage.node')).toThrow(/^native addon \/.*\/garbage\.node: dlopen /);
});

// Module init runs under the host's addon lock; this nested init is a same-thread re-entry and
// must not deadlock. (Host-only: Node hangs here, its dlopen mutex is held across module init.)
test("require() of a .node from inside another addon's module init", () => {
  (globalThis as any).loadNested = () => require('./second.node');
  expect(require('./init_nested.node').nested.who()).toBe('second');
});
