import { describe, expect, it, vi } from 'vitest';

// A spy records the arguments of every call. Twelve of these files record ~60 MB each; once a
// file is over, nothing may keep that history reachable, or the 12 together (~700 MB) outgrow the
// 400 MB cap the test runs under.
describe('spy history 12', () => {
  it('records its calls', () => {
    const onRow = vi.fn();
    const hoisted = vi.fn((row: unknown) => row);
    for (let i = 0; i < 3_000; i++) {
      onRow(new Array(2_500).fill({ i }));
      hoisted({ rows: new Array(500).fill('row-' + i) });
    }
    expect(onRow).toHaveBeenCalledTimes(3_000);
    expect(hoisted.mock.calls.length).toBe(3_000);
  });
});
