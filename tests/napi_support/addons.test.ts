import { test, expect } from 'vitest';

// full.node / second.node are compiled by tests/napi_host.rs next to this file.
const a = require('./full.node');

test('a second addon gets its own env: function tables and references stay apart', () => {
  const target = { id: 1 };
  const id = a.ref(target);
  expect(id).toBe(1);
  const b = require('./second.node');
  expect(b.who()).toBe('second');
  // full.node's functions still dispatch into full.node's table ...
  expect(typeof a.status()).toBe('number');
  expect(a.typeOf('s')).toBe(4);
  // ... and each addon's reference 1 is its own
  expect(a.deref(id)).toBe(target);
  expect(b.held()).toBe('held by second');
});

test('functions created after another addon loaded belong to their creator', () => {
  const f = a.makeFn(3);
  expect(f('x')).toEqual([1, globalThis, 3, 'x']);
  expect(require('./second.node').who()).toBe('second');
});
