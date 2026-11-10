const mock = jest.fn();
const service = { value: () => 'original' };

test('installs a mock and a spy for the next test to observe', () => {
  mock.mockReturnValue('temporary');
  jest.spyOn(service, 'value').mockReturnValue('temporary');
  expect(mock()).toBe('temporary');
  expect(service.value()).toBe('temporary');
});

test('resetMocks clears the previous test implementation', () => {
  expect(mock()).toBeUndefined();
});

test('restoreMocks restores the previous test spy', () => {
  expect(service.value()).toBe('original');
});

test('setupFiles runs before the framework and setupFilesAfterEnv runs after it', () => {
  expect([globalThis.fixtureEarly, globalThis.fixtureLate]).toEqual(['undefined', 'function']);
});

test('moduleDirectories resolves a bare name from a custom directory', () => {
  expect(require('fixture-local')).toBe('custom directory');
});

test('customExportConditions selects the configured package export', () => {
  expect(require('conditional-fixture')).toBe('custom');
});
