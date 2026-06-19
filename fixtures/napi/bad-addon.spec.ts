import { describe, it, expect } from 'vitest';

// A native addon whose load triggers an unsupported N-API entrypoint must surface
// as a catchable JS error — NOT a process-level SIGSEGV that kills the whole run.
describe('native addon crash hardening', () => {
  it('a misbehaving .node throws a catchable error', () => {
    expect(() => {
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      require('./bad_addon.node');
    }).toThrow();
  });

  it('a sibling test in the same file still runs', () => {
    expect(2 + 2).toBe(4);
  });
});
