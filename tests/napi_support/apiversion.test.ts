import { test, expect } from 'vitest';

// apiversion.c built as v10 / v3 / vexp (NAPI_VERSION_EXPERIMENTAL) / v11 by tests/napi_host.rs.
// The expected values are Node 26's (but the Node.js version reported, which is the one
// process.versions.node names).
const OK = 0;
const INVALID_ARG = 1;

test('napi_get_version and napi_get_node_version', () => {
  const [s1, v, s2, major, minor, patch, release, nulls] = require('./v10.node').versions();
  expect([s1, v, s2, release, nulls]).toEqual([OK, 10, OK, 'node', INVALID_ARG * 100 + INVALID_ARG]);
  expect(`${major}.${minor}.${patch}`).toBe(process.versions.node);
  expect(process.versions.napi).toBe('10');
});

test('the declared module API version: primitives can be referenced from 10', () => {
  for (const name of ['./v10.node', './vexp.node']) {
    const m = require(name);
    expect(m.refValue('str', 1)).toEqual([OK, 'str']);
    // a value that cannot be held weakly goes at count 0
    expect(m.refValue('str', 0)).toEqual([OK, 'NULL']);
    expect(m.refValue({ o: 1 }, 1)).toEqual([OK, { o: 1 }]);
  }
  // below 8 is 8: only objects, functions and symbols
  const v3 = require('./v3.node');
  expect(v3.refValue('str', 1)).toEqual([INVALID_ARG, 'NULL']);
  expect(v3.refValue({ o: 1 }, 1)).toEqual([OK, { o: 1 }]);
});

test('an addon declaring a newer Node-API version fails require()', () => {
  expect(() => require('./v11.node')).toThrow(
    /file:\/\/\/.*\/v11\.node requires Node-API version 11, but this version of Node\.js only supports version 10 add-ons\./,
  );
});

test('at env teardown a call that would run JS is napi_cannot_run_js from version 10', () => {
  const kept = [{}, {}];
  (globalThis as any).keptByVersion = kept;
  require('./v10.node').keepUntilTeardown(kept[0]);
  require('./v3.node').keepUntilTeardown(kept[1]);
});
