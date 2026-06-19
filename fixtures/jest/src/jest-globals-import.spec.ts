// Jest projects with `injectGlobals: false` (or TS-strict setups) import the API explicitly
// from '@jest/globals' instead of relying on injected globals. turbo-test must satisfy that
// bare specifier from its own runtime, never the real @jest/globals package.
import { describe, it, expect, jest, beforeEach } from '@jest/globals';

let seen = 0;
beforeEach(() => { seen++; });

describe('@jest/globals named imports', () => {
  it('describe/it/expect are the real runtime API', () => {
    expect(typeof describe).toBe('function');
    expect(typeof it).toBe('function');
    expect(seen).toBeGreaterThan(0);
  });

  it('jest.fn imported from @jest/globals tracks calls', () => {
    const f = jest.fn((x: number) => x + 1);
    expect(f(41)).toBe(42);
    expect(f).toHaveBeenCalledWith(41);
    expect(jest.isMockFunction(f)).toBe(true);
  });
});
