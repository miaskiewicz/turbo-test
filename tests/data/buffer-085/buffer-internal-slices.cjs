const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
// busboy (multer) decodes with latin1Slice / utf8Slice / ucs2Slice / base64Slice (#418).
const fails = (fn, name, code, message) => assert.throws(fn, (e) => e.name === name && e.code === code && e.message === message);
const index = (fn) => fails(fn, 'RangeError', 'ERR_OUT_OF_RANGE', 'Index out of range');
const outside = (fn, what) => fails(fn, 'RangeError', 'ERR_BUFFER_OUT_OF_BOUNDS', `"${what}" is outside of buffer bounds`);
const encodings = ['latin1', 'ascii', 'base64', 'base64url', 'hex', 'ucs2', 'utf8'];
const b = Buffer.from([0x68, 0xe9, 0x6c, 0x6c, 0x6f, 0xff, 0x00, 0x41]);

test('the internal methods are enumerable prototype functions', () => {
  for (const encoding of encodings) {
    for (const [suffix, length] of [['Slice', 2], ['Write', 3]]) {
      const method = Buffer.prototype[encoding + suffix];
      assert.equal(typeof method, 'function', encoding + suffix);
      assert.equal(method.length, length);
      assert.equal(Object.getOwnPropertyDescriptor(Buffer.prototype, encoding + suffix).enumerable, true);
    }
  }
});
test('slices decode the byte range in their encoding', () => {
  const expected = {
    latin1: ['hélloÿ\u0000A', 'élloÿ\u0000A', 'él'],
    ascii: ['hillo\x7f\u0000A', 'illo\x7f\u0000A', 'il'],
    base64: ['aOlsbG//AEE=', '6Wxsb/8AQQ==', '6Ww='],
    base64url: ['aOlsbG__AEE', '6Wxsb_8AQQ', '6Ww'],
    hex: ['68e96c6c6fff0041', 'e96c6c6fff0041', 'e96c'],
    ucs2: [b.toString('ucs2'), b.toString('ucs2', 1), b.toString('ucs2', 1, 3)],
    utf8: ['h�llo�\u0000A', '�llo�\u0000A', '�l'],
  };
  for (const encoding of encodings) {
    const slice = (...args) => b[encoding + 'Slice'](...args);
    assert.deepEqual([slice(), slice(1), slice(1, 3)], expected[encoding], encoding);
    assert.deepEqual([slice(3, 1), slice(8), slice(9), slice(2 ** 53), slice(Infinity), slice(0, '3').length > 0], ['', '', '', '', '', true], encoding);
    assert.equal(slice('1', 2), slice(1, 2), encoding);
    assert.equal(slice(1.7, 3.2), slice(1, 3), encoding);
    assert.equal(slice(NaN, 2), slice(0, 2), encoding);
    assert.equal(slice(null, 2), slice(0, 2), encoding);
    assert.equal(slice(undefined, 2), slice(0, 2), encoding);
  }
});
test('slice bounds and argument errors', () => {
  for (const encoding of encodings) {
    const slice = Buffer.prototype[encoding + 'Slice'];
    index(() => slice.call(b, 0, 100));
    index(() => slice.call(b, -1));
    index(() => slice.call(b, 1, -1));
    index(() => slice.call(b, 5, 9));
    index(() => slice.call(b, 9, 10));
    fails(() => slice.call({}, 0, 1), 'TypeError', 'ERR_INVALID_ARG_TYPE', 'argument must be a buffer');
    assert.throws(() => slice.call(b, 1n), (e) => e instanceof TypeError && e.message === 'Cannot convert a BigInt value to a number');
    assert.throws(() => slice.call(b, Symbol()), (e) => e instanceof TypeError && e.message === 'Cannot convert a Symbol value to a number');
    assert.equal(slice.call(Buffer.alloc(0), -1), '');
    assert.equal(slice.call(Buffer.alloc(0), 5, 1), '');
  }
  assert.equal(Buffer.prototype.latin1Slice.call(new Uint8Array([65, 66]), 0, 1), 'A');
  assert.equal(Buffer.prototype.hexSlice.call(new DataView(new ArrayBuffer(2))), '0000');
});
test('latin1, ascii and utf8 writes check offset and length against the buffer', () => {
  for (const encoding of ['latin1', 'ascii', 'utf8']) {
    const write = (args, size = 8) => { const t = Buffer.alloc(size); const n = t[encoding + 'Write'](...args); return [n, t.toString('hex')]; };
    assert.deepEqual(write(['abc', 1]), [3, '0061626300000000'], encoding);
    assert.deepEqual(write(['abc', 7]), [1, '0000000000000061']);
    assert.deepEqual(write(['abc', 8]), [0, '0000000000000000']);
    assert.deepEqual(write(['abc', 1.5, 2.5]), [2, '0061620000000000']);
    assert.deepEqual(write(['abcdefgh', 1n], 6), [5, '006162636465']);
    assert.deepEqual(write(['abcdefgh', '2'], 6), [4, '000061626364']);
    assert.deepEqual(write(['abcdefgh', 'x'], 6), [0, '000000000000']);
    assert.deepEqual(write(['abcdefgh', {}, 2], 6), [2, '616200000000']);
    assert.deepEqual(write(['abcdefgh', true, 2], 6), [2, '006162000000']);
    assert.deepEqual(write(['abcdefgh', 0, 'x'], 6), [0, '000000000000']);
    assert.deepEqual(write(['abcdefgh', undefined, 3], 6), [3, '616263000000']);
    outside(() => write(['abc', 9]), 'offset');
    outside(() => write(['abc', -0.5]), 'offset');
    outside(() => write(['abc', Infinity]), 'offset');
    outside(() => write(['abc', 0, 100]), 'length');
    outside(() => write(['abc', 0, -0.5]), 'length');
    outside(() => write(['abc', 8, 1]), 'length');
    fails(() => write([5]), 'TypeError', 'ERR_INVALID_ARG_TYPE', 'argument must be a string');
    fails(() => write([new String('ab')]), 'TypeError', 'ERR_INVALID_ARG_TYPE', 'argument must be a string');
  }
  const t = Buffer.alloc(8);
  assert.deepEqual([t.latin1Write('hé!'), t.toString('hex')], [3, '68e9210000000000']);
  assert.deepEqual([t.asciiWrite('h€x', 0, 2), t.toString('hex', 0, 2)], [2, '68ac']);
  assert.deepEqual([Buffer.alloc(8).utf8Write('hé!'), Buffer.alloc(8).utf8Write('h€x', 0, 2)], [4, 1]);
  fails(() => Buffer.prototype.latin1Write.call({}, 'a'), 'TypeError', 'ERR_INVALID_ARG_TYPE', 'argument must be a buffer');
});
test('base64, base64url, hex and ucs2 writes clamp the length to the buffer', () => {
  const write = (encoding, args, size = 8) => { const t = Buffer.alloc(size); const n = t[encoding + 'Write'](...args); return [n, t.toString('hex')]; };
  for (const encoding of ['base64', 'base64url']) {
    assert.deepEqual(write(encoding, ['aGk=', 2]), [2, '0000686900000000']);
    assert.deepEqual(write(encoding, ['zz', 0]), [1, 'cf00000000000000']);
    assert.deepEqual(write(encoding, ['abc', 0, 100]), [2, '69b7000000000000']);
    assert.deepEqual(write(encoding, ['abc', '1']), [2, '0069b70000000000']);
  }
  assert.deepEqual(write('hex', ['6869', 1, 1]), [1, '0068000000000000']);
  assert.deepEqual(write('hex', ['zz']), [0, '0000000000000000']);
  assert.deepEqual(write('hex', ['abc', 0, 100]), [1, 'ab00000000000000']);
  assert.deepEqual(write('ucs2', ['hé!']), [6, '6800e90021000000']);
  assert.deepEqual(write('ucs2', ['6869', 1, 1]), [0, '0000000000000000']);
  assert.deepEqual(write('ucs2', ['abc', 1.5, 2.5]), [2, '0061000000000000']);
  for (const encoding of ['base64', 'base64url', 'hex', 'ucs2']) {
    assert.deepEqual(write(encoding, ['abc', 8]), [0, '0000000000000000'], encoding);
    assert.deepEqual(write(encoding, ['ab', 0, NaN]), [0, '0000000000000000'], encoding);
    assert.equal(Buffer.alloc(0)[encoding + 'Write']('ab'), 0);
    index(() => write(encoding, ['abc', -1]));
    index(() => write(encoding, ['abcd', 0, -1]));
    outside(() => write(encoding, ['abc', 9]), 'offset');
    fails(() => write(encoding, [5]), 'TypeError', 'ERR_INVALID_ARG_TYPE', 'argument must be a string');
    assert.throws(() => write(encoding, ['ab', 1n]), (e) => e instanceof TypeError && e.message === 'Cannot convert a BigInt value to a number');
  }
});
