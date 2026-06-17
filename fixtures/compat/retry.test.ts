import { describe, it, expect } from 'vitest';

// A test that fails the first time(s) and passes once a counter reaches a threshold.
// `globalThis` persists across retries within a single file run, so this exercises the
// global --retry default (no explicit per-test { retry }).
const g: any = globalThis;
g.__retry_counter = (g.__retry_counter || 0);

describe('retry group', () => {
  it('passes only on the 3rd attempt', () => {
    g.__retry_counter++;
    expect(g.__retry_counter).toBeGreaterThanOrEqual(3);
  });
});
