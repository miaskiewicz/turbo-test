// node:stream Readable state machine orderings, as Node v26.11.0 runs them. Scenarios cover
// push/read in flowing and paused, read(n) byte accounting (including read(0)) and
// howMuchToRead, 'readable' first-HWM-cross and end semantics, needReadable/emittedReadable
// latching, _read throttling via state.reading and maybeReadMore, pause/resume, backpressure
// via pipe() and awaitDrainWriters, and destroy/end/close ordering. Each scenario runs from a
// timer callback (a macrotask, as Node's I/O would) and logs every _read call, read() return,
// push() return, event, plus nextTick / microtask markers. RECORD=1 node readable-port.cjs
// prints the table this file asserts.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { Readable, Writable } = require('node:stream');

const settle = () => new Promise((resolve) => setTimeout(resolve, 10));
const mark = (log, label) => {
  process.nextTick(() => log.push(label + ':tick'));
  Promise.resolve().then(() => log.push(label + ':micro'));
};
const inTimer = (fn) => new Promise((resolve) => setTimeout(() => { fn(); resolve(); }));
const outcome = (error) => (error ? error.code || error.message : 'ok');

// A readable whose _read is inert by default: it logs the requested size but does not push.
// Scenarios push directly from the test body, as a Node I/O source would.
function readable(log, options = {}) {
  const reads = [];
  const feeder = options.feeder;
  const stream = new Readable({
    highWaterMark: options.highWaterMark,
    objectMode: options.objectMode,
    encoding: options.encoding,
    read(n) {
      log.push('_read ' + n);
      reads.push(n);
      if (feeder) {
        const chunk = feeder(reads.length - 1);
        if (chunk !== undefined) this.push(chunk);
      }
    },
  });
  if (options.events !== false) {
    for (const event of ['end', 'close']) stream.on(event, () => log.push('event ' + event));
    stream.on('error', (error) => log.push('event error ' + outcome(error)));
  }
  return { stream, reads };
}

const scenarios = {
  async 'push buffers in paused mode; read() dequeues it; _read is called when empty'(log) {
    const { stream } = readable(log, { highWaterMark: 4 });
    await inTimer(() => {
      log.push('push ab -> ' + stream.push(Buffer.from('ab')));
      log.push('length ' + stream.readableLength);
      log.push('read -> ' + String(stream.read()));
      log.push('length ' + stream.readableLength);
      log.push('read -> ' + String(stream.read()));
      mark(log, 'm');
    });
  },
  async "'readable' fires on first HWM-cross; stays latched until the queue drains"(log) {
    const { stream } = readable(log, { highWaterMark: 4 });
    stream.on('readable', () => log.push('event readable length=' + stream.readableLength));
    await inTimer(() => {
      log.push('push ab -> ' + stream.push(Buffer.from('ab')));
      log.push('push cd -> ' + stream.push(Buffer.from('cd')));
      log.push('push e -> ' + stream.push(Buffer.from('e')));
      mark(log, 'm');
    });
    await inTimer(() => {
      log.push('read -> ' + String(stream.read()));
      log.push('length ' + stream.readableLength);
    });
  },
  async 'read(n) returns exactly n bytes; read(0) is a null read for signalling'(log) {
    const { stream } = readable(log, { highWaterMark: 16 });
    stream.on('readable', () => log.push('event readable'));
    await inTimer(() => {
      stream.push(Buffer.from('abcdef'));
      log.push('read(3) -> ' + String(stream.read(3)));
      log.push('length ' + stream.readableLength);
      log.push('read(2) -> ' + String(stream.read(2)));
      log.push('length ' + stream.readableLength);
      log.push('read(0) -> ' + String(stream.read(0)));
      log.push('length ' + stream.readableLength);
    });
  },
  async 'read(n) returns null when n > buffered and not ended; remainder at end'(log) {
    const { stream } = readable(log, { highWaterMark: 16 });
    stream.on('readable', () => log.push('event readable'));
    await inTimer(() => {
      stream.push(Buffer.from('abc'));
      log.push('read(10) -> ' + String(stream.read(10)));
      log.push('length ' + stream.readableLength);
      stream.push(null);
    });
    await inTimer(() => {
      log.push('read(10) -> ' + String(stream.read(10)));
      log.push('ended ' + stream.readableEnded);
      mark(log, 'm');
    });
  },
  async 'read(n) across chunks concatenates up to n bytes'(log) {
    const { stream } = readable(log, { highWaterMark: 16 });
    await inTimer(() => {
      stream.push(Buffer.from('ab'));
      stream.push(Buffer.from('cd'));
      stream.push(Buffer.from('ef'));
      log.push('length ' + stream.readableLength);
      log.push('read(5) -> ' + String(stream.read(5)));
      log.push('length ' + stream.readableLength);
      log.push('read() -> ' + String(stream.read()));
    });
  },
  async 'flowing mode emits data for every chunk; _read refills until feeder ends'(log) {
    let left = 3;
    const { stream } = readable(log, { highWaterMark: 1, feeder: () => (left-- > 0 ? Buffer.from(String(left + 1)) : null) });
    stream.on('data', (chunk) => log.push('data ' + chunk.toString()));
    await settle();
  },
  async 'pause() stops data; resume() restarts flowing'(log) {
    let left = 4;
    const { stream } = readable(log, { highWaterMark: 1, feeder: () => (left-- > 0 ? Buffer.from(String(left + 1)) : null) });
    let paused = false;
    stream.on('data', (chunk) => {
      log.push('data ' + chunk.toString());
      if (!paused && chunk.toString() === '4') {
        paused = true;
        stream.pause();
        setTimeout(() => { log.push('resume'); stream.resume(); }, 5);
      }
    });
    await settle();
    await settle();
  },
  async "'readable' first-HWM cross is latched; later pushes below hwm do not re-emit"(log) {
    const { stream } = readable(log, { highWaterMark: 4 });
    const events = [];
    stream.on('readable', () => events.push(stream.readableLength));
    await inTimer(() => { stream.push(Buffer.from('abcd')); });
    await inTimer(() => { stream.push(Buffer.from('e')); log.push('events ' + JSON.stringify(events)); });
  },
  async "'readable' re-fires after the queue drains and more is pushed"(log) {
    const { stream } = readable(log, { highWaterMark: 4 });
    const events = [];
    stream.on('readable', () => events.push('r len=' + stream.readableLength));
    await inTimer(() => { stream.push(Buffer.from('ab')); });
    await inTimer(() => {
      log.push('read -> ' + String(stream.read()));
      log.push('events after drain ' + JSON.stringify(events));
    });
    await inTimer(() => { stream.push(Buffer.from('cd')); log.push('events after refill ' + JSON.stringify(events)); });
  },
  async '_read is not re-entered while a prior _read has not pushed or returned yet'(log) {
    let called = 0;
    const { stream } = readable(log, { highWaterMark: 4, events: false });
    stream._read = function() { called += 1; log.push('_read#' + called); this.push(Buffer.from('xy')); };
    await inTimer(() => {
      log.push('read -> ' + String(stream.read()));
      log.push('called ' + called);
    });
  },
  async 'push(null) emits end after the buffered chunks are read, then close'(log) {
    const { stream } = readable(log, { highWaterMark: 16 });
    await inTimer(() => {
      stream.push(Buffer.from('a'));
      stream.push(Buffer.from('b'));
      stream.push(null);
      log.push('read -> ' + String(stream.read()));
      log.push('ended? ' + stream.readableEnded);
    });
    await inTimer(() => {
      log.push('read -> ' + String(stream.read()));
      log.push('read -> ' + String(stream.read()));
    });
    await settle();
  },
  async 'pipe() forwards data and ends the destination'(log) {
    let left = 3;
    const { stream } = readable(log, { highWaterMark: 1, feeder: () => (left-- > 0 ? Buffer.from(String(left + 1)) : null), events: false });
    const dest = new Writable({ highWaterMark: 1, write(chunk, enc, cb) { log.push('write ' + chunk.toString()); setTimeout(cb, 1); } });
    dest.on('finish', () => log.push('event finish'));
    stream.pipe(dest);
    await settle(); await settle(); await settle();
  },
  async 'destroy(error) emits error then close; the buffered chunk is discarded'(log) {
    const { stream } = readable(log, { highWaterMark: 16 });
    await inTimer(() => {
      stream.push(Buffer.from('a'));
      stream.destroy(new Error('boom'));
      log.push('destroyed ' + stream.destroyed);
      mark(log, 'm');
    });
    await settle();
  },
  async 'setEncoding returns strings; read(n) measures bytes before decoding'(log) {
    const { stream } = readable(log, { highWaterMark: 16, encoding: 'utf8' });
    await inTimer(() => {
      stream.push(Buffer.from('abcdef'));
      log.push('read(3) -> ' + JSON.stringify(stream.read(3)));
      log.push('read() -> ' + JSON.stringify(stream.read()));
      stream.push(null);
    });
    await settle();
  },
  async 'objectMode: read(n) ignores n and returns one item at a time; read of empty is null'(log) {
    const { stream } = readable(log, { objectMode: true, highWaterMark: 4 });
    await inTimer(() => {
      stream.push({ a: 1 });
      stream.push({ b: 2 });
      log.push('length ' + stream.readableLength);
      log.push('read(10) -> ' + JSON.stringify(stream.read(10)));
      log.push('read(10) -> ' + JSON.stringify(stream.read(10)));
      log.push('read(10) -> ' + String(stream.read(10)));
    });
  },
};

const expected = {
  "push buffers in paused mode; read() dequeues it; _read is called when empty": [
    "push ab -> true",
    "length 2",
    "_read 4",
    "read -> ab",
    "length 0",
    "read -> null",
    "m:tick",
    "m:micro"
  ],
  "'readable' fires on first HWM-cross; stays latched until the queue drains": [
    "_read 4",
    "push ab -> true",
    "push cd -> false",
    "push e -> false",
    "event readable length=5",
    "m:tick",
    "m:micro",
    "_read 4",
    "read -> ab",
    "length 3"
  ],
  "read(n) returns exactly n bytes; read(0) is a null read for signalling": [
    "_read 16",
    "_read 16",
    "read(3) -> abc",
    "length 3",
    "read(2) -> de",
    "length 1",
    "read(0) -> null",
    "length 1",
    "event readable"
  ],
  "read(n) returns null when n > buffered and not ended; remainder at end": [
    "_read 16",
    "_read 16",
    "read(10) -> null",
    "length 3",
    "event readable",
    "event readable",
    "read(10) -> abc",
    "ended false",
    "event end",
    "m:tick",
    "event close",
    "m:micro"
  ],
  "read(n) across chunks concatenates up to n bytes": [
    "length 6",
    "_read 16",
    "read(5) -> abcde",
    "length 1",
    "read() -> f"
  ],
  "flowing mode emits data for every chunk; _read refills until feeder ends": [
    "_read 1",
    "_read 1",
    "data 3",
    "_read 1",
    "data 2",
    "_read 1",
    "data 1",
    "event end",
    "event close"
  ],
  "pause() stops data; resume() restarts flowing": [
    "_read 1",
    "_read 1",
    "data 4",
    "resume",
    "_read 1",
    "data 3",
    "_read 1",
    "data 2",
    "_read 1",
    "data 1",
    "event end",
    "event close"
  ],
  "'readable' first-HWM cross is latched; later pushes below hwm do not re-emit": [
    "_read 4",
    "events [4]"
  ],
  "'readable' re-fires after the queue drains and more is pushed": [
    "_read 4",
    "_read 4",
    "read -> ab",
    "events after drain [\"r len=2\"]",
    "events after refill [\"r len=2\"]",
    "_read 4"
  ],
  "_read is not re-entered while a prior _read has not pushed or returned yet": [
    "_read#1",
    "read -> xy",
    "called 1",
    "_read#2",
    "_read#3"
  ],
  "push(null) emits end after the buffered chunks are read, then close": [
    "read -> a",
    "ended? false",
    "read -> b",
    "read -> null",
    "event end",
    "event close"
  ],
  "pipe() forwards data and ends the destination": [
    "_read 1",
    "_read 1",
    "write 3",
    "_read 1",
    "write 2",
    "_read 1",
    "write 1",
    "event finish"
  ],
  "destroy(error) emits error then close; the buffered chunk is discarded": [
    "destroyed true",
    "event error boom",
    "event close",
    "m:tick",
    "m:micro"
  ],
  "setEncoding returns strings; read(n) measures bytes before decoding": [
    "_read 16",
    "read(3) -> \"abc\"",
    "read() -> \"def\""
  ],
  "objectMode: read(n) ignores n and returns one item at a time; read of empty is null": [
    "length 2",
    "_read 16",
    "read(10) -> {\"a\":1}",
    "read(10) -> {\"b\":2}",
    "read(10) -> null"
  ]
};

if (process.env.RECORD || expected === undefined) {
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
