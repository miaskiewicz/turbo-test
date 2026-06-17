import { describe, it, expect } from 'vitest';

describe('only group', () => {
  it.only('the only test', () => {
    expect(1).toBe(1);
  });
  it('the skipped test', () => {
    expect(1).toBe(1);
  });
});
