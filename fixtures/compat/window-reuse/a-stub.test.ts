import { it, expect } from 'vitest';
// Names sort a-* before b-*, so this file runs first in the shared (reused) worker. It stubs
// window.parent and deliberately does NOT restore it — the next file must not inherit the stub.
it('stubs window.parent without restoring it', () => {
  Object.defineProperty(window, 'parent', { value: { framed: true }, configurable: true });
  expect((window.parent as any).framed).toBe(true);
});
