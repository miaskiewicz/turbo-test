// Compile-only regression guard for Jest's `test.failing` and `.failing.each` (turbo-test#402)
// in the shipped `.d.ts` shim. NOT run by the test runner — checked by
// `test/compat-types.test.mjs` via `tsc --noEmit --strict`.
/// <reference path="../globals.d.ts" />

test.failing('body throws', () => {
  throw new Error('expected');
});
it.failing('async rejection', async () => {
  await Promise.reject(new Error('expected'));
}, 50);
test.only.failing('only', () => {});
test.skip.failing('skip', () => {});
it.failing.each([
  [1, 2],
  [3, 4],
])('row %i plus %i', (a, b) => {
  const _sum: number = a + b;
  void _sum;
});
