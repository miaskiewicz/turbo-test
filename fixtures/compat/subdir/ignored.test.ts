import { describe, it, expect } from 'vitest';

// Matches default discovery (*.test.ts) but NOT the subdir config's include
// (**/*.feature.spec.ts). When the subdir config drives discovery this file is skipped.
describe('ignored by feature-spec include', () => {
  it('should not run under the subdir config', () => {
    expect(true).toBe(false); // would FAIL if wrongly discovered
  });
});
