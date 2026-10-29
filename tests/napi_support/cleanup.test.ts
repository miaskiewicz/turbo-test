import { test, expect } from 'vitest';

// cleanup.node (tests/napi_support/cleanup.c): cleanup hooks, instance data and finalizers that
// run when the addon's env is torn down (the end of this file's isolate); tests/napi_host.rs
// reads what they report on stderr. The statuses are Node 26's.
const c = require('./cleanup.node');
const OK = 0;
const INVALID_ARG = 1;

test('registrations for env teardown', () => {
  expect(c.instanceData()).toBe(0);
  const kept: any = {};
  (globalThis as any).kept = kept;
  expect(c.setUp(kept)).toEqual([OK, OK, OK, OK, OK, OK, OK, OK, OK, OK, OK, 1, OK, OK]);
  expect(kept.str).toBe('kept until teardown');
  // the data set last
  expect(c.instanceData()).toBe(6);
  expect(c.checks()).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, OK, OK, OK]);
});
