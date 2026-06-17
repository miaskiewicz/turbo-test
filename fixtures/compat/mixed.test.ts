import { describe, it, expect } from 'vitest';

describe('math', () => {
  it('adds', () => {
    expect(1 + 1).toBe(2);
  });
  it('fails subtraction', () => {
    expect(3 - 1).toBe(5);
  });
});

describe('strings', () => {
  it('concatenates', () => {
    expect('a' + 'b').toBe('ab');
  });
});
