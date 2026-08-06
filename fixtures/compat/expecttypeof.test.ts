// `expectTypeOf` must exist on the `vitest` module surface (issue #16). Type assertions are erased
// at compile time, so the only runtime requirement is that the import resolves and the chain is
// callable — including through a namespace import (the shape a CJS interop transform produces).
import { describe, it, expect, expectTypeOf, assertType } from 'vitest';
import * as vitest from 'vitest';

type User = { id: string; name: string };

describe('expectTypeOf', () => {
  it('is exported from vitest as a function', () => {
    expect(typeof expectTypeOf).toBe('function');
    expect(typeof assertType).toBe('function');
  });

  it('is callable and chainable', () => {
    expectTypeOf<User>().toHaveProperty('id');
    expectTypeOf<User>().toEqualTypeOf<{ id: string; name: string }>();
    expectTypeOf<User>().not.toEqualTypeOf<{ id: string }>();
    expectTypeOf<string[]>().items.toBeString();
    assertType<User>({ id: 'a', name: 'b' });
  });

  it('works through a namespace import', () => {
    expect(typeof vitest.expectTypeOf).toBe('function');
    vitest.expectTypeOf<number>().toEqualTypeOf<number>();
  });

  it('is available as a global', () => {
    expect(typeof globalThis.expectTypeOf).toBe('function');
    (globalThis as any).expectTypeOf<number>().toBeNumber();
  });

  it('does not disturb the surrounding test', () => {
    expectTypeOf<number>().toBeNumber();
    expect(1 + 1).toBe(2);
  });
});
