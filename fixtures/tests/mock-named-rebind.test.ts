import { getMandatorySpecialPayPeriods, otherHelper } from '../cjs/special.cjs';

// async vi.mock factory that dynamically imports the SAME spec (delegating vi.fn spy) —
// mirrors the payroll-app usePayrollDates.test.ts failure.
vi.mock('../cjs/special.cjs', async () => {
  const real: any = await import('../cjs/special.cjs');
  return {
    otherHelper: real.otherHelper,
    getMandatorySpecialPayPeriods: vi.fn(real.getMandatorySpecialPayPeriods),
  };
});

describe('vi.mocked named-import rebind (async self-importing factory)', () => {
  it('named import IS the factory spy and mockReturnValueOnce takes effect', () => {
    expect(typeof getMandatorySpecialPayPeriods).toBe('function');
    expect(vi.isMockFunction(getMandatorySpecialPayPeriods)).toBe(true);
    vi.mocked(getMandatorySpecialPayPeriods).mockReturnValueOnce(['mocked-once'] as any);
    expect(getMandatorySpecialPayPeriods()).toEqual(['mocked-once']);
    // delegates to the real impl after the once-value is consumed
    expect(getMandatorySpecialPayPeriods()).toEqual(['real']);
  });

  it('non-spied named export keeps the real implementation', () => {
    expect(otherHelper()).toBe('other-real');
  });
});
