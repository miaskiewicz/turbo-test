test('runs globalSetup before the suite', () => {
  expect(process.env.TURBO_REPRO_GLOBAL_SETUP).toBe('ready');
});

test('resolves a moduleNameMapper alias', () => {
  expect(require('@fixture')).toBe('mapped');
});
