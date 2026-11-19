// A throwing subscriber or store transform never breaks the publisher (turbo-test#407): Node
// v26.11.0 re-raises it as an uncaught exception on the next tick and skips that store.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const dc = require('node:diagnostics_channel');
const { AsyncLocalStorage } = require('node:async_hooks');

testCase('a throwing subscriber is isolated from the traced call', async () => {
  const tc = dc.tracingChannel('isolation-subscriber');
  const seen = [];
  tc.subscribe({ start: () => { throw new Error('subscriber-boom'); }, end: (message) => seen.push(message.result) });
  assert.equal(tc.traceSync(() => 'ok'), 'ok');
  assert.deepEqual(seen, ['ok']);
  await new Promise((resolve) => setTimeout(resolve, 5));
});

testCase('a throwing store transform skips only that store', async () => {
  const broken = new AsyncLocalStorage();
  const kept = new AsyncLocalStorage();
  const channel = dc.channel('isolation-transform');
  channel.bindStore(broken, () => { throw new Error('transform-boom'); });
  channel.bindStore(kept, (data) => data * 2);
  assert.deepEqual(channel.runStores(4, () => [broken.getStore(), kept.getStore()]), [undefined, 8]);
  await new Promise((resolve) => setTimeout(resolve, 5));
});
