import { describe, it, expect } from 'vitest';

// Lives in the same run as bad-addon.spec.ts. Before the fix, the addon SIGSEGV
// took the whole process down and this file never executed. It must run now.
describe('unrelated spec survives a sibling addon failure', () => {
  it('runs independently', () => { expect('alive').toBe('alive'); });
});
