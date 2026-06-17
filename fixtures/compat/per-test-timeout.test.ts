import { describe, it, expect } from 'vitest';

describe('per-test timeout group', () => {
  it('hangs but has a tiny explicit timeout', async () => {
    await new Promise(() => {}); // never resolves
  }, { timeout: 20 });

  it('hangs with a numeric timeout arg', async () => {
    await new Promise(() => {});
  }, 20);

  it('fast test still passes', () => {
    expect(true).toBe(true);
  });
});
