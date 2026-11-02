import { test, expect } from 'vitest';

// full.node / second.node are compiled by tests/napi_host.rs next to this file. Each callback
// below runs while another native callback of the same addon is still on the stack.
const a = require('./full.node');

test('a napi function called from inside a native callback (napi_call_function)', () => {
  const f = a.makeFn(5);
  const recv = { me: 1 };
  expect(a.call(f, recv, 'q')).toEqual([1, recv, 5, 'q']);
});

test('nested native -> JS -> native -> JS calls unwind cleanly', () => {
  expect(a.call(() => a.typeOf('s'), null)).toBe(4);
  expect(a.call(() => a.call((x: number) => x * 2, null, 21), null)).toBe(42);
  // the outer callback keeps making napi calls after the nested ones returned
  expect(a.call(() => a.basics()[7], null)).toBe('héllo wörld €');
  expect(a.status()).toBe(0);
});

test('require() of a .node from inside a native callback', () => {
  expect(a.call(() => require('./second.node').who(), null)).toBe('second');
});
