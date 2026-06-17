import { describe, it, expect } from 'vitest';

describe('bail file a', () => {
  it('fails 1', () => { expect(1).toBe(2); });
  it('fails 2', () => { expect(1).toBe(2); });
});
