describe('math', () => {
  let x: number;
  beforeEach(() => { x = 10; });
  it('adds', () => { expect(1 + 1).toBe(2); });
  it('hook ran', () => { expect(x).toBe(10); });
  it.each([[1, 2, 3], [2, 3, 5]])('sums', (a: number, b: number, c: number) => {
    expect(a + b).toBe(c);
  });
  it('throws', () => { expect(() => { throw new Error('boom'); }).toThrow('boom'); });
  it('deep equal', () => { expect({ a: [1, 2] }).toEqual({ a: [1, 2] }); });
  it('negation', () => { expect(3).not.toBe(4); });
});
