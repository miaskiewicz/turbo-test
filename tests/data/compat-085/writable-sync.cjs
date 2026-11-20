// node:stream Writable write/cork/writev/drain/error orderings, as Node v26.11.0 runs them.
// Each scenario runs from a timer callback (a macrotask, as Node's I/O would) and logs every
// _write/_writev call, write() return, callback and event, plus nextTick / microtask markers.
// RECORD=1 node writable-sync.cjs prints the table this file asserts.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { Writable, Transform } = require('node:stream');

const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
const mark = (log, label) => {
  process.nextTick(() => log.push(label + ':tick'));
  Promise.resolve().then(() => log.push(label + ':micro'));
};
const outcome = (error) => (error ? error.code || error.message : 'ok');

function writable(log, options = {}) {
  const held = [];
  const stream = new Writable({
    highWaterMark: options.highWaterMark,
    objectMode: options.objectMode,
    write(chunk, encoding, callback) {
      log.push('_write ' + (options.objectMode ? JSON.stringify(chunk) : chunk.toString()) + ' ' + encoding);
      if (options.hold) held.push(callback);
      else callback(options.fail ? new Error('boom') : undefined);
    },
    writev: options.writev ? function(chunks, callback) {
      log.push('_writev ' + chunks.map((c) => c.chunk.toString()).join(','));
      if (options.hold) held.push(callback); else callback();
    } : undefined,
    final(callback) { log.push('_final'); callback(); },
  });
  for (const event of ['drain', 'prefinish', 'finish', 'close']) stream.on(event, () => log.push('event ' + event));
  stream.on('error', (error) => log.push('event error ' + outcome(error)));
  return { stream, held };
}
const write = (log, stream, chunk) =>
  log.push('write ' + chunk + ' -> ' + stream.write(chunk, (error) => log.push('cb ' + chunk + ' ' + outcome(error))));
const inTimer = (fn) => new Promise((resolve) => setTimeout(() => { fn(); resolve(); }));

const scenarios = {
  async 'sync _write runs inside write() and callbacks follow on the next tick'(log) {
    const { stream } = writable(log);
    await inTimer(() => {
      write(log, stream, 'a'); write(log, stream, 'b');
      log.push('length ' + stream.writableLength);
      mark(log, 'm'); log.push('sync end');
    });
  },
  async 'a pending _write buffers later chunks until its callback'(log) {
    const { stream, held } = writable(log, { hold: true });
    await inTimer(() => {
      write(log, stream, 'a'); write(log, stream, 'b'); write(log, stream, 'c');
      log.push('length ' + stream.writableLength);
    });
    await inTimer(() => { log.push('release a'); held.shift()(); log.push('after release a'); mark(log, 'm'); });
    await inTimer(() => { log.push('release b'); held.shift()(); log.push('release c'); held.shift()(); log.push('length ' + stream.writableLength); });
  },
  async 'buffered chunks go to _writev in one batch'(log) {
    const { stream, held } = writable(log, { hold: true, writev: true });
    await inTimer(() => { write(log, stream, 'a'); write(log, stream, 'b'); write(log, stream, 'c'); });
    await inTimer(() => { log.push('release a'); held.shift()(); log.push('after release a'); });
    await inTimer(() => { log.push('release bc'); held.shift()(); mark(log, 'm'); });
  },
  async 'cork buffers until the matching uncork, then _writev flushes'(log) {
    const { stream } = writable(log, { writev: true });
    await inTimer(() => {
      stream.cork(); stream.cork();
      write(log, stream, 'a'); write(log, stream, 'b');
      log.push('corked ' + stream.writableCorked);
      stream.uncork(); log.push('after uncork 1, corked ' + stream.writableCorked);
      stream.uncork(); log.push('after uncork 2, corked ' + stream.writableCorked);
      stream.uncork(); log.push('extra uncork, corked ' + stream.writableCorked);
      mark(log, 'm');
    });
  },
  async 'uncork without _writev writes each chunk with _write'(log) {
    const { stream } = writable(log);
    await inTimer(() => {
      stream.cork(); write(log, stream, 'a'); write(log, stream, 'b');
      stream.uncork(); log.push('uncorked'); mark(log, 'm');
    });
  },
  async 'end() uncorks and runs final, prefinish, finish, the end callback and close'(log) {
    const { stream } = writable(log);
    await inTimer(() => {
      stream.cork(); write(log, stream, 'a');
      stream.end('b', () => log.push('end cb'));
      log.push('ended, corked ' + stream.writableCorked); mark(log, 'm');
    });
    await settle();
  },
  async 'drain follows a full buffer once a sync write completes'(log) {
    const { stream } = writable(log, { highWaterMark: 2 });
    await inTimer(() => {
      write(log, stream, 'abc'); log.push('needDrain ' + stream.writableNeedDrain); mark(log, 'm');
    });
  },
  async 'drain is emitted inside an async write callback, before the user callback'(log) {
    const { stream, held } = writable(log, { highWaterMark: 2, hold: true });
    await inTimer(() => { write(log, stream, 'ab'); write(log, stream, 'c'); });
    await inTimer(() => { log.push('release ab'); held.shift()(); log.push('release c'); held.shift()(); log.push('released'); });
  },
  async 'a sync _write error reaches the callback and error event on later ticks'(log) {
    const { stream } = writable(log, { fail: true });
    await inTimer(() => { write(log, stream, 'a'); write(log, stream, 'b'); log.push('destroyed ' + stream.destroyed); mark(log, 'm'); });
    await settle();
  },
  async 'an async _write error fails the buffered writes too'(log) {
    const { stream, held } = writable(log, { hold: true });
    await inTimer(() => { write(log, stream, 'a'); write(log, stream, 'b'); });
    await inTimer(() => { log.push('fail a'); held.shift()(new Error('boom')); log.push('after fail'); mark(log, 'm'); });
    await settle();
  },
  async 'write after end errors on the next tick'(log) {
    const { stream } = writable(log);
    await inTimer(() => { stream.end(); write(log, stream, 'x'); mark(log, 'm'); });
    await settle();
  },
  async 'write after destroy calls back with ERR_STREAM_DESTROYED'(log) {
    const { stream } = writable(log);
    await inTimer(() => { stream.destroy(); write(log, stream, 'x'); mark(log, 'm'); });
    await settle();
  },
  async 'calling a write callback twice is ERR_MULTIPLE_CALLBACK'(log) {
    const stream = new Writable({ write(chunk, encoding, callback) { callback(); callback(); } });
    stream.on('error', (error) => log.push('event error ' + outcome(error)));
    await inTimer(() => { log.push('write -> ' + stream.write('a')); mark(log, 'm'); });
    await settle();
  },
  async 'object mode writes keep the value and its encoding'(log) {
    const { stream } = writable(log, { objectMode: true });
    await inTimer(() => { log.push('write -> ' + stream.write({ n: 1 })); log.push('write -> ' + stream.write('s', 'latin1')); });
  },
  async 'destroy fails buffered writes; the in-flight write waits for its own callback'(log) {
    const { stream, held } = writable(log, { hold: true });
    await inTimer(() => { write(log, stream, 'a'); write(log, stream, 'b'); stream.destroy(); log.push('destroyed ' + stream.destroyed); mark(log, 'm'); });
    await settle();
    await inTimer(() => { log.push('release a'); held.shift()(); log.push('released'); mark(log, 'n'); });
    await settle();
  },
  async 'destroy(error) calls _destroy synchronously, then error and close on the next tick'(log) {
    const { Readable } = require('node:stream');
    const streams = { writable: new Writable({ write() {} }), readable: new Readable({ read() {} }) };
    for (const [name, stream] of Object.entries(streams)) {
      stream._destroy = (error, callback) => { log.push(name + ' _destroy ' + outcome(error)); callback(error); };
      stream.on('error', (error) => log.push(name + ' error ' + outcome(error)));
      stream.on('close', () => log.push(name + ' close'));
    }
    await inTimer(() => {
      streams.writable.destroy(new Error('w-boom')); streams.readable.destroy(new Error('r-boom'));
      log.push('errored ' + outcome(streams.writable.errored) + ' ' + outcome(streams.readable.errored));
      mark(log, 'm');
    });
    await settle();
  },
  async 'Transform write calls _transform synchronously'(log) {
    const stream = new Transform({ transform(chunk, encoding, callback) { log.push('_transform ' + chunk); callback(null, chunk); } });
    stream.resume();
    await inTimer(() => { log.push('write -> ' + stream.write('a')); log.push('write -> ' + stream.write('b')); log.push('sync end'); });
  },
};

const expected = {
  "sync _write runs inside write() and callbacks follow on the next tick": [
    "_write a buffer",
    "write a -> true",
    "_write b buffer",
    "write b -> true",
    "length 0",
    "sync end",
    "cb a ok",
    "cb b ok",
    "m:tick",
    "m:micro"
  ],
  "a pending _write buffers later chunks until its callback": [
    "_write a buffer",
    "write a -> true",
    "write b -> true",
    "write c -> true",
    "length 3",
    "release a",
    "_write b buffer",
    "cb a ok",
    "after release a",
    "m:tick",
    "m:micro",
    "release b",
    "_write c buffer",
    "cb b ok",
    "release c",
    "cb c ok",
    "length 0"
  ],
  "buffered chunks go to _writev in one batch": [
    "_write a buffer",
    "write a -> true",
    "write b -> true",
    "write c -> true",
    "release a",
    "_writev b,c",
    "cb a ok",
    "after release a",
    "release bc",
    "cb b ok",
    "cb c ok",
    "m:tick",
    "m:micro"
  ],
  "cork buffers until the matching uncork, then _writev flushes": [
    "write a -> true",
    "write b -> true",
    "corked 2",
    "after uncork 1, corked 1",
    "_writev a,b",
    "after uncork 2, corked 0",
    "extra uncork, corked 0",
    "cb a ok",
    "cb b ok",
    "m:tick",
    "m:micro"
  ],
  "uncork without _writev writes each chunk with _write": [
    "write a -> true",
    "write b -> true",
    "_write a buffer",
    "_write b buffer",
    "uncorked",
    "cb a ok",
    "cb b ok",
    "m:tick",
    "m:micro"
  ],
  "end() uncorks and runs final, prefinish, finish, the end callback and close": [
    "write a -> true",
    "_write a buffer",
    "_write b buffer",
    "_final",
    "event prefinish",
    "ended, corked 0",
    "cb a ok",
    "end cb",
    "event finish",
    "m:tick",
    "event close",
    "m:micro"
  ],
  "drain follows a full buffer once a sync write completes": [
    "_write abc buffer",
    "write abc -> true",
    "needDrain false",
    "cb abc ok",
    "m:tick",
    "m:micro"
  ],
  "drain is emitted inside an async write callback, before the user callback": [
    "_write ab buffer",
    "write ab -> false",
    "write c -> false",
    "release ab",
    "_write c buffer",
    "cb ab ok",
    "release c",
    "event drain",
    "cb c ok",
    "released"
  ],
  "a sync _write error reaches the callback and error event on later ticks": [
    "_write a buffer",
    "write a -> false",
    "write b -> false",
    "destroyed false",
    "cb a boom",
    "cb b boom",
    "m:tick",
    "event error boom",
    "event close",
    "m:micro"
  ],
  "an async _write error fails the buffered writes too": [
    "_write a buffer",
    "write a -> true",
    "write b -> true",
    "fail a",
    "cb a boom",
    "cb b boom",
    "after fail",
    "event error boom",
    "event close",
    "m:tick",
    "m:micro"
  ],
  "write after end errors on the next tick": [
    "_final",
    "event prefinish",
    "write x -> false",
    "event finish",
    "cb x ERR_STREAM_WRITE_AFTER_END",
    "event error ERR_STREAM_WRITE_AFTER_END",
    "event close",
    "m:tick",
    "m:micro"
  ],
  "write after destroy calls back with ERR_STREAM_DESTROYED": [
    "write x -> false",
    "event close",
    "cb x ERR_STREAM_DESTROYED",
    "m:tick",
    "m:micro"
  ],
  "calling a write callback twice is ERR_MULTIPLE_CALLBACK": [
    "write -> false",
    "event error ERR_MULTIPLE_CALLBACK",
    "m:tick",
    "m:micro"
  ],
  "object mode writes keep the value and its encoding": [
    "_write {\"n\":1} undefined",
    "write -> true",
    "_write \"s\" latin1",
    "write -> true"
  ],
  "destroy fails buffered writes; the in-flight write waits for its own callback": [
    "_write a buffer",
    "write a -> true",
    "write b -> true",
    "destroyed true",
    "event close",
    "m:tick",
    "m:micro",
    "release a",
    "cb a ok",
    "cb b ERR_STREAM_DESTROYED",
    "released",
    "n:tick",
    "n:micro"
  ],
  "destroy(error) calls _destroy synchronously, then error and close on the next tick": [
    "writable _destroy w-boom",
    "readable _destroy r-boom",
    "errored w-boom r-boom",
    "writable error w-boom",
    "writable close",
    "readable error r-boom",
    "readable close",
    "m:tick",
    "m:micro"
  ],
  "Transform write calls _transform synchronously": [
    "_transform a",
    "write -> true",
    "_transform b",
    "write -> true",
    "sync end"
  ]
};

if (process.env.RECORD) {
  (async () => {
    const table = {};
    for (const [name, run] of Object.entries(scenarios)) { const log = []; await run(log); await settle(); table[name] = log; }
    console.log(JSON.stringify(table, null, 2));
  })();
} else {
  for (const [name, run] of Object.entries(scenarios)) {
    testCase(name, async () => {
      const log = [];
      await run(log);
      await settle();
      assert.deepEqual(log, expected[name]);
    });
  }
}
