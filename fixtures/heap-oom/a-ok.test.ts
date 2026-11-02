import { expect, it } from 'vitest';
it('runs before the file that runs out of memory', () => expect(1 + 1).toBe(2));
