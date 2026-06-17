import { describe, it, expect, vi } from 'vitest';

describe('common matchers', () => {
  it('toMatchObject', () => {
    expect({ a: 1, b: 2, c: 3 }).toMatchObject({ a: 1, b: 2 });
  });

  it('toContainEqual', () => {
    expect([{ a: 1 }, { b: 2 }]).toContainEqual({ b: 2 });
  });

  it('toSatisfy', () => {
    expect(4).toSatisfy((n: number) => n % 2 === 0);
  });

  it('toHaveBeenCalledOnce', () => {
    const fn = vi.fn();
    fn();
    expect(fn).toHaveBeenCalledOnce();
  });

  it('toHaveBeenNthCalledWith', () => {
    const fn = vi.fn();
    fn('a');
    fn('b');
    expect(fn).toHaveBeenNthCalledWith(2, 'b');
  });
});
