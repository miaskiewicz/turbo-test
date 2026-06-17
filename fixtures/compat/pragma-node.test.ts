// @vitest-environment node
import { describe, it, expect } from 'vitest';

// The per-file pragma above forces the `node` environment regardless of `--environment` /
// config. The DOM install is skipped even though this file references `document`.
describe('pragma node environment', () => {
  it('document is undefined under @vitest-environment node', () => {
    expect(typeof document).toBe('undefined');
  });
});
