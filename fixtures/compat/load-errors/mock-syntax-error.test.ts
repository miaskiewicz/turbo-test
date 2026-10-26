import { it, expect, vi } from 'vitest';
import { x } from './dep';

vi.mock('./dep', () => ({ x: ( }));

it('never runs: the mock factory above is not valid JavaScript', () => {
  expect(x).toBe(1);
});
