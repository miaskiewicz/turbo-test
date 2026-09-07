import { it, expect } from 'vitest';
// Runs after a-stub.test.ts in the SAME reused worker. Under isolate reuse the prior file's
// unrestored `window.parent` stub must NOT leak here — a top-level window self-references.
it('window.parent/top/self === window despite a prior file leaving parent stubbed', () => {
  expect(window.parent === window).toBe(true);
  expect(window.top === window).toBe(true);
  expect(window.self === window).toBe(true);
});
