const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
function warningFrom(action) {
  return new Promise((resolve, reject) => {
    function listener(warning) { resolve({ warning, receiver: this }); }
    process.once('warning', listener);
    try { assert.equal(action(), undefined); }
    catch (error) { process.off('warning', listener); reject(error); }
  });
}
testCase('emitWarning creates real Warning errors and delivers on nextTick', async () => {
  const order = [];
  process.nextTick(() => order.push('before'));
  let synchronous = true;
  const seen = warningFrom(() => process.emitWarning('warning-string-fixture'));
  seen.then(() => order.push('promise'));
  function listener(warning) { assert.equal(synchronous, false); order.push(warning.message); }
  process.once('warning', listener);
  process.nextTick(() => order.push('after'));
  assert.deepEqual(order, []); synchronous = false;
  const result = await seen;
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(result.receiver, process);
  assert.equal(result.warning instanceof Error, true);
  assert.equal(result.warning.name, 'Warning');
  assert.equal(result.warning.message, 'warning-string-fixture');
  assert.deepEqual(order.slice(0, 3), ['before', 'warning-string-fixture', 'after']);
});
testCase('emitWarning preserves Error identity and existing warning metadata', async () => {
  const original = new Error('warning-error-fixture'); original.name = 'ExistingWarning'; original.code = 'EXISTING';
  const result = await warningFrom(() => process.emitWarning(original, { type: 'IgnoredType', code: 'IGNORED' }));
  assert.equal(result.warning, original);
  assert.equal(result.warning.name, 'ExistingWarning'); assert.equal(result.warning.code, 'EXISTING');
});
testCase('emitWarning supports positional type code and constructor arguments', async () => {
  function warningCaller() { return process.emitWarning('warning-positional-fixture', 'CustomWarning', 'TT_POSITIONAL', warningCaller); }
  const result = await warningFrom(warningCaller);
  assert.equal(result.warning.name, 'CustomWarning'); assert.equal(result.warning.code, 'TT_POSITIONAL');
  assert.equal(result.warning.message, 'warning-positional-fixture');
  assert.equal(typeof result.warning.stack, 'string');
  assert.equal(result.warning.stack.includes('at warningCaller'), false);
});
testCase('emitWarning supports options detail and function constructor overloads', async () => {
  const result = await warningFrom(() => process.emitWarning('warning-options-fixture', { type: 'OptionWarning', code: 'TT_OPTIONS', detail: 'warning-detail-fixture' }));
  assert.equal(result.warning.name, 'OptionWarning'); assert.equal(result.warning.code, 'TT_OPTIONS');
  assert.equal(result.warning.detail, 'warning-detail-fixture');
  function caller() { return process.emitWarning('warning-constructor-fixture', caller); }
  assert.equal((await warningFrom(caller)).warning.name, 'Warning');
});
testCase('emitWarning validates warning type and code inputs like Node', () => {
  for (const value of [undefined, null, {}, [], 1, true]) assert.throws(() => process.emitWarning(value), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
  for (const type of [null, 1, true, []]) assert.throws(() => process.emitWarning('bad-type', type), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
  for (const code of [null, 1, true, {}]) assert.throws(() => process.emitWarning('bad-code', 'Warning', code), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
});
testCase('process warning listeners support once removal and process receiver', async () => {
  let removedCalls = 0; let onceCalls = 0;
  function removed() { removedCalls++; }
  function onceListener() { assert.equal(this, process); onceCalls++; }
  assert.equal(process.on('warning', removed), process); assert.equal(process.off('warning', removed), process);
  assert.equal(process.once('warning', onceListener), process);
  await warningFrom(() => process.emitWarning('first-listener-warning'));
  await warningFrom(() => process.emitWarning('second-listener-warning'));
  assert.equal(removedCalls, 0); assert.equal(onceCalls, 1);
});
testCase('warnings remain visible on stderr without a user warning listener', async () => {
  process.emitWarning('TT_WARNING_STDERR_VISIBLE', { code: 'TT_WARNING_CODE', detail: 'TT_WARNING_DETAIL_VISIBLE' });
  await new Promise((resolve) => setImmediate(resolve));
});
