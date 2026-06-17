import { it, expect } from 'vitest';

const myTest = it.extend({
  num: 7,
  doubled: async ({ num }, use) => {
    await use(num * 2);
  },
});

myTest('provides a plain fixture', ({ num }) => {
  expect(num).toBe(7);
});

myTest('provides a use()-based fixture that depends on another', ({ doubled }) => {
  expect(doubled).toBe(14);
});
