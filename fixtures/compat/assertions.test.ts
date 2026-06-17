import { describe, it, expect } from 'vitest';

describe('assertion counting', () => {
  it('passes when expect.assertions(n) matches', () => {
    expect.assertions(2);
    expect(1).toBe(1);
    expect(2).toBe(2);
  });

  it('fails when expect.assertions(n) does not match', () => {
    expect.assertions(2);
    expect(1).toBe(1); // only one — should fail the count check
  });

  it('passes when hasAssertions and at least one ran', () => {
    expect.hasAssertions();
    expect(true).toBe(true);
  });

  it('fails hasAssertions when none ran', () => {
    expect.hasAssertions();
    // no expect() call
  });
});
