import { describe, it, expect } from 'vitest';
import { greet } from '@/util';
import { add } from '~lib/math';
import { add as addExact } from '@math';

describe('resolve.alias + test.alias', () => {
  it('object-form prefix alias (@ -> ./src) resolves', () => {
    expect(greet('world')).toBe('hi world');
  });
  it('second object-form prefix alias (~lib -> ./lib) resolves', () => {
    expect(add(2, 3)).toBe(5);
  });
  it('array-form test.alias exact alias (@math -> ./lib/math.ts) resolves', () => {
    expect(addExact(4, 5)).toBe(9);
  });
});
