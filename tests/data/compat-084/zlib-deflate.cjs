const assert = require('node:assert/strict');
const zlib = require('node:zlib');
const fs = require('node:fs');
const oracle = __ORACLE__;
const clear = Buffer.from(oracle.clear, 'hex');
test('deflate produces compressed bytes and preserves the input', () => {
  const compressed = zlib.deflateSync(clear);
  assert.notDeepEqual(compressed, clear);
  assert.ok(compressed.length < clear.length);
  assert.equal(zlib.inflateSync(compressed).toString('hex'), oracle.clear);
  assert.equal(clear.toString('hex'), oracle.clear);
  fs.writeFileSync(__OUTPUT__, compressed);
});
test('inflate decodes independent Node data and sliced binary views', () => {
  const compressed = Buffer.from(oracle.compressed, 'hex');
  const padded = Buffer.concat([Buffer.from([0]), compressed, Buffer.from([0])]);
  const view = new DataView(padded.buffer, padded.byteOffset + 1, compressed.length);
  assert.equal(zlib.inflateSync(view).toString('hex'), oracle.clear);
  assert.equal(zlib.inflateSync(zlib.deflateSync('hello')).toString(), 'hello');
});
test('empty inputs and explicit compression levels roundtrip', () => {
  assert.equal(zlib.inflateSync(zlib.deflateSync(Buffer.alloc(0))).length, 0);
  for (const level of [0, 1, 9]) {
    assert.equal(zlib.inflateSync(zlib.deflateSync(clear, {level})).toString('hex'), oracle.clear);
  }
  const large = Buffer.alloc(32769, 65);
  assert.equal(zlib.inflateSync(zlib.deflateSync(large)).toString('hex'), large.toString('hex'));
  assert.throws(() => zlib.deflateSync(clear, {level: 10}), RangeError);
  assert.throws(() => zlib.deflateSync(clear, {level: '1'}), TypeError);
  for (const options of [42, true, 'ignored']) assert.equal(zlib.inflateSync(zlib.deflateSync(clear, options)).toString('hex'), oracle.clear);
});
test('malformed and truncated zlib input rejects', () => {
  const compressed = Buffer.from(oracle.compressed, 'hex');
  assert.throws(() => zlib.inflateSync(Buffer.from('not compressed')));
  assert.throws(() => zlib.inflateSync(compressed.subarray(0, compressed.length - 2)));
});
