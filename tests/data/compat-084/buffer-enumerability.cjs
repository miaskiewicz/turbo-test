const assert = require('node:assert/strict');
const { Buffer: NodeBuffer } = require('node:buffer');
const { StringDecoder } = require('node:string_decoder');

const publicStatics = ['from', 'alloc', 'allocUnsafe', 'byteLength', 'isBuffer', 'concat'];

test('Buffer public statics retain Node enumerable writable configurable descriptors', () => {
  assert.equal(NodeBuffer, Buffer);
  for (const name of publicStatics) {
    const descriptor = Object.getOwnPropertyDescriptor(NodeBuffer, name);
    assert.equal(typeof descriptor.value, 'function', name);
    assert.equal(descriptor.enumerable, true, name);
    assert.equal(descriptor.writable, true, name);
    assert.equal(descriptor.configurable, true, name);
  }
  assert.equal(Object.getOwnPropertyDescriptor(NodeBuffer, 'length').enumerable, false);
  assert.equal(Object.getOwnPropertyDescriptor(NodeBuffer, 'prototype').enumerable, false);
});

test('enumeration consumers can validate and decode actual JSON Buffer bytes', () => {
  const copied = {};
  for (const name in NodeBuffer) {
    if (Object.prototype.hasOwnProperty.call(NodeBuffer, name)) copied[name] = NodeBuffer[name];
  }
  for (const name of publicStatics) assert.equal(copied[name], NodeBuffer[name], name);
  const bytes = NodeBuffer.from('{"rating":"positive"}');
  assert.equal(copied.isBuffer(bytes), true);
  assert.equal(copied.isBuffer(new Uint8Array(bytes)), false);
  assert.equal(copied.byteLength(bytes), 21);
  const decoder = new StringDecoder('utf8');
  const json = decoder.write(copied.from(bytes)) + decoder.end();
  assert.deepEqual(JSON.parse(json), { rating: 'positive' });
});
