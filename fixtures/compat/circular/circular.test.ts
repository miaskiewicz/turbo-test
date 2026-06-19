import { describe, it, expect } from 'vitest';
// Importing the entry of a require cycle must not throw
// `Cannot access 'X' before initialization` (TDZ). The graph is A <-> B.
import { aName, greetingFromB, AModel } from './a';
import { bName, describeA } from './b';

describe('circular module graph loads', () => {
  it('loads both modules in the cycle without a TDZ error', () => {
    expect(aName).toBe('A');
    expect(bName).toBe('B');
  });

  it('A could read B\'s value at eval time', () => {
    expect(greetingFromB).toBe('A sees: B');
  });

  it('cross-cycle class extension works', () => {
    const m = new AModel();
    expect(m.tag()).toBe('Base');
    expect(m.who()).toBe('AModel:B');
  });

  it('lazy back-reference into the partially-initialized module resolves', () => {
    expect(describeA()).toBe('B sees: A via AModel');
  });
});
