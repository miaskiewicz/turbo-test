// node:stream Writable options and state Node v26.11.0 has: decodeStrings: false, the default
// encoding (option and setDefaultEncoding), encoding validation, _construct (writes, end and
// destroy wait for it; Readable's _read too) and `writable` turning false at end/destroy/error.
// RECORD=1 node writable-options.cjs prints the table this file asserts.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { Writable, Readable, Duplex } = require('node:stream');

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));
const mark = (log, label) => {
  process.nextTick(() => log.push(label + ':tick'));
  Promise.resolve().then(() => log.push(label + ':micro'));
};
const outcome = (error) => (error ? error.code || error.message : 'ok');
const show = (chunk) => (typeof chunk === 'string' ? 'string ' + JSON.stringify(chunk) : 'buffer ' + Buffer.from(chunk).toString('hex'));
const attempt = (log, label, fn) => { try { log.push(label + ' -> ' + fn()); } catch (error) { log.push(label + ' throws ' + outcome(error)); } };
function recorder(log, options = {}) {
  const stream = new Writable({
    ...options,
    write(chunk, encoding, callback) { log.push('_write ' + show(chunk) + ' ' + encoding); callback(); },
  });
  for (const event of ['prefinish', 'finish', 'close']) stream.on(event, () => log.push('event ' + event));
  stream.on('error', (error) => log.push('event error ' + outcome(error)));
  return stream;
}
const inTimer = (fn) => new Promise((resolve) => setTimeout(() => { fn(); resolve(); }));

const scenarios = {
  async 'decodeStrings: false passes strings and their encoding through'(log) {
    const held = [];
    const stream = new Writable({
      decodeStrings: false,
      write(chunk, encoding, callback) { log.push('_write ' + show(chunk) + ' ' + encoding); held.push(callback); },
    });
    await inTimer(() => {
      stream.write('héllo'); stream.write('ab', 'hex'); stream.write(Buffer.from('z'));
      log.push('length ' + stream.writableLength);
      while (held.length) held.shift()();
    });
  },
  async 'the default encoding decodes strings and setDefaultEncoding validates'(log) {
    await inTimer(() => {
      const stream = recorder(log, { defaultEncoding: 'base64' });
      stream.write('aGk=');
      attempt(log, 'setDefaultEncoding HEX', () => stream.setDefaultEncoding('HEX') === stream);
      stream.write('6869');
      attempt(log, 'setDefaultEncoding nope', () => stream.setDefaultEncoding('nope'));
      attempt(log, 'write unknown encoding', () => stream.write('x', 'nope'));
      attempt(log, 'write string as buffer', () => stream.write('x', 'buffer'));
      log.push('writableLength ' + stream.writableLength);
    });
  },
  async '_construct runs on the next tick and holds writes and end until it calls back'(log) {
    let release;
    let stream;
    await inTimer(() => {
      stream = recorder(log, { construct(callback) { log.push('_construct'); release = callback; } });
      log.push('constructed');
      mark(log, 'm');
    });
    await inTimer(() => {
      log.push('write -> ' + stream.write('a', () => log.push('cb a')));
      stream.end('b', () => log.push('end cb'));
      log.push('length ' + stream.writableLength);
    });
    await inTimer(() => { log.push('release'); release(); log.push('released'); mark(log, 'n'); });
    await settle();
  },
  async 'a _construct error destroys the stream and fails pending writes'(log) {
    const stream = recorder(log, { construct(callback) { setTimeout(() => callback(new Error('nope')), 5); } });
    stream.write('a', (error) => log.push('cb a ' + outcome(error)));
    await settle();
  },
  async 'destroy during _construct waits for it before _destroy'(log) {
    let release;
    const stream = recorder(log, {
      construct(callback) { log.push('_construct'); release = callback; },
      destroy(error, callback) { log.push('_destroy'); callback(error); },
    });
    await inTimer(() => { stream.destroy(); log.push('destroyed ' + stream.destroyed); });
    await inTimer(() => { log.push('release'); release(); });
    await settle();
  },
  async 'Readable _read waits for _construct'(log) {
    let release;
    const stream = new Readable({
      construct(callback) { log.push('_construct'); release = callback; },
      read() { log.push('_read'); this.push('x'); this.push(null); },
    });
    stream.on('data', (chunk) => log.push('data ' + chunk));
    stream.on('end', () => log.push('end'));
    await settle();
    log.push('release'); release();
    await settle();
  },
  async 'writable turns false at end, destroy and error'(log) {
    const ended = recorder(log);
    log.push('fresh ' + ended.writable);
    ended.end(); log.push('after end ' + ended.writable);
    const destroyed = recorder(log); destroyed.destroy(); log.push('after destroy ' + destroyed.writable);
    const failing = new Writable({ write(chunk, encoding, callback) { callback(new Error('boom')); } });
    failing.on('error', () => {});
    failing.write('x'); log.push('after error ' + failing.writable);
    const off = new Duplex({ writable: false, read() {} }); log.push('duplex writable:false ' + off.writable);
    await settle();
  },
};

const expected = {
  "decodeStrings: false passes strings and their encoding through": [
    "_write string \"héllo\" utf8",
    "length 8",
    "_write string \"ab\" hex",
    "_write buffer 7a buffer"
  ],
  "the default encoding decodes strings and setDefaultEncoding validates": [
    "_write buffer 6869 buffer",
    "setDefaultEncoding HEX -> true",
    "_write buffer 6869 buffer",
    "setDefaultEncoding nope throws ERR_UNKNOWN_ENCODING",
    "write unknown encoding throws ERR_UNKNOWN_ENCODING",
    "write string as buffer throws ERR_UNKNOWN_ENCODING",
    "writableLength 0"
  ],
  "_construct runs on the next tick and holds writes and end until it calls back": [
    "constructed",
    "_construct",
    "m:tick",
    "m:micro",
    "write -> true",
    "length 2",
    "release",
    "released",
    "_write buffer 61 buffer",
    "_write buffer 62 buffer",
    "event prefinish",
    "n:tick",
    "cb a",
    "end cb",
    "event finish",
    "event close",
    "n:micro"
  ],
  "a _construct error destroys the stream and fails pending writes": [
    "cb a nope",
    "event error nope",
    "event close"
  ],
  "destroy during _construct waits for it before _destroy": [
    "_construct",
    "destroyed true",
    "release",
    "_destroy",
    "event close"
  ],
  "Readable _read waits for _construct": [
    "_construct",
    "release",
    "_read",
    "data x",
    "end"
  ],
  "writable turns false at end, destroy and error": [
    "fresh true",
    "event prefinish",
    "after end false",
    "after destroy false",
    "after error false",
    "duplex writable:false false",
    "event finish",
    "event close",
    "event close"
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
