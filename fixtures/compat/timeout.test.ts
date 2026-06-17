import { describe, it, expect } from 'vitest';

describe('timeout group', () => {
  it('fast test passes', () => {
    expect(1).toBe(1);
  });
  it('slow test hangs forever', async () => {
    await new Promise(() => {}); // never resolves
  });
});
