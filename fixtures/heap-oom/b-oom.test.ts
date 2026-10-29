import { expect, it } from 'vitest';

// Allocates until V8's heap limit. The guard must terminate it and fail this file only.
const hog: unknown[] = [];
it('never gets here', () => expect(hog.length).toBeGreaterThan(0));
for (;;) hog.push(new Array(100_000).fill({ x: 1 }));
