import { describe, expect, it } from 'vitest';

// Each file keeps ~100 MB reachable from a global, the way a reused isolate keeps a finished
// file's module graph, DOM and timer closures. Twelve of them are ~1.2 GB: more than the 700 MB
// cap the test runs under, unless the worker replaces its isolate along the way.
const keep = ((globalThis as any).__heapStress ??= []) as unknown[][];
const chunk: { i: number; s: string }[] = [];
for (let i = 0; i < 1_000_000; i++) chunk.push({ i, s: 'row-' + i });
keep.push(chunk);

describe('heap stress 05', () => {
  it('still runs', () => {
    expect(chunk.length).toBe(1_000_000);
  });
});
