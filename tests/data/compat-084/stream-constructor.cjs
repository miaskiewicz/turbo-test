const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const Stream = require('node:stream');
const EventEmitter = require('node:events');
const { inherits } = require('node:util');

testCase('stream module is the callable Stream constructor under both aliases', () => {
  assert.equal(typeof Stream, 'function');
  assert.equal(require('stream'), Stream);
  assert.equal(Stream.Stream, Stream);
  assert.equal(Stream.prototype.constructor, Stream);
});
testCase('modern class extends the imported stream module and delivers real events', () => {
  class Request extends Stream { constructor() { super(); this.method = 'GET'; } }
  const request = new Request(); const seen = [];
  assert.equal(request instanceof Stream, true);
  assert.equal(request instanceof EventEmitter, true);
  request.once('response', function(value) { assert.equal(this, request); seen.push(value); });
  assert.equal(request.emit('response', 42), true);
  assert.equal(request.emit('response', 43), false);
  assert.deepEqual(seen, [42]);
});
testCase('legacy Stream.call and util.inherits initialize independent emitter state', () => {
  function Request(method) { Stream.call(this); this.method = method; }
  inherits(Request, Stream);
  const first = new Request('GET'); const second = new Request('POST'); const seen = [];
  assert.equal(first instanceof Stream, true);
  assert.equal(first instanceof EventEmitter, true);
  first.on('data', (value) => seen.push(value));
  assert.equal(second.emit('data', 'wrong'), false);
  assert.equal(first.emit('data', 'bytes'), true);
  assert.deepEqual(seen, ['bytes']);
  assert.equal(second.method, 'POST');
});
testCase('Readable Writable Duplex Transform and PassThrough inherit the exported Stream', () => {
  const types = [Stream.Readable, Stream.Writable, Stream.Duplex, Stream.Transform, Stream.PassThrough];
  for (const Type of types) {
    const stream = new Type();
    assert.equal(stream instanceof Stream, true);
    assert.equal(stream instanceof EventEmitter, true);
    stream.destroy();
  }
});
testCase('legacy stream pipe forwards bytes and completes the destination', () => {
  const source = new Stream(); const destination = new Stream(); const seen = [];
  source.readable = true; destination.writable = true;
  destination.write = (chunk) => { seen.push(chunk); return true; };
  destination.end = () => { seen.push('finished'); };
  assert.equal(source.pipe(destination), destination);
  source.emit('data', 'one'); source.emit('data', 'two'); source.emit('end');
  assert.deepEqual(seen, ['one', 'two', 'finished']);
});
testCase('default and named stream imports retain constructor and implementation identity', async () => {
  const plain = await import('stream'); const prefixed = await import('node:stream');
  assert.equal(plain.default, Stream);
  assert.equal(prefixed.default, Stream);
  assert.equal(plain.Stream, Stream);
  assert.equal(prefixed.Readable, Stream.Readable);
  assert.equal(prefixed.Writable, Stream.Writable);
});
