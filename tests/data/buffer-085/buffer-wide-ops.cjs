const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const fails = (fn, name, code, message) => assert.throws(fn, (e) => e.name === name && e.code === code && e.message === message);
const range = (fn, message) => fails(fn, 'RangeError', 'ERR_OUT_OF_RANGE', message);
const type = (fn, message) => fails(fn, 'TypeError', 'ERR_INVALID_ARG_TYPE', message);
const bounds = (fn) => fails(fn, 'RangeError', 'ERR_BUFFER_OUT_OF_BOUNDS', 'Attempt to access memory outside buffer bounds');

test('float and double reads and writes in both byte orders', () => {
  const b = Buffer.alloc(16);
  assert.equal(b.writeFloatLE(1.5, 0), 4);
  assert.equal(b.writeDoubleBE(-0.1, 8), 16);
  assert.equal(b.toString('hex'), '0000c03f00000000' + 'bfb999999999999a');
  assert.deepEqual([b.readFloatLE(0), b.readDoubleBE(8), b.readFloatBE(), b.readDoubleLE(8)], [1.5, -0.1, 6.896490392174587e-41, -1.5423487136676073e-180]);
  assert.equal(b.writeFloatBE('2'), 4);
  assert.equal(b.readFloatBE(0), 2);
  b.writeFloatLE({}, 0);
  assert.ok(Number.isNaN(b.readFloatLE(0)));
  b.writeFloatLE(1e40, 0);
  assert.equal(b.readFloatLE(0), Infinity);
  assert.equal(b.writeDoubleLE(2), 8);
  assert.equal(b.readDoubleLE(), 2);
});
test('float bounds and offset errors', () => {
  const b = Buffer.alloc(16);
  range(() => b.writeDoubleLE(1, 13), 'The value of "offset" is out of range. It must be >= 0 and <= 8. Received 13');
  range(() => b.readFloatLE(13), 'The value of "offset" is out of range. It must be >= 0 and <= 12. Received 13');
  range(() => b.readFloatLE(1.5), 'The value of "offset" is out of range. It must be an integer. Received 1.5');
  type(() => b.readFloatLE('1'), 'The "offset" argument must be of type number. Received type string (\'1\')');
  type(() => b.writeFloatLE(1, '1'), 'The "offset" argument must be of type number. Received type string (\'1\')');
  bounds(() => Buffer.alloc(2).readFloatLE(0));
});
test('64-bit BigInt reads and writes', () => {
  const b = Buffer.alloc(16);
  assert.equal(b.writeBigInt64LE(-2n, 0), 8);
  assert.equal(b.readBigInt64LE(0), -2n);
  assert.equal(b.readBigUInt64BE(0), 18374686479671623679n);
  assert.equal(b.writeBigUInt64BE(0x0102030405060708n, 8), 16);
  assert.equal(b.toString('hex', 8), '0102030405060708');
  assert.equal(b.readBigUInt64LE(8), 0x0807060504030201n);
  assert.equal(b.writeBigInt64BE(3n), 8);
  assert.equal(b.readBigInt64BE(), 3n);
  for (const op of ['read', 'write']) for (const end of ['LE', 'BE']) {
    assert.equal(Buffer.prototype[`${op}BigUint64${end}`], Buffer.prototype[`${op}BigUInt64${end}`]);
  }
});
test('64-bit BigInt range and type errors', () => {
  const b = Buffer.alloc(16);
  assert.throws(() => b.writeBigUInt64LE(1, 0), (e) => e instanceof TypeError && e.message === 'Cannot mix BigInt and other types, use explicit conversions');
  range(() => b.writeBigUInt64LE(-1n), 'The value of "value" is out of range. It must be >= 0n and < 2n ** 64n. Received -1n');
  range(() => b.writeBigInt64BE(2n ** 63n), 'The value of "value" is out of range. It must be >= -(2n ** 63n) and < 2n ** 63n. Received 9_223_372_036_854_775_808n');
  range(() => b.writeBigUInt64BE(2n ** 64n), 'The value of "value" is out of range. It must be >= 0n and < 2n ** 64n. Received 18_446_744_073_709_551_616n');
  range(() => b.writeBigInt64BE(-(2n ** 63n) - 1n), 'The value of "value" is out of range. It must be >= -(2n ** 63n) and < 2n ** 63n. Received -9_223_372_036_854_775_809n');
  range(() => b.readBigInt64LE(9), 'The value of "offset" is out of range. It must be >= 0 and <= 8. Received 9');
});
test('variable-length integers of 1 to 6 bytes', () => {
  const b = Buffer.alloc(16);
  assert.equal(b.writeIntLE(-123456, 0, 3), 3);
  assert.equal(b.readIntLE(0, 3), -123456);
  assert.equal(b.writeUIntBE(0xffffffffffff, 0, 6), 6);
  assert.deepEqual([b.readUIntBE(0, 6), b.readIntBE(0, 6), b.readUIntLE(0, 1)], [281474976710655, -1, 255]);
  assert.equal(b.writeIntBE(5, 2, 4), 6);
  assert.equal(b.writeUIntLE(0x0102030405, 8, 5), 13);
  assert.equal(b.toString('hex', 8, 13), '0504030201');
  assert.deepEqual([b.readUIntLE(8, 5), b.readUIntBE(8, 5), b.readIntLE(8, 2), b.readIntBE(8, 2)], [0x0102030405, 0x0504030201, 0x0405, 0x0504]);
  b.writeIntLE(-2, 0, 6);
  assert.deepEqual([b.readIntLE(0, 6), b.readUIntLE(0, 6), b.readIntBE(0, 1)], [-2, 2 ** 48 - 2, -2]);
  assert.deepEqual([b.writeUIntLE('5', 0, 1), b.writeUIntLE(NaN, 1, 1), b.writeUIntLE(1.5, 2, 1)], [1, 2, 3]);
  assert.equal(b.toString('hex', 0, 3), '050001');
  for (const name of ['readUIntLE', 'readUIntBE', 'writeUIntLE', 'writeUIntBE']) {
    assert.equal(Buffer.prototype[name.replace('UInt', 'Uint')], Buffer.prototype[name]);
  }
});
test('variable-length byteLength, value and offset errors', () => {
  const b = Buffer.alloc(16);
  range(() => b.writeUIntLE(1, 0, 7), 'The value of "byteLength" is out of range. It must be >= 1 and <= 6. Received 7');
  range(() => b.readUIntLE(0, 0), 'The value of "byteLength" is out of range. It must be >= 1 and <= 6. Received 0');
  type(() => b.readUIntLE(0), 'The "byteLength" argument must be of type number. Received undefined');
  type(() => b.readUIntLE(0, '2'), 'The "byteLength" argument must be of type number. Received type string (\'2\')');
  range(() => b.readUIntLE(0, 1.5), 'The value of "byteLength" is out of range. It must be an integer. Received 1.5');
  range(() => b.writeIntLE(2 ** 23, 0, 3), 'The value of "value" is out of range. It must be >= -8388608 and <= 8388607. Received 8388608');
  range(() => b.writeUIntBE(-1, 0, 2), 'The value of "value" is out of range. It must be >= 0 and <= 65535. Received -1');
  range(() => b.writeIntBE(2 ** 47, 0, 6), 'The value of "value" is out of range. It must be >= -(2 ** 47) and < 2 ** 47. Received 140_737_488_355_328');
  range(() => b.writeUIntLE(2 ** 40, 0, 5), 'The value of "value" is out of range. It must be >= 0 and < 2 ** 40. Received 1_099_511_627_776');
  range(() => b.writeUIntLE(256, 0, 1), 'The value of "value" is out of range. It must be >= 0 and <= 255. Received 256');
  range(() => b.writeIntLE(2 ** 31, 0, 4), 'The value of "value" is out of range. It must be >= -2147483648 and <= 2147483647. Received 2147483648');
  range(() => b.readUIntLE(15, 2), 'The value of "offset" is out of range. It must be >= 0 and <= 14. Received 15');
  range(() => b.writeUIntLE(1, 15, 2), 'The value of "offset" is out of range. It must be >= 0 and <= 14. Received 15');
  type(() => b.readUIntLE(undefined, 2), 'The "offset" argument must be of type number. Received undefined');
  type(() => b.writeIntBE(1, undefined, 2), 'The "offset" argument must be of type number. Received undefined');
  bounds(() => Buffer.alloc(1).readUIntLE(0, 2));
});
