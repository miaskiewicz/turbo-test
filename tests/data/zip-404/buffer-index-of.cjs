const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
// Zip readers find signatures with lastIndexOf / includes on Buffer needles (#404).
const b = Buffer.from('abcabc');
test('indexOf finds strings, buffers and bytes from a byte offset', () => {
  assert.deepEqual(
    [b.indexOf('bc'), b.indexOf('bc', 2), b.indexOf('bc', -3), b.indexOf('bc', -100), b.indexOf('bc', 100),
      b.indexOf('bc', null), b.indexOf('bc', NaN), b.indexOf('bc', 1.7), b.indexOf('abcabcd'),
      b.indexOf(Buffer.from('ca')), b.indexOf(new Uint8Array([99, 97])), b.indexOf('YmM=', 'base64'),
      b.indexOf(98), b.indexOf(98 + 256), b.indexOf(-158), b.indexOf(98.9), Buffer.alloc(0).indexOf(0)],
    [1, 4, 4, 1, -1, 1, 1, 1, -1, 2, 2, 1, 1, 1, 1, 1, -1],
  );
});
test('empty needles point at the clamped offset', () => {
  assert.deepEqual([b.indexOf(''), b.indexOf('', 4), b.indexOf('', 100), b.lastIndexOf(''), b.lastIndexOf('', 2), b.indexOf('', -100)], [0, 4, 6, 6, 2, 0]);
});
test('lastIndexOf searches backwards from the offset', () => {
  assert.deepEqual(
    [b.lastIndexOf('bc'), b.lastIndexOf('bc', 3), b.lastIndexOf('bc', -2), b.lastIndexOf('bc', -100),
      b.lastIndexOf(98), b.lastIndexOf('bc', NaN), b.lastIndexOf('bc', undefined), b.lastIndexOf('bc', null),
      b.lastIndexOf('bc', 100), b.lastIndexOf(Buffer.from([0x50, 0x4b])), b.lastIndexOf(97, 2)],
    [4, 1, 4, -1, 4, 4, 4, -1, 4, -1, 0],
  );
});
test('includes reports whether indexOf finds the value', () => {
  assert.equal(b.includes('ca'), true);
  assert.equal(b.includes('ca', 3), false);
  assert.equal(b.includes(Buffer.from([0x01, 0x99])), false);
  assert.equal(Buffer.from([0, 0x01, 0x99, 0]).includes(Buffer.from([0x01, 0x99])), true);
  assert.equal(b.includes(99), true);
});
test('invalid needles and encodings throw Node errors', () => {
  assert.throws(() => b.indexOf({}), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => b.includes([1]), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => b.indexOf('bc', '1'), { name: 'TypeError', code: 'ERR_UNKNOWN_ENCODING' });
});
