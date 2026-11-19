const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
// Buffer.isEncoding (#434): Node 26's normalizeEncoding, case-insensitive, strings only.
test('known encodings and their aliases in any case', () => {
  for (const name of ['utf8', 'UTF8', 'utf-8', 'Utf-8', 'hex', 'HEX', 'ucs2', 'ucs-2', 'UCS-2', 'utf16le', 'utf-16le', 'UTF-16LE',
    'latin1', 'Latin1', 'binary', 'BINARY', 'base64', 'BASE64', 'base64url', 'BASE64URL', 'ascii', 'ASCII']) {
    assert.equal(Buffer.isEncoding(name), true, name);
  }
});
test('unknown names and non-strings are false', () => {
  for (const value of ['', 'utf', 'utf16', 'utf32', ' utf8', 'utf8 ', 'utf-16', 'base-64', 'latin-1', null, undefined, 1, {}, ['utf8'], new String('utf8'), Symbol('utf8')]) {
    assert.equal(Buffer.isEncoding(value), false, String(value));
  }
});
test('isEncoding is an enumerable static of length 1', () => {
  const descriptor = Object.getOwnPropertyDescriptor(Buffer, 'isEncoding');
  assert.equal(descriptor.enumerable, true);
  assert.equal(descriptor.writable, true);
  assert.equal(Buffer.isEncoding.length, 1);
  assert.equal(Buffer.isEncoding.name, 'isEncoding');
  assert.equal(require('node:buffer').Buffer.isEncoding('utf8'), true);
});
