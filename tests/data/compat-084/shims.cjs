const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const dc = require('node:diagnostics_channel');
testCase('EventEmitter propagates unhandled errors and once emission results', () => {
  const emitter = new EventEmitter(); const error = new Error('expected');
  assert.throws(() => emitter.emit('error', error), (e) => e === error);
  assert.throws(() => emitter.emit('error', 42), (e) => e.code === 'ERR_UNHANDLED_ERROR' && e.context === 42);
  let seen = 0;
  function listener(value) { assert.equal(this, emitter); seen += value; }
  emitter.once('__proto__', listener);
  assert.equal(emitter.listeners('__proto__')[0], listener);
  assert.equal(emitter.emit('__proto__', 2), true);
  assert.equal(emitter.emit('__proto__', 2), false);
  assert.equal(seen, 2);
  emitter.once('removed', listener); emitter.off('removed', listener);
  assert.equal(emitter.emit('removed', 5), false);
  emitter.on('error', (e) => assert.equal(e, error));
  assert.equal(emitter.emit('error', error), true);
  emitter.removeAllListeners(); emitter.once('__proto__', listener);
  assert.equal(emitter.emit('__proto__', 3), true);
  assert.equal(seen, 5);
});
testCase('diagnostics channels retain identity and unsubscribe deliveries', () => {
  const name = Symbol('compat-084'); const channel = dc.channel(name); const seen = [];
  assert.equal(dc.channel(name), channel);
  assert.equal(channel.hasSubscribers, false);
  const listener = (value, actualName) => seen.push([value, actualName]);
  dc.subscribe(name, listener);
  assert.equal(dc.hasSubscribers(name), true);
  channel.publish('payload');
  assert.deepEqual(seen, [['payload', name]]);
  assert.equal(channel.unsubscribe(listener), true);
  assert.equal(dc.unsubscribe(name, listener), false);
  channel.publish('ignored');
  assert.equal(seen.length, 1);
  assert.equal(channel.hasSubscribers, false);
  assert.throws(() => channel.subscribe(null), TypeError);
});
testCase('FormData converts values and preserves insertion order', () => {
  const data = new FormData();
  data.append(1, 42); data.append('last', true); data.append(1, null);
  assert.equal(data.get(1), '42');
  assert.deepEqual(data.getAll('1'), ['42', 'null']);
  assert.deepEqual(Array.from(data), [['1', '42'], ['last', 'true'], ['1', 'null']]);
  assert.deepEqual(Array.from(data.keys()), ['1', 'last', '1']);
  assert.deepEqual(Array.from(data.values()), ['42', 'true', 'null']);
  const receiver = {}; const seen = [];
  data.forEach(function(v, k, form) { assert.equal(this, receiver); assert.equal(form, data); seen.push([k, v]); }, receiver);
  assert.deepEqual(seen, Array.from(data.entries()));
  data.set(1, 7); assert.deepEqual(Array.from(data), [['1', '7'], ['last', 'true']]);
  data.set('new', undefined); assert.equal(data.get('new'), 'undefined');
  data.delete(1); assert.equal(data.has(1), false); assert.equal(data.get(1), null);
});
testCase('data URL fetch delivers text JSON and independent clones', async () => {
  const response = await fetch('data:text/plain,hello%20world');
  assert.equal(response.status, 200); assert.equal(response.ok, true);
  assert.equal(response.headers.get('content-type'), 'text/plain');
  assert.equal(response.bodyUsed, false);
  const clone = response.clone();
  assert.equal(await response.text(), 'hello world');
  assert.equal(response.bodyUsed, true);
  await assert.rejects(response.text(), TypeError);
  assert.throws(() => response.clone(), TypeError);
  assert.equal(await clone.text(), 'hello world');
  const json = await fetch('data:application/json,%7B%22value%22%3A42%7D');
  assert.deepEqual(await json.json(), { value: 42 });
  const invalid = await fetch('data:application/json,nope');
  await assert.rejects(invalid.json(), SyntaxError);
});
testCase('data URL binary and escaped payloads retain exact bytes', async () => {
  const response = await fetch('data:application/octet-stream;base64,AP8B');
  assert.deepEqual(Array.from(new Uint8Array(await response.arrayBuffer())), [0, 255, 1]);
  const escaped = await fetch('data:,%FF%00%25');
  assert.deepEqual(Array.from(new Uint8Array(await escaped.arrayBuffer())), [255, 0, 37]);
  const fragment = await fetch('data:,a%23b#ignored');
  assert.equal(await fragment.text(), 'a#b');
  assert.equal(fragment.headers.get('content-type'), 'text/plain;charset=US-ASCII');
});
testCase('data URL stream locks and consumption follow Response behavior', async () => {
  const response = await fetch('data:,abc'); const reader = response.body.getReader();
  assert.equal(response.bodyUsed, false);
  assert.throws(() => response.clone(), TypeError);
  await assert.rejects(response.text(), TypeError);
  const first = await reader.read();
  assert.deepEqual(Array.from(first.value), [97, 98, 99]);
  assert.equal(response.bodyUsed, true);
  assert.equal((await reader.read()).done, true);
  const cancelled = await fetch('data:,abc');
  await cancelled.body.cancel();
  assert.equal(cancelled.bodyUsed, true);
  await assert.rejects(cancelled.text(), TypeError);
});
testCase('data URL fetch rejects malformed input and honors aborted signals', async () => {
  for (const url of ['data:missing-comma', 'data:;base64,%%%', 'data:;base64,a=', 'data:;base64,ab=']) {
    await assert.rejects(fetch(url), TypeError);
  }
  const controller = new AbortController(); const reason = new Error('stop'); controller.abort(reason);
  await assert.rejects(fetch('data:,abc', { signal: controller.signal }), (e) => e === reason);
});
