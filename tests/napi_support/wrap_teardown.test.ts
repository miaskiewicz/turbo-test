import { test, expect } from 'vitest';

// wrap.node (tests/napi_support/wrap.c): objects still wrapped (or with a napi_add_finalizer
// finalizer) when the addon's env is torn down (the end of this file's isolate) have their
// finalizers run then; tests/napi_host.rs reads what they report on stderr.
const w = require('./wrap.node');

test('objects that stay wrapped (or keep a finalizer) until the env is torn down', () => {
  const kept = [{}, {}, {}];
  (globalThis as any).kept = kept;
  expect(w.wrapTeardown(kept[0], 41)).toBe(0);
  expect(w.finalizerTeardown(kept[1], 43)).toBe(0);
  expect(w.wrapTeardown(kept[2], 42)).toBe(0);
});
