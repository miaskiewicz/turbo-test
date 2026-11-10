test('restores the outer module registry after isolateModulesAsync', async () => {
  const outer = require('./module-state.cjs');
  await jest.isolateModulesAsync(async () => {
    expect(require('./module-state.cjs')).not.toBe(outer);
  });
  expect(require('./module-state.cjs')).toBe(outer);
});

test('restoreAllMocks undoes jest.replaceProperty', () => {
  const object = { value: 'original' };
  jest.replaceProperty(object, 'value', 'temporary');
  expect(object.value).toBe('temporary');
  jest.restoreAllMocks();
  expect(object.value).toBe('original');
});

test('runs a script in a separate vm context', () => {
  const { runInNewContext } = require('node:vm');
  expect(runInNewContext('input + 1', { input: 41 })).toBe(42);
});
