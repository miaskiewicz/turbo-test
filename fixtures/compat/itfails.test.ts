import { describe, it, expect } from 'vitest';

describe('it.fails', () => {
  it.fails('passes because the body throws', () => {
    expect(1).toBe(2);
  });

  it.fails('passes because it throws a plain error', () => {
    throw new Error('expected to fail');
  });
});
