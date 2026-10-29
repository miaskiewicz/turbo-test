import { test, expect } from 'vitest';

// selfreg.node (tests/napi_support/selfreg.c) registers itself with napi_module_register from a
// static constructor. tests/napi_host.rs runs two copies of this file: the second loads the
// library again, whose constructors do not run again; its saved module's init runs for the new
// exports (as in a second Node worker).
test('a module registered with napi_module_register from a static constructor', () => {
  const m = require('./selfreg.node');
  expect(m.how).toBe('self-registered');
  expect(m.inits).toBeGreaterThanOrEqual(1);
});
