const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');

testCase('Script rejects invalid script grammar during construction', () => {
  assert.equal(vm.Script, require('vm').Script);
  for (const source of ['}', 'let value =', 'return 42', 'await 1']) {
    assert.throws(() => new vm.Script(source), { name: 'SyntaxError' });
  }
});
testCase('Script compilation does not execute and coerces source only once', () => {
  let conversions = 0;
  globalThis.__vmCompileSideEffect364 = 0;
  try {
    const code = { toString() { conversions++; return 'globalThis.__vmCompileSideEffect364 += 1'; } };
    const script = new vm.Script(code);
    assert.equal(conversions, 1);
    assert.equal(globalThis.__vmCompileSideEffect364, 0);
    assert.equal(script.runInThisContext(), 1);
    assert.equal(script.runInThisContext(), 2);
    assert.equal(conversions, 1);
  } finally { delete globalThis.__vmCompileSideEffect364; }
});
testCase('Script syntax errors retain the constructor filename', () => {
  for (const options of ['vm-eager-364.js', { filename: 'vm-eager-364.js' }]) {
    let failure;
    try { new vm.Script('}', options); } catch (error) { failure = error; }
    assert.equal(failure?.name, 'SyntaxError');
    assert.ok(String(failure.stack).includes('vm-eager-364.js'));
  }
});
testCase('one Script runs repeatedly in independent contexts without construction effects', () => {
  const left = vm.createContext({ count: 0, step: 2 });
  const right = vm.createContext({ count: 10, step: 3 });
  const script = new vm.Script('count += step; count', { filename: 'vm-reuse-364.js' });
  assert.equal(left.count, 0);
  assert.equal(right.count, 10);
  assert.equal(script.runInContext(left), 2);
  assert.equal(script.runInContext(right), 13);
  assert.equal(script.runInContext(left), 4);
  assert.equal(script.runInNewContext({ count: 20, step: 4 }), 24);
});
testCase('Script contextual runtime errors use its compile filename, not run options', () => {
  const script = new vm.Script('throw new Error("runtime origin 364")', { filename: 'vm-compile-origin-364.js' });
  const context = vm.createContext({});
  for (const run of [() => script.runInContext(context, { filename: 'ignored-run-origin.js' }), () => script.runInNewContext({}, { filename: 'ignored-run-origin.js' })]) {
    let failure;
    try { run(); } catch (error) { failure = error; }
    assert.equal(failure?.message, 'runtime origin 364');
    assert.ok(String(failure.stack).includes('vm-compile-origin-364.js'));
    assert.equal(String(failure.stack).includes('ignored-run-origin.js'), false);
  }
});
testCase('Script constructor accepts numeric code and validates options and symbols', () => {
  assert.equal(new vm.Script(42).runInThisContext(), 42);
  assert.throws(() => new vm.Script(Symbol('code')), { name: 'TypeError' });
  for (const options of [null, 2]) assert.throws(() => new vm.Script('1', options), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => new vm.Script('1', { filename: 2 }), { code: 'ERR_INVALID_ARG_TYPE' });
});
