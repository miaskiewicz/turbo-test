import { realValue } from './dep.cjs';

jest.mock('./dep.cjs', () => ({ realValue: jest.fn(() => 'mocked') }));

describe('jest global shim', () => {
  it('jest.fn tracks calls + return', () => {
    const f = jest.fn((x: number) => x * 2);
    expect(f(21)).toBe(42);
    expect(f).toHaveBeenCalledWith(21);
  });

  it('jest.mock replaces a module (factory)', () => {
    expect(realValue()).toBe('mocked');
    expect(jest.isMockFunction(realValue)).toBe(true);
  });

  it('jest.spyOn wraps a method', () => {
    const obj = { greet: () => 'hi' };
    const spy = jest.spyOn(obj, 'greet').mockReturnValue('spied');
    expect(obj.greet()).toBe('spied');
    expect(spy).toHaveBeenCalled();
  });

  it('jest config setupFiles ran (env marker, <rootDir> resolved)', () => {
    expect(process.env.JEST_SETUP_RAN).toBe('yes');
  });
});
