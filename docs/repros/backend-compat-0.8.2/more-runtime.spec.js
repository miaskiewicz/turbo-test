test('classifies IPv4 and IPv6 without opening a socket', () => {
  const { isIP, isIPv4, isIPv6 } = require('node:net');
  expect([isIP('127.0.0.1'), isIPv4('127.0.0.1'), isIPv6('::1')]).toEqual([4, true, true]);
});
test('throws an unhandled EventEmitter error', () => {
  const { EventEmitter } = require('node:events');
  const error = new Error('test error');
  expect(() => new EventEmitter().emit('error', error)).toThrow(/test error/);
});
test('returns true when an EventEmitter once listener was called', () => {
  const { EventEmitter } = require('node:events');
  const emitter = new EventEmitter();
  emitter.once('event', () => {});
  expect(emitter.emit('event')).toBe(true);
});
test('passes emitted values to diagnostics_channel subscribers', () => {
  const { channel } = require('node:diagnostics_channel');
  const events = channel('test.compat.example');
  const received = [];
  const subscriber = value => received.push(value);
  events.subscribe(subscriber);
  try {
    events.publish({ value: 42 });
    expect(received).toEqual([{ value: 42 }]);
  } finally {
    events.unsubscribe(subscriber);
  }
});
test('preserves a Map in a V8 serialization round-trip', () => {
  const { serialize, deserialize } = require('node:v8');
  const bytes = serialize(new Map([['answer', 42]]));
  expect(Buffer.isBuffer(bytes)).toBe(true);
  expect(deserialize(bytes).get('answer')).toBe(42);
});
test('coerces FormData scalars and exposes entries', () => {
  const data = new FormData();
  data.append('count', 42);
  expect(data.get('count')).toBe('42');
  expect([...data.entries()]).toEqual([['count', '42']]);
});
test('writes bytes to a Writable before pipeline reports success', async () => {
  const { PassThrough, Writable, pipeline } = require('node:stream');
  const source = new PassThrough();
  const bytes = [];
  const sink = new Writable({
    write(chunk, _encoding, done) {
      bytes.push(chunk.toString());
      done();
    },
  });
  await new Promise((resolve, reject) => {
    pipeline(source, sink, error => error ? reject(error) : resolve());
    source.end('hello');
  });
  expect(bytes).toEqual(['hello']);
});
