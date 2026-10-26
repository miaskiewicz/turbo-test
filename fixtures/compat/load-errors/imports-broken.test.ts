import { it, expect } from 'vitest';
import { x } from './broken';

it('never runs: the import above fails to compile', () => {
  expect(x).toBe(1);
});
