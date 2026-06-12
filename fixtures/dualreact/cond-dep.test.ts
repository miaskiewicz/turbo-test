import { marker, greet } from 'cond-dep';

describe('node_modules dep with import.node ESM + require CJS conditions', () => {
  it('loads through the CJS module-runner without a compile error', () => {
    expect(typeof marker).toBe('string');
    expect(greet()).toBe('hi-' + marker);
  });
});
