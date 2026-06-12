let attempts = 0;
describe('m6 modifiers', () => {
  it.concurrent('concurrent api accepted', () => { expect(1).toBe(1); });
  it('retries until pass', () => { attempts++; if (attempts < 3) throw new Error('flaky'); expect(attempts).toBe(3); }, { retry: 3 });
  it.skipIf(true)('skipIf true is skipped', () => { throw new Error('should not run'); });
  it.runIf(true)('runIf true runs', () => { expect(true).toBe(true); });
});
