// Compile-only regression guard for `test.runIf` / `test.skipIf` / `describe.runIf` /
// `describe.skipIf` in the shipped `.d.ts` shim. NOT run by the test runner — checked by
// `test/compat-types.test.mjs` via `tsc --noEmit --strict`. Every line must type-check; a
// regression in `TestAPI`/`SuiteAPI` surfaces as a tsc error here.
/// <reference path="../globals.d.ts" />

declare const CI: string | undefined;

// test.runIf / test.skipIf return a TestAPI, callable as `(name, fn)` with the usual shape.
test.runIf(1 === 1)('runs when true', () => {});
test.skipIf(CI)('skipped on CI', ({ skip }) => {
  void skip;
});
// chainable: TestAPI result keeps the TestAPI surface (skip, only, each, concurrent, …).
test.runIf(true).skip('still a TestAPI', () => {});
test.skipIf(false).concurrent('still a TestAPI', () => {});
test.runIf(1).each([1, 2, 3])('n = %i', (n) => {
  const _n: number = n;
  void _n;
});

// describe.runIf / describe.skipIf return a SuiteAPI, callable as `(name, fn)`.
describe.runIf(1)('suite runs when truthy', () => {});
describe.skipIf(null)('suite skipped when truthy', () => {});
describe.runIf(true).skip('still a SuiteAPI', () => {});

// condition accepts anything (unknown) — booleans, numbers, strings, functions.
test.runIf('production' === 'production')('string compare', () => {});
test.skipIf(() => true)('function ok', () => {});
test.runIf(CI)('env var ok', () => {});
