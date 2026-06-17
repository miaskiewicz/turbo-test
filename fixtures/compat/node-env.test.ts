import { describe, it, expect } from 'vitest';

// Mentions `document` so the content heuristic WOULD pick a DOM environment; under
// `--environment node` the DOM install is skipped, so `document` stays undefined and the
// Node globals (process, Buffer) are present. Used by test/compat-config-env.test.mjs.
describe('node environment', () => {
  it('has Node globals', () => {
    expect(typeof process).toBe('object');
    expect(typeof process.versions.node).toBe('string');
  });
  it('has no DOM (document is undefined)', () => {
    expect(typeof document).toBe('undefined');
  });
});
