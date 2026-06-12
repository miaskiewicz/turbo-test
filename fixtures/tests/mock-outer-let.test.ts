import { get, lookup, theme } from '../cjs/dep-outer.cjs';

// factory closes over module-scope mutable `let`s that TESTS reassign (the ImssWorkRiskCard /
// usePartnerRole / WizardLayout patterns): factory only READS the bindings; values set in it().
let outerValue: { n: number } = { n: 1 };
let mockRole: string | null = null;         // name also appears as a STRING LITERAL in the factory
let themeMode: 'light' | 'dark' = 'light';  // read via a live GETTER whose NAME collides

vi.mock('../cjs/dep-outer.cjs', async () => {
  const actual: any = await vi.importActual('../cjs/dep-outer.cjs');
  const ctx = {
    get themeMode() {            // accessor NAME must stay; body `return themeMode` must rewrite
      return themeMode;
    },
  };
  return {
    ...actual,
    get: () => outerValue,
    lookup: (k: string) => (k === 'mockRole' ? mockRole : null), // 'mockRole' string stays intact
    theme: () => ctx.themeMode,
  };
});

describe('vi.mock factory closes over module-scope lets reassigned by tests', () => {
  it('reads the latest outer-let value', () => {
    outerValue = { n: 42 };
    expect((get() as any).n).toBe(42);
  });
  it('rewrites the identifier but preserves a colliding string literal', () => {
    mockRole = 'admin';
    expect((lookup as any)('mockRole')).toBe('admin');
    expect((lookup as any)('other')).toBeNull();
  });
  it('rewrites a getter body read but not the getter name', () => {
    themeMode = 'dark';
    expect((theme as any)()).toBe('dark');
  });
});
