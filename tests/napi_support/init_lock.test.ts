import { test, expect } from 'vitest';

// tests/napi_host.rs runs this spec as two files on two workers; both require init_lock.node
// (one shared library) at about the same time.
test('addon module init never runs on two threads at once', () => {
  expect(require('./init_lock.node').maxInside()).toBe(1);
});
