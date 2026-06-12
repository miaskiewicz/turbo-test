const mod = await import('../esm/const.mjs');   // top-level await + dynamic import
describe('dynamic import', () => {
  it('loads namespace', () => { expect(mod.TWO).toBe(2); expect(mod.THREE).toBe(3); });
});
