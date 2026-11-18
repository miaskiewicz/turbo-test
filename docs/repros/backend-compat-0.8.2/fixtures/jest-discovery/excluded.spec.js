const { test, expect } = require('@jest/globals');

test('must be excluded by testMatch', () => {
  expect(false).toBe(true);
});
