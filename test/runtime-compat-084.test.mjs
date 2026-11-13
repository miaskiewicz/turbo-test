import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { EventEmitter } from 'node:events';
import * as diagnostics from 'node:diagnostics_channel';

const source = readFileSync(new URL('../src/runtime.js', import.meta.url), 'utf8');
function between(start, end) { return source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start))); }
const context = vm.createContext({ Blob, TextEncoder, TextDecoder, ReadableStream, URL, Uint8Array, Error, TypeError });
vm.runInContext(between('function __unhandledEventError', 'const __events =') + '\nglobalThis.Emitter = __EventEmitter;', context);
vm.runInContext(between('const __dcChannels', '__dc.default = __dc;') + '\nglobalThis.dc = __dc;', context);
vm.runInContext(between("if (typeof globalThis.FormData", "if (typeof globalThis.structuredClone"), context);

function emitterChecks(Emitter) {
  const emitter = new Emitter();
  const error = new Error('unhandled');
  assert.throws(() => emitter.emit('error', error), (e) => e === error);
  assert.throws(() => emitter.emit('error', 'oops'), (e) => e.code === 'ERR_UNHANDLED_ERROR' && e.context === 'oops');
  let calls = 0;
  emitter.once('__proto__', function(value) { assert.equal(this, emitter); calls += value; });
  assert.equal(emitter.emit('__proto__', 2), true);
  assert.equal(emitter.emit('__proto__', 2), false);
  assert.equal(calls, 2);
  emitter.on('error', (e) => assert.equal(e, error));
  assert.equal(emitter.emit('error', error), true);
}
test('EventEmitter controls and runtime preserve errors and once return values', () => {
  emitterChecks(EventEmitter); emitterChecks(context.Emitter);
});
function channelChecks(dc) {
  const name = Symbol('compat');
  const channel = dc.channel(name);
  assert.equal(channel, dc.channel(name));
  const seen = [];
  const fn = (value, key) => seen.push([value, key]);
  dc.subscribe(name, fn);
  assert.equal(channel.hasSubscribers, true);
  assert.equal(dc.hasSubscribers(name), true);
  channel.publish(42);
  assert.deepEqual(seen, [[42, name]]);
  assert.equal(dc.unsubscribe(name, fn), true);
  assert.equal(dc.unsubscribe(name, fn), false);
  channel.publish(43);
  assert.equal(seen.length, 1);
  assert.equal(channel.hasSubscribers, false);
}
test('diagnostics channel controls and runtime registry deliver and unsubscribe', () => {
  channelChecks(diagnostics); channelChecks(context.dc);
});
function formChecks(Form) {
  const form = new Form();
  form.append(1, 42); form.append(1, null); form.append('last', true);
  assert.equal(form.get('1'), '42');
  assert.deepEqual(Array.from(form, (pair) => Array.from(pair)), [['1', '42'], ['1', 'null'], ['last', 'true']]);
  const seen = []; form.forEach((v, k, f) => { assert.equal(f, form); seen.push([k, v]); });
  assert.equal(seen.length, 3);
  form.set('1', 7); assert.deepEqual(Array.from(form.getAll('1')), ['7']);
  form.delete('1'); assert.equal(form.has('1'), false);
}
test('FormData controls and runtime coerce scalars and iterate in order', () => {
  formChecks(FormData); formChecks(context.FormData);
});
vm.runInContext("globalThis.__STATUS_TEXT = {200: 'OK'};", context);
vm.runInContext(between('const __B64CHARS', "if (typeof globalThis.btoa"), context);
vm.runInContext(between("if (typeof globalThis.fetch", "if (typeof globalThis.FormData"), context);
async function fetchChecks(fetcher) {
  const response = await fetcher('data:text/plain,hello%20world');
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('content-type'), 'text/plain');
  const clone = response.clone();
  assert.equal(await response.text(), 'hello world');
  assert.equal(response.bodyUsed, true);
  await assert.rejects(response.text(), TypeError);
  assert.equal(await clone.text(), 'hello world');
  const binary = await fetcher('data:application/octet-stream;base64,AP8=');
  assert.deepEqual(Array.from(new Uint8Array(await binary.arrayBuffer())), [0, 255]);
  const json = await fetcher('data:application/json,%7B%22a%22:1%7D');
  assert.equal((await json.json()).a, 1);
  await assert.rejects(fetcher('data:missing-comma'), TypeError);
  await assert.rejects(fetcher('data:;base64,%%%'), TypeError);
}
test('offline data fetch controls and runtime expose real Response bytes', async () => {
  await fetchChecks(fetch); await fetchChecks(context.fetch);
});
