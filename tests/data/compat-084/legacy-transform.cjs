const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { Transform, Duplex, Readable, Writable } = require('node:stream');
const { pipeline } = require('node:stream/promises');
const { inherits } = require('node:util');

function LegacyTransform(options) { Transform.call(this, options); }
inherits(LegacyTransform, Transform);
function collect(output) {
  return new Writable({ write(chunk, encoding, callback) { output.push(chunk.toString()); callback(); } });
}

testCase('legacy Transform.call and util.inherits transform actual bytes and flush', async () => {
  const output = [];
  const stream = new LegacyTransform({
    transform(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); },
    flush(callback) { callback(null, '!'); },
  });
  assert.equal(stream instanceof LegacyTransform, true);
  assert.equal(stream instanceof Transform, true);
  assert.equal(stream instanceof Duplex, true);
  assert.equal(stream instanceof Readable, true);
  assert.equal(stream.writable, true);
  await pipeline(Readable.from(['ab', 'cd']), stream, collect(output));
  assert.equal(output.join(''), 'ABCD!');
  assert.equal(stream.readableEnded, true);
  assert.equal(stream.writableFinished, true);
  assert.equal(stream.destroyed, true);
  assert.equal(stream.closed, true);
});

testCase('ES class Transform subclasses retain real streaming lifecycle', async () => {
  class Prefix extends Transform {
    constructor() { super({ objectMode: true }); }
    _transform(value, encoding, callback) { callback(null, value + 10); }
  }
  const stream = new Prefix(); const output = [];
  const destination = new Writable({ objectMode: true, write(value, encoding, callback) { output.push(value); callback(); } });
  assert.equal(stream instanceof Prefix, true);
  assert.equal(stream instanceof Duplex, true);
  assert.equal(stream.readableHighWaterMark, 16);
  assert.equal(stream.writableHighWaterMark, 16);
  await pipeline(Readable.from([1, 2]), stream, destination);
  assert.deepEqual(output, [11, 12]);
});

testCase('legacy Transform instances keep options and emitter state independent', async () => {
  const first = new LegacyTransform({ transform(chunk, encoding, callback) { callback(null, chunk); } });
  const second = new LegacyTransform({ transform(chunk, encoding, callback) { callback(null, Buffer.from('second')); } });
  const events = []; first.on('custom', (value) => events.push(value));
  assert.equal(second.emit('custom', 'wrong'), false);
  first.emit('custom', 'first');
  const left = []; const right = [];
  await Promise.all([
    pipeline(Readable.from(['left']), first, collect(left)),
    pipeline(Readable.from(['right']), second, collect(right)),
  ]);
  assert.deepEqual(events, ['first']);
  assert.equal(left.join(''), 'left');
  assert.equal(right.join(''), 'second');
});

testCase('legacy Transform propagates the original callback error and closes once', async () => {
  const failure = new Error('transform rejected bytes');
  const stream = new LegacyTransform({ transform(chunk, encoding, callback) { callback(failure); } });
  let closes = 0; stream.on('close', () => { closes++; });
  await assert.rejects(pipeline(Readable.from(['input']), stream, collect([])), (error) => error === failure);
  assert.equal(stream.errored, failure);
  assert.equal(stream.destroyed, true);
  assert.equal(stream.closed, true);
  assert.equal(closes, 1);
});
