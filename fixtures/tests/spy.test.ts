describe('vi.fn', () => {
  it('tracks calls', () => {
    const f = vi.fn((a: number) => a * 2);
    expect(f(3)).toBe(6);
    expect(f).toHaveBeenCalledWith(3);
    expect(f).toHaveBeenCalledTimes(1);
  });
});
