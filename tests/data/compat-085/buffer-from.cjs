// Buffer.from over every input kind, as Node v26.11.0 runs it (also passes under node --test):
// ArrayBuffer views share memory and honour byteOffset/length with Node's bounds errors, other
// objects copy element-wise, and unsupported values throw ERR_INVALID_ARG_TYPE.
const test = globalThis.test || require('node:test');
const assert = require('node:assert/strict');

const bytes = (b) => (assert.ok(Buffer.isBuffer(b)), [...b]);
const outOfBounds = (name) => ({ code: 'ERR_BUFFER_OUT_OF_BOUNDS', name: 'RangeError', message: `"${name}" is outside of buffer bounds` });
const invalid = (received) => ({
  code: 'ERR_INVALID_ARG_TYPE',
  name: 'TypeError',
  message: 'The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object. Received ' + received,
});

test('an ArrayBuffer becomes a view honouring byteOffset and length', () => {
  const ab = new Uint8Array([1, 2, 3, 4, 5]).buffer;
  assert.deepEqual(bytes(Buffer.from(ab)), [1, 2, 3, 4, 5]);
  assert.deepEqual(bytes(Buffer.from(ab, 1)), [2, 3, 4, 5]);
  assert.deepEqual(bytes(Buffer.from(ab, 1, 2)), [2, 3]);
  assert.deepEqual(bytes(Buffer.from(ab, '1', '2')), [2, 3]);
  assert.deepEqual(bytes(Buffer.from(ab, NaN, 2)), [1, 2]);
  assert.deepEqual(bytes(Buffer.from(ab, 1, -3)), []);
  const view = Buffer.from(ab, 1, 2);
  new Uint8Array(ab)[1] = 9;
  assert.deepEqual([...view], [9, 3]);
  assert.equal(view.buffer, ab);
  assert.deepEqual(bytes(Buffer.from(new SharedArrayBuffer(2))), [0, 0]);
});

test('ArrayBuffer offsets and lengths outside the buffer throw like Node', () => {
  const ab = new ArrayBuffer(5);
  assert.throws(() => Buffer.from(ab, 6), outOfBounds('offset'));
  assert.throws(() => Buffer.from(ab, 1, 10), outOfBounds('length'));
  assert.throws(() => Buffer.from(ab, -1), RangeError);
});

test('typed arrays, arrays and array-likes are copied with byte coercion', () => {
  const u = new Uint8Array([1, 2]);
  const copy = Buffer.from(u);
  u[0] = 7;
  assert.deepEqual([...copy], [1, 2]);
  assert.deepEqual(bytes(Buffer.from(new Uint16Array([258, 3]))), [2, 3]);
  assert.deepEqual(bytes(Buffer.from({ length: 2, 0: 5, 1: 300 })), [5, 44]);
  assert.deepEqual(bytes(Buffer.from([1, 256, -1, 1.7, 'x'])), [1, 0, 255, 1, 0]);
  assert.deepEqual(bytes(Buffer.from({ type: 'Buffer', data: [1, 2] })), [1, 2]);
  assert.deepEqual(bytes(Buffer.from(new DataView(new ArrayBuffer(3)))), []);
});

test('valueOf and Symbol.toPrimitive objects convert as Node does', () => {
  assert.deepEqual(bytes(Buffer.from(new String('hi'))), [104, 105]);
  assert.deepEqual(bytes(Buffer.from({ [Symbol.toPrimitive]() { return 'ab'; } }, 'utf8')), [97, 98]);
  assert.deepEqual(bytes(Buffer.from(new String('6869'), 'hex')), [104, 105]);
});

test('unsupported values throw ERR_INVALID_ARG_TYPE with Node\'s message', () => {
  assert.throws(() => Buffer.from(5), invalid('type number (5)'));
  assert.throws(() => Buffer.from(null), invalid('null'));
  assert.throws(() => Buffer.from(), invalid('undefined'));
  assert.throws(() => Buffer.from({}), invalid('an instance of Object'));
  assert.throws(() => Buffer.from(Symbol('s')), invalid('type symbol (Symbol(s))'));
});
