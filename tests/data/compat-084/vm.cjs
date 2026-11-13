const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');

testCase('runInNewContext evaluates with injected bindings and copies global writes back', () => {
  const sandbox = { answer: 40, removed: true };
  assert.equal(vm.runInNewContext('answer += 2; var added = "yes"; delete removed; answer', sandbox), 42);
  assert.equal(sandbox.answer, 42);
  assert.equal(sandbox.added, 'yes');
  assert.equal('removed' in sandbox, false);
  const changed = {};
  vm.runInNewContext('globalThis = { replacement: true }; var stillGlobal = 7', changed);
  assert.equal(changed.globalThis.replacement, true);
  assert.equal(changed.stillGlobal, 7);
  const empty = {};
  vm.runInNewContext('1', empty);
  assert.deepEqual(Reflect.ownKeys(empty), []);
  assert.deepEqual(Object.getOwnPropertyDescriptors(empty), {});
  const shadow = { Math: 7 };
  assert.equal(vm.runInNewContext('Math', shadow), 7);
  delete shadow.Math;
  assert.equal(vm.runInNewContext('typeof Math', shadow), 'object');
  assert.equal(Object.hasOwn(shadow, 'Math'), false);
  const immutable = {};
  Object.defineProperty(immutable, 'undefined', { value: undefined });
  Object.defineProperty(immutable, 'NaN', { value: NaN });
  assert.equal(vm.runInNewContext('undefined', immutable), undefined);
  assert.equal(Number.isNaN(vm.runInNewContext('NaN', immutable)), true);
});
testCase('new VM contexts expose pristine globals and isolate intrinsic prototypes', () => {
  const hidden = vm.runInNewContext('[typeof process, typeof require, typeof test, typeof expect, typeof document, typeof __ttStructuredClone]');
  assert.deepEqual(Array.from(hidden), Array(6).fill('undefined'));
  vm.runInNewContext('Array.prototype.vmOnly = true');
  assert.equal(Array.prototype.vmOnly, undefined);
  assert.equal(vm.runInNewContext('Array.prototype.vmOnly'), undefined);
});
testCase('VM bindings preserve object references cycles and callable values', () => {
  const value = { n: 1 }; value.self = value;
  const sandbox = { value, twice: (n) => n * 2 };
  const result = vm.runInNewContext('value.n = twice(3); value', sandbox);
  assert.equal(result, value); assert.equal(result.self, value); assert.equal(value.n, 6);
});
testCase('reusing a sandbox retains its context and lexical declarations', () => {
  const sandbox = {};
  assert.equal(vm.isContext(sandbox), false);
  vm.runInNewContext('let count = 1; globalThis.label = "kept"', sandbox);
  assert.equal(vm.isContext(sandbox), true);
  assert.equal(vm.runInNewContext('++count', sandbox), 2);
  assert.equal(vm.runInNewContext('label', sandbox), 'kept');
  assert.equal(vm.runInNewContext('typeof count', {}), 'undefined');
});
testCase('VM returns real realm values instead of JSON replacements', () => {
  assert.equal(vm.runInNewContext('12345678901234567890n'), 12345678901234567890n);
  const result = vm.runInNewContext('({ map: new Map([["x", 42]]), view: new Uint16Array([1, 65535]) })');
  assert.equal(result.map.get('x'), 42);
  assert.deepEqual(Array.from(result.view), [1, 65535]);
});
testCase('VM propagates syntax and runtime exceptions and keeps prior writes', () => {
  assert.throws(() => vm.runInNewContext('}'), { name: 'SyntaxError' });
  const sandbox = {};
  assert.throws(() => vm.runInNewContext('written = 42; throw new TypeError("vm failure")', sandbox), { name: 'TypeError', message: 'vm failure' });
  assert.equal(sandbox.written, 42);
  assert.equal(vm.runInNewContext('written', sandbox), 42);
});
testCase('VM timeout interrupts execution and leaves the runner usable', () => {
  const sandbox = {};
  assert.throws(() => vm.runInNewContext('started = true; while (true) {}', sandbox, { timeout: 10 }), { code: 'ERR_SCRIPT_EXECUTION_TIMEOUT' });
  assert.equal(sandbox.started, true);
  assert.equal(vm.runInNewContext('6 * 7', sandbox, { timeout: 1000 }), 42);
  assert.throws(() => vm.runInNewContext('while (true) {}', {}, { timeout: 10 }), { code: 'ERR_SCRIPT_EXECUTION_TIMEOUT' });
  assert.equal(vm.runInNewContext('21 * 2'), 42);
});
testCase('VM validates code sandbox and timeout arguments', () => {
  assert.equal(vm.runInNewContext(1), 1);
  for (const sandbox of [null, 1, 'sandbox', () => {}]) {
    assert.throws(() => vm.runInNewContext('1', sandbox), { code: 'ERR_INVALID_ARG_TYPE' });
  }
  for (const timeout of [0, -1, 0.5, Infinity]) {
    assert.throws(() => vm.runInNewContext('1', {}, { timeout }), { code: 'ERR_OUT_OF_RANGE' });
  }
  assert.throws(() => vm.runInNewContext('1', {}, { timeout: '1' }), { code: 'ERR_INVALID_ARG_TYPE' });
});
