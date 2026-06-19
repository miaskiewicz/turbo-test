// Compile-only regression guard for the shipped `.d.ts` shim. NOT run by the test runner (types
// are stripped at run time) — checked by `test/compat-types.test.mjs` via `tsc --noEmit --strict`
// when a TypeScript compiler is resolvable. Every line below must type-check; a regression in
// turbo-test-api.d.ts surfaces as a tsc error here.
/// <reference path="../globals.d.ts" />

// ---- it.each: tuple rows preserve per-position types -----------------------------------------

// `as const` multi-arg (the dominant payroll pattern) — spread to typed positional args.
it.each([
  ['rejected', 'error'],
  ['paid', 'info'],
] as const)('maps %s to %s', (status, color) => {
  const _s: 'rejected' | 'paid' = status;
  const _c: 'error' | 'info' = color;
  void _s; void _c;
});

// `as const` single-element tuple — the 1-tuple is spread to ONE arg, not the whole tuple.
it.each([
  ['changes_requested'],
] as const)('chip %s', (status) => {
  const _v: 'changes_requested' = status;
  void _v;
});

// non-const heterogeneous rows still infer per-position (not a widened string|boolean).
it.each([
  ['not the last row', false],
  ['the last row', true],
])('is %s', (label, isLast) => {
  const _l: string = label;
  const _b: boolean = isLast;
  void _l; void _b;
});

// numeric tuple still spreads.
it.each([[1, 2]])('sum %i', (a, b) => {
  const _n: number = a + b;
  void _n;
});

// explicit type arg over a non-array union → single-value overload.
type InviteType = 'partner_join_org' | 'employee';
it.each<InviteType>(['partner_join_org', 'employee'])('is %s', (t) => {
  const _t: InviteType = t;
  void _t;
});

// ---- vi.mocked / vi.hoisted / vi.fn<T> / mock importOriginal (rounds 1–2 guards) -------------

function useThing(): { v: number } { return { v: 1 }; }
const mocked = vi.mocked(useThing);
mocked.mockReturnValue({ v: 2 });
mocked.mockReturnValueOnce({ v: 3 });
void mocked.mock.calls;

const hoisted = vi.hoisted(() => ({ shared: vi.fn() }));
hoisted.shared();

const typedFn = vi.fn<(a: string) => number>();
typedFn.mockReturnValue(7);
const _r: number = typedFn('x');
void _r;

type Mod = { useQuery: (opts: { queryFn?: () => unknown }) => unknown; other: number };
vi.mock('@tanstack/react-query', async (importOriginal) => {
  const actual = await importOriginal<Mod>();
  return { ...actual, useQuery: (options: Parameters<typeof actual.useQuery>[0]) => options.queryFn };
});

// expect.fail returns never.
function assertTrue(c: boolean): void {
  if (!c) expect.fail('nope');
}
void assertTrue;
