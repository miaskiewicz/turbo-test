import lib, { VERSION, greet } from '../cjs/lib.cjs';
describe('vi.mock', () => {
  it('intercepts module', () => {
    expect(VERSION).toBe('9.9.9');
    expect(greet('x')).toBe('mocked');
    expect(lib.VERSION).toBe('9.9.9');
  });
});
vi.mock('../cjs/lib.cjs', () => ({ VERSION: '9.9.9', greet: () => 'mocked' }));
