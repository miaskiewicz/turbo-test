import { it, expect } from 'vitest';

it('never runs: the setup file imports a module that does not compile', () => {
  expect(1).toBe(1);
});
