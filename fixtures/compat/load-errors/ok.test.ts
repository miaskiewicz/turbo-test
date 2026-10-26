import { it, expect } from 'vitest';

it('still runs and passes next to files that fail to load', () => {
  expect(1 + 1).toBe(2);
});
