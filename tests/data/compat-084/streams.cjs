const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const streams = require('node:stream');
const { Readable, Writable, PassThrough, Transform, pipeline, finished } = streams;
const streamPromises = () => require('node:stream/promises');
const later = (fn) => setTimeout(fn, 0);
const callbackPipeline = (...args) => new Promise((resolve, reject) => {
  pipeline(...args, (error) => error ? reject(error) : resolve());
});

function collecting(options = {}) {
  const chunks = [];
  const sink = new Writable({ ...options, write(chunk, encoding, callback) {
    chunks.push(chunk);
    later(callback);
  } });
  return { sink, chunks };
}

testCase('stream promises aliases expose real pipeline and finished functions', () => {
  const plain = require('stream/promises');
  assert.equal(typeof plain.pipeline, 'function');
  assert.equal(typeof plain.finished, 'function');
  assert.equal(plain.pipeline, streamPromises().pipeline);
  assert.equal(plain.finished, streamPromises().finished);
  assert.equal(require('stream').pipeline, pipeline);
});

testCase('callback pipeline waits for bytes, write callbacks and final callback', async () => {
  const writes = [];
  let finalized = false;
  let callbacks = 0;
  const sink = new Writable({
    write(chunk, encoding, callback) {
      assert.equal(Buffer.isBuffer(chunk), true);
      later(() => { writes.push(chunk.toString()); callback(); });
    },
    final(callback) { later(() => { finalized = true; callback(); }); },
  });
  await new Promise((resolve, reject) => {
    const returned = pipeline(Readable.from(['a', Buffer.from('b')]), sink, (error) => {
      callbacks++;
      if (error) return reject(error);
      try {
        assert.deepEqual(writes, ['a', 'b']);
        assert.equal(finalized, true);
        assert.equal(sink.writableFinished, true);
        resolve();
      } catch (failure) { reject(failure); }
    });
    assert.equal(returned, sink);
    assert.deepEqual(writes, []);
  });
  assert.equal(callbacks, 1);
});

testCase('promise pipeline processes read option and resolves only after destination finish', async () => {
  let reading = false;
  const source = new Readable({ read() {
    if (reading) return;
    reading = true;
    later(() => { this.push(Buffer.from('read')); this.push(null); });
  } });
  const { sink, chunks } = collecting();
  assert.equal(await streamPromises().pipeline(source, sink), undefined);
  assert.equal(Buffer.concat(chunks).toString(), 'read');
  assert.equal(source.readableEnded, true);
  assert.equal(sink.writableFinished, true);
});

testCase('pipeline supports array input and multiple real transform stages', async () => {
  const upper = new Transform({ transform(chunk, encoding, callback) {
    later(() => callback(null, chunk.toString().toUpperCase()));
  } });
  const { sink, chunks } = collecting();
  await streamPromises().pipeline([Readable.from(['a', 'b']), new PassThrough(), upper, sink]);
  assert.equal(Buffer.concat(chunks).toString(), 'AB');
});

testCase('Readable.from preserves object-mode chunks from async iterables', async () => {
  const first = { id: 1 };
  const second = { id: 2 };
  async function* values() { yield first; await Promise.resolve(); yield second; }
  const { sink, chunks } = collecting({ objectMode: true });
  await streamPromises().pipeline(Readable.from(values()), sink);
  assert.equal(chunks[0], first);
  assert.equal(chunks[1], second);
});

testCase('Writable queues asynchronous writes, emits drain and runs callbacks in order', async () => {
  let active = 0;
  const received = [];
  const callbacks = [];
  const sink = new Writable({ highWaterMark: 1, write(chunk, encoding, callback) {
    active++;
    assert.equal(active, 1);
    later(() => { received.push(chunk.toString()); active--; callback(); });
  } });
  let drains = 0;
  sink.on('drain', () => drains++);
  assert.equal(sink.write('a', () => callbacks.push('a')), false);
  assert.equal(sink.write('b', () => callbacks.push('b')), false);
  await new Promise((resolve) => sink.once('drain', resolve));
  assert.deepEqual(received, ['a', 'b']);
  assert.deepEqual(callbacks, ['a', 'b']);
  assert.equal(drains, 1);
  const done = streamPromises().finished(sink, { cleanup: true });
  sink.end('c', () => callbacks.push('end'));
  await done;
  assert.deepEqual(received, ['a', 'b', 'c']);
  assert.deepEqual(callbacks, ['a', 'b', 'end']);
});

testCase('callback finished observes final completion and returns listener cleanup', async () => {
  const { sink } = collecting();
  const eventNames = ['end', 'finish', 'error', 'close'];
  const before = eventNames.map((name) => sink.listenerCount(name));
  let calls = 0;
  let cleanup;
  const done = new Promise((resolve, reject) => {
    cleanup = finished(sink, (error) => {
      calls++;
      if (error) return reject(error);
      try { assert.equal(sink.writableFinished, true); resolve(); }
      catch (failure) { reject(failure); }
    });
  });
  assert.equal(typeof cleanup, 'function');
  sink.end('data');
  await done;
  cleanup(); cleanup();
  assert.equal(calls, 1);
  assert.deepEqual(eventNames.map((name) => sink.listenerCount(name)), before);
});

testCase('finished promise cleanup removes its event listeners after completion', async () => {
  const { sink } = collecting();
  const eventNames = ['end', 'finish', 'error', 'close'];
  const before = eventNames.map((name) => sink.listenerCount(name));
  const done = streamPromises().finished(sink, { cleanup: true });
  sink.end('data');
  await done;
  assert.deepEqual(eventNames.map((name) => sink.listenerCount(name)), before);
});

testCase('finished watches readable completion and handles already completed streams asynchronously', async () => {
  const source = Readable.from(['data']);
  const done = streamPromises().finished(source, { cleanup: true });
  source.resume();
  await done;
  let synchronous = true;
  await new Promise((resolve, reject) => {
    finished(source, (error) => {
      try { assert.ifError(error); assert.equal(synchronous, false); resolve(); }
      catch (failure) { reject(failure); }
    });
    synchronous = false;
  });
});

testCase('finished cleanup can stop watching without invoking its callback', async () => {
  const { sink } = collecting();
  let called = false;
  const cleanup = finished(sink, () => { called = true; });
  cleanup();
  const done = streamPromises().finished(sink, { cleanup: true });
  sink.end('data');
  await done;
  assert.equal(called, false);
});

testCase('pipeline preserves write errors, rejects once, and destroys its streams', async () => {
  const failure = new Error('sink failed');
  const source = Readable.from(['a', 'b']);
  const sink = new Writable({ write(chunk, encoding, callback) { later(() => callback(failure)); } });
  let callbacks = 0;
  await new Promise((resolve, reject) => {
    pipeline(source, sink, (error) => {
      callbacks++;
      try {
        assert.equal(error, failure);
        assert.equal(source.destroyed, true);
        assert.equal(sink.destroyed, true);
        resolve();
      } catch (unexpected) { reject(unexpected); }
    });
  });
  assert.equal(callbacks, 1);
});

testCase('promise pipeline propagates source and final errors without reporting success', async () => {
  const failure = new Error('read failed');
  const source = new Readable({ read() { this.destroy(failure); } });
  const { sink } = collecting();
  await assert.rejects(streamPromises().pipeline(source, sink), (error) => error === failure);
  assert.equal(sink.destroyed, true);
  const finalFailure = new Error('final failed');
  const finalSink = new Writable({
    write(chunk, encoding, callback) { callback(); },
    final(callback) { later(() => callback(finalFailure)); },
  });
  await assert.rejects(streamPromises().pipeline(Readable.from(['a']), finalSink), (error) => error === finalFailure);
});

testCase('pipeline propagates async iterator and transform failures', async () => {
  const failure = new Error('iterator failed');
  async function* values() { yield 'one'; throw failure; }
  const { sink } = collecting();
  await assert.rejects(streamPromises().pipeline(Readable.from(values()), sink), (error) => error === failure);
  const transformFailure = new Error('transform failed');
  const transform = new Transform({ transform(chunk, encoding, callback) { callback(transformFailure); } });
  const { sink: destination } = collecting();
  await assert.rejects(streamPromises().pipeline(Readable.from(['a']), transform, destination), (error) => error === transformFailure);
});

testCase('finished and pipeline reject premature close instead of reporting success', async () => {
  const { sink } = collecting();
  const done = streamPromises().finished(sink, { cleanup: true });
  sink.destroy();
  await assert.rejects(done, { code: 'ERR_STREAM_PREMATURE_CLOSE' });
  const abandoned = new Writable({ write(chunk, encoding, callback) { this.destroy(); } });
  await assert.rejects(streamPromises().pipeline(Readable.from(['a']), abandoned), { code: 'ERR_STREAM_PREMATURE_CLOSE' });
});
