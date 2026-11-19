const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
// compress-commons (zip-stream / archiver) writes zip headers with these.
test('8, 16 and 32-bit writes lay out bytes and return the next offset', () => {
  const b = Buffer.alloc(16);
  assert.equal(b.writeUInt8(0xab), 1);
  assert.equal(b.writeInt8(-2, 1), 2);
  assert.equal(b.writeUInt16LE(0x1234, 2), 4);
  assert.equal(b.writeUInt16BE(0x1234, 4), 6);
  assert.equal(b.writeInt16LE(-2, 6), 8);
  assert.equal(b.writeInt32LE(-2, 8), 12);
  assert.equal(b.writeUInt32LE(0xdeadbeef, 12), 16);
  assert.equal(b.toString('hex'), 'abfe34121234feff' + 'feffffff' + 'efbeadde');
  assert.equal(Buffer.alloc(2).writeUInt16LE('258'), 2);
});
test('8, 16 and 32-bit reads decode both byte orders and signs', () => {
  const b = Buffer.from('abfe34121234feff' + 'feffffff' + 'efbeadde' + '0080', 'hex');
  assert.deepEqual(
    [b.readUInt8(), b.readUInt8(0), b.readInt8(1), b.readUInt16LE(2), b.readUInt16BE(4), b.readInt16LE(6), b.readInt32LE(8), b.readUInt32LE(12), b.readUInt32BE(12), b.readInt16LE(16), b.readUInt16BE(16)],
    [0xab, 0xab, -2, 0x1234, 0x1234, -2, -2, 0xdeadbeef, 0xefbeadde, -32768, 0x80],
  );
});
test('lower-case Uint aliases are the same functions', () => {
  const p = Buffer.prototype;
  for (const name of ['writeUInt8', 'writeUInt16LE', 'writeUInt16BE', 'writeUInt32LE', 'writeUInt32BE', 'readUInt8', 'readUInt16LE', 'readUInt16BE', 'readUInt32LE', 'readUInt32BE']) {
    assert.equal(typeof p[name], 'function', name);
    assert.equal(p[name.replace('UInt', 'Uint')], p[name], name);
  }
});
test('range and bounds errors carry Node codes', () => {
  const b = Buffer.alloc(16);
  assert.throws(() => b.writeUInt8(256), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  assert.throws(() => b.writeInt8(-129), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  assert.throws(() => b.writeUInt16LE(1, 15), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  assert.throws(() => b.readUInt16LE(15), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  assert.throws(() => b.readInt32LE(1.5), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  assert.throws(() => b.readUInt8('1'), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => Buffer.alloc(1).readUInt16LE(0), { name: 'RangeError', code: 'ERR_BUFFER_OUT_OF_BOUNDS' });
});
