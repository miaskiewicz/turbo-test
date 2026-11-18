const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const webcrypto = require('node:crypto').webcrypto;
const get = (value) => webcrypto.getRandomValues(value);
testCase('getRandomValues accepts every integer typed array and retains identity', () => {
  const constructors = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array];
  if (typeof BigInt64Array !== 'undefined') constructors.push(BigInt64Array, BigUint64Array);
  for (const Constructor of constructors) {
    const value = new Constructor(8);
    assert.equal(get(value), value);
    assert.equal(value.length, 8);
  }
  const buffer = Buffer.alloc(8);
  assert.equal(get(buffer), buffer);
});
testCase('getRandomValues rejects floating point arrays DataView and non typed arrays', () => {
  const invalid = [new Float32Array(2), new Float64Array(2), new DataView(new ArrayBuffer(8))];
  if (typeof Float16Array !== 'undefined') invalid.push(new Float16Array(2));
  for (const value of invalid) assert.throws(() => get(value), { name: 'TypeMismatchError' });
  for (const value of [new ArrayBuffer(8), {}, [], null, undefined, 2, 'bytes']) assert.throws(() => get(value), { name: 'TypeError' });
  assert.throws(() => get(new Float32Array(20000)), { name: 'TypeMismatchError' });
});
testCase('getRandomValues enforces the quota in bytes rather than element count', () => {
  for (const value of [new Uint8Array(65536), new Uint16Array(32768), new Uint32Array(16384)]) assert.equal(get(value), value);
  for (const value of [new Uint8Array(65537), new Uint16Array(32769), new Uint32Array(16385)]) assert.throws(() => get(value), { name: 'QuotaExceededError' });
  if (typeof BigUint64Array !== 'undefined') assert.throws(() => get(new BigUint64Array(8193)), { name: 'QuotaExceededError' });
});
testCase('getRandomValues checks the real typed array brand and honors subarray boundaries', () => {
  const floating = new Float32Array(2);
  Object.defineProperty(floating, Symbol.toStringTag, { value: 'Uint8Array' });
  assert.throws(() => get(floating), { name: 'TypeMismatchError' });
  const backing = new Uint8Array(16).fill(170); const view = backing.subarray(4, 12);
  assert.equal(get(view), view);
  assert.deepEqual([...backing.subarray(0, 4)], [170, 170, 170, 170]);
  assert.deepEqual([...backing.subarray(12)], [170, 170, 170, 170]);
});
testCase('getRandomValues accepts integer arrays created in a different VM context', () => {
  const value = require('node:vm').runInNewContext('new Uint32Array(4)');
  assert.equal(get(value), value);
  assert.equal(value.byteLength, 16);
});
