import { describe, it, expect } from 'vitest';

describe.skipIf(true)('skipped describe block', () => {
  it('never runs', () => {
    expect(1).toBe(2); // would fail if it ran
  });
});

describe.runIf(false)('also skipped describe block', () => {
  it('never runs either', () => {
    expect(1).toBe(2);
  });
});

describe.skipIf(false)('included describe block', () => {
  it('runs', () => {
    expect(1).toBe(1);
  });
});

describe.runIf(true)('also included describe block', () => {
  it('runs too', () => {
    expect(2).toBe(2);
  });
});

describe.concurrent('concurrent describe', () => {
  it('runs in a concurrent block', () => {
    expect(3).toBe(3);
  });
});

describe.todo('a todo describe');
