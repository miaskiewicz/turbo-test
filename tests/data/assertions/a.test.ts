// turbo-test#102: expect.assertions / hasAssertions and test timeouts fail with vitest's messages
// and frames (hasAssertions: where it was called; a timeout: the test's registration).
import { describe, expect, it } from 'vitest';
it('assertions', () => { expect.assertions(2); expect(1).toBe(1); });
it('has assertions', () => { expect.hasAssertions(); });
it('timeout', { timeout: 30 }, async () => { await new Promise(() => {}); });
it('timeout as the last argument', async () => { await new Promise(() => {}); }, 20);
describe('d', () => {
  it('nested timeout', { timeout: 10 }, async () => { await new Promise(() => {}); });
});
it('assertions met', () => { expect.assertions(1); expect.hasAssertions(); expect(1).toBe(1); });
