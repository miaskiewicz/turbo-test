test('loads a published package with a dev-only tsconfig parent', () => {
  expect(require('es-errors/type')).toBe(TypeError);
  expect(typeof require('side-channel')).toBe('function');
});

test('resolves a sibling package from a dependency with a missing tsconfig parent', () => {
  expect(require('consumer-fixture')).toBe(TypeError);
});
