import { describe, it, expect } from 'vitest';

describe('alpha group', () => {
  it('adds numbers', () => {
    expect(1 + 1).toBe(2);
  });
  it('subtracts numbers', () => {
    expect(3 - 1).toBe(2);
  });
});

describe('beta group', () => {
  it('concatenates strings', () => {
    expect('a' + 'b').toBe('ab');
  });
});
