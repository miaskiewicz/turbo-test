import lib, { VERSION } from '../cjs/lib.cjs';
describe('interop', () => {
  it('default + named from CJS', () => {
    expect(VERSION).toBe('1.0.0');
    expect(lib.greet('y')).toBe('hi y');
  });
});
