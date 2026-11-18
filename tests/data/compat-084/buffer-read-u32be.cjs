const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
testCase('Buffer readUInt32BE returns actual unsigned big endian numbers', () => {
  assert.equal(Buffer.from([1, 2, 3, 4]).readUInt32BE(), 0x01020304);
  assert.equal(Buffer.from([255, 255, 255, 255]).readUInt32BE(0), 4294967295);
  assert.equal(Buffer.from([128, 0, 0, 1]).readUInt32BE(0), 2147483649);
  assert.equal(Buffer.alloc(4).readUInt32BE(0), 0);
});
testCase('Buffer readUInt32BE uses byte offsets including the final legal position', () => {
  const bytes = Buffer.from([9, 1, 2, 3, 4, 5]);
  assert.equal(bytes.readUInt32BE(1), 0x01020304);
  assert.equal(bytes.readUInt32BE(2), 0x02030405);
  assert.equal(bytes.readUInt32BE(-0), 0x09010203);
});
testCase('Buffer readUInt32BE reads only within its own subarray view', () => {
  const backing = Buffer.from([255, 255, 1, 2, 3, 4, 255, 255]);
  const view = backing.subarray(2, 6);
  assert.equal(view.readUInt32BE(0), 0x01020304);
  assert.throws(() => view.readUInt32BE(1), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  assert.throws(() => backing.subarray(2, 5).readUInt32BE(0), { name: 'RangeError', code: 'ERR_BUFFER_OUT_OF_BOUNDS' });
});
testCase('Buffer readUInt32BE rejects invalid offset types and fractional values', () => {
  const bytes = Buffer.alloc(8);
  for (const offset of ['0', null, true, {}, 0n]) assert.throws(() => bytes.readUInt32BE(offset), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
  for (const offset of [-1, 1.5, NaN, Infinity, -Infinity, 5, 4294967296]) assert.throws(() => bytes.readUInt32BE(offset), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
});
testCase('Buffer readUInt32BE preserves Node short-buffer error precedence', () => {
  for (const length of [0, 1, 2, 3]) {
    const bytes = Buffer.alloc(length);
    for (const offset of [0, 1, -1, Infinity]) assert.throws(() => bytes.readUInt32BE(offset), { name: 'RangeError', code: 'ERR_BUFFER_OUT_OF_BOUNDS' });
    for (const offset of [NaN, 0.5]) assert.throws(() => bytes.readUInt32BE(offset), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  }
});
