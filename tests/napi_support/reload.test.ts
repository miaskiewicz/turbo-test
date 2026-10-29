import { test, expect } from 'vitest';

// selfreg.node (tests/napi_support/selfreg.c). Expected values: Node 26's.
test('an addon dropped from require.cache is loaded again: its init runs for new exports', () => {
  const m = require('./selfreg.node');
  const before = m.inits;
  const key = require.resolve('./selfreg.node');
  expect(require.cache[key]!.exports).toBe(m);
  delete require.cache[key];
  expect(key in require.cache).toBe(false);
  const again = require('./selfreg.node');
  expect(again).not.toBe(m);
  expect([again.how, again.inits]).toEqual(['self-registered', before + 1]);
});
