describe('only mode', () => {
  it('this is skipped under only', () => { throw new Error('must not run'); });
  it.only('only this runs', () => { expect(1 + 1).toBe(2); });
});
