const { test, expect } = require('@jest/globals');

test('runs setupFilesAfterEnv', () => {
  expect(globalThis.backendCompatSetup).toBe('ready');
});

test('keeps Jest globals disabled when injectGlobals is false', () => {
  expect(typeof globalThis.test).toBe('undefined');
  expect(typeof globalThis.expect).toBe('undefined');
});
