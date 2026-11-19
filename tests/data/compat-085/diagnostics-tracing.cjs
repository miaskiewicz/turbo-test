// node:diagnostics_channel tracing channels and bound stores (turbo-test#407).
// Every expectation below was taken from real Node v26.11.0 (`node --test` runs this file too).
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const dc = require('node:diagnostics_channel');
const { AsyncLocalStorage } = require('node:async_hooks');

const EVENTS = ['start', 'end', 'asyncStart', 'asyncEnd', 'error'];
let unique = 0;
function recorder(tc) {
  const log = [];
  const handlers = {};
  for (const event of EVENTS) {
    handlers[event] = (message, name) => log.push([event, name, Object.keys(message), message.error && message.error.message, message.result]);
  }
  tc.subscribe(handlers);
  return { log, handlers };
}
function traced(prefix) {
  const name = prefix + '-' + unique++;
  const tc = dc.tracingChannel(name);
  return { name, tc, ...recorder(tc) };
}

testCase('tracingChannel exposes named channels and subscriber state', () => {
  assert.equal(typeof dc.tracingChannel, 'function');
  const tc = dc.tracingChannel('pino_asJson');
  assert.equal(tc.hasSubscribers, false);
  assert.equal(tc.start, dc.channel('tracing:pino_asJson:start'));
  assert.deepEqual(EVENTS.map((event) => tc[event].name), EVENTS.map((event) => 'tracing:pino_asJson:' + event));
  assert.deepEqual(Object.getOwnPropertyNames(tc), ['error']);
  const onError = () => {};
  tc.subscribe({ error: onError });
  assert.equal(tc.hasSubscribers, true);
  assert.equal(tc.error.hasSubscribers, true);
  assert.equal(tc.start.hasSubscribers, false);
  assert.equal(tc.unsubscribe({ error: onError }), true);
  assert.equal(tc.unsubscribe({ error: onError }), false);
  assert.equal(tc.unsubscribe({}), true);
  assert.equal(tc.hasSubscribers, false);
});

testCase('tracingChannel accepts an object of channels and rejects other inputs', () => {
  const channels = {};
  for (const event of EVENTS) channels[event] = dc.channel('object-' + event);
  const tc = dc.tracingChannel(channels);
  for (const event of EVENTS) assert.equal(tc[event], channels[event]);
  assert.equal(dc.tracingChannel(tc).asyncEnd, channels.asyncEnd);
  assert.throws(() => dc.tracingChannel({ ...channels, error: 'x' }), (e) => e instanceof TypeError && e.code === 'ERR_INVALID_ARG_TYPE');
  assert.throws(() => dc.tracingChannel(5), (e) => e.code === 'ERR_INVALID_ARG_TYPE' && /Received type number \(5\)/.test(e.message));
  assert.throws(() => dc.tracingChannel(Symbol('s')), (e) => e.code === 'ERR_INVALID_ARG_TYPE' && /Received type symbol \(Symbol\(s\)\)/.test(e.message));
});

testCase('traceSync publishes start/end around the call with result and thisArg/args', () => {
  const { name, tc, log } = traced('sync');
  const context = { a: 1 };
  const result = tc.traceSync(function (x, y) { log.push(['fn', this.t, x, y]); return x + y; }, context, { t: 'T' }, 2, 3);
  assert.equal(result, 5);
  assert.deepEqual(log, [
    ['start', 'tracing:' + name + ':start', ['a'], undefined, undefined],
    ['fn', 'T', 2, 3],
    ['end', 'tracing:' + name + ':end', ['a', 'result'], undefined, 5],
  ]);
  assert.deepEqual(context, { a: 1, result: 5 });
  log.length = 0;
  tc.traceSync(() => 1);
  assert.deepEqual(log.map((entry) => [entry[0], entry[2]]), [['start', []], ['end', ['result']]]);
});

testCase('traceSync publishes error before end and rethrows', () => {
  const { tc, log } = traced('sync-error');
  const boom = new Error('boom');
  const context = {};
  assert.throws(() => tc.traceSync(() => { throw boom; }, context), (e) => e === boom);
  assert.deepEqual(log.map((entry) => [entry[0], entry[2], entry[3]]), [
    ['start', [], undefined],
    ['error', ['error'], 'boom'],
    ['end', ['error'], 'boom'],
  ]);
  assert.equal(context.error, boom);
});

testCase('traceSync default context has no prototype and untraced calls skip publishing', () => {
  const tc = dc.tracingChannel('default-context-' + unique++);
  let seen;
  tc.subscribe({ start: (message) => { seen = message; } });
  tc.traceSync(() => 'x');
  assert.equal(Object.getPrototypeOf(seen), null);
  assert.equal(seen.result, 'x');
  const quiet = dc.tracingChannel('quiet-' + unique++);
  assert.equal(quiet.traceSync(function (a) { return [this, a]; }, undefined, 'self', 9)[1], 9);
  assert.equal(quiet.traceCallback((cb) => 3, 0, {}, null, 5), 3);
});

testCase('tracePromise publishes asyncStart/asyncEnd after resolution', async () => {
  const { tc, log } = traced('promise');
  const promise = tc.tracePromise(async (value) => { log.push(['fn']); return value * 2; }, {}, null, 21);
  log.push(['returned', promise instanceof Promise]);
  assert.equal(await promise, 42);
  assert.deepEqual(log.map((entry) => [entry[0], entry[2], entry[4]]), [
    ['start', [], undefined],
    ['fn', undefined, undefined],
    ['end', [], undefined],
    ['returned', undefined, undefined],
    ['asyncStart', ['result'], 42],
    ['asyncEnd', ['result'], 42],
  ]);
});

testCase('tracePromise publishes error then asyncStart/asyncEnd on rejection and rejects', async () => {
  const { tc, log } = traced('promise-reject');
  const context = {};
  await assert.rejects(tc.tracePromise(() => Promise.reject(new Error('rej')), context), /rej/);
  assert.deepEqual(log.map((entry) => [entry[0], entry[2], entry[3]]), [
    ['start', [], undefined],
    ['end', [], undefined],
    ['error', ['error'], 'rej'],
    ['asyncStart', ['error'], 'rej'],
    ['asyncEnd', ['error'], 'rej'],
  ]);
});

testCase('tracePromise returns non-native thenables as-is and warns on non-thenables', async () => {
  const { tc, log } = traced('thenable');
  const thenable = { then(resolve) { resolve('t'); } };
  assert.equal(tc.tracePromise(() => thenable, {}), thenable);
  assert.deepEqual(log.map((entry) => [entry[0], entry[4]]), [['start', undefined], ['asyncStart', 't'], ['asyncEnd', 't'], ['end', 't']]);
  log.length = 0;
  const rejecting = { then(resolve, reject) { reject(new Error('nope')); } };
  assert.equal(tc.tracePromise(() => rejecting, {}), rejecting);
  assert.deepEqual(log.map((entry) => [entry[0], entry[3]]), [['start', undefined], ['error', 'nope'], ['asyncStart', 'nope'], ['asyncEnd', 'nope'], ['end', 'nope']]);
  log.length = 0;
  const warnings = [];
  const onWarning = (warning) => warnings.push(warning.message);
  process.on('warning', onWarning);
  const context = {};
  assert.equal(tc.tracePromise(function named() { return 7; }, context), 7);
  assert.equal(dc.tracingChannel('unobserved-' + unique++).tracePromise(() => 8), 8);
  await new Promise((resolve) => setTimeout(resolve, 5));
  process.off('warning', onWarning);
  assert.deepEqual(log.map((entry) => [entry[0], entry[4]]), [['start', undefined], ['end', 7]]);
  assert.equal(context.result, 7);
  assert.deepEqual(warnings, [
    "tracePromise was called with the function 'named', which returned a non-thenable.",
    "tracePromise was called with the function '<anonymous>', which returned a non-thenable.",
  ]);
});

testCase('tracePromise publishes error when the function throws synchronously', () => {
  const { tc, log } = traced('promise-throw');
  assert.throws(() => tc.tracePromise(() => { throw new Error('sync'); }, {}), /sync/);
  assert.deepEqual(log.map((entry) => entry[0]), ['start', 'error', 'end']);
});

testCase('traceCallback wraps the callback at the last position', async () => {
  const { tc, log } = traced('callback');
  const returned = await new Promise((resolve) => {
    const value = tc.traceCallback(function (a, cb) {
      log.push(['fn', a]);
      setTimeout(() => cb(null, a + 1), 1);
      return 'ret';
    }, -1, {}, null, 1, (err, v) => { log.push(['cb', err, v]); resolve(value); });
  });
  assert.equal(returned, 'ret');
  assert.deepEqual(log.map((entry) => (entry.length === 5 ? [entry[0], entry[2], entry[4]] : entry)), [
    ['start', [], undefined],
    ['fn', 1],
    ['end', [], undefined],
    ['asyncStart', ['result'], 2],
    ['cb', null, 2],
    ['asyncEnd', ['result'], 2],
  ]);
});

testCase('traceCallback publishes callback errors at an explicit position', async () => {
  const { tc, log } = traced('callback-error');
  let receiver;
  await new Promise((resolve) => {
    tc.traceCallback((cb, a) => { cb.call('self', new Error('cberr'), a); }, 0, {}, null, function (err) {
      receiver = this;
      log.push(['cb', err.message]);
      resolve();
    }, 5);
  });
  assert.equal(String(receiver), 'self');
  assert.deepEqual(log.map((entry) => [entry[0], entry[0] === 'cb' ? entry[1] : entry[3]]), [
    ['start', undefined],
    ['error', 'cberr'],
    ['asyncStart', 'cberr'],
    ['cb', 'cberr'],
    ['asyncEnd', 'cberr'],
    ['end', 'cberr'],
  ]);
});

testCase('traceCallback validates the callback and its default context is frozen', () => {
  const { tc, log } = traced('callback-validate');
  assert.throws(() => tc.traceCallback(() => {}, -1, {}, null, 1), (e) => e instanceof TypeError && e.code === 'ERR_INVALID_ARG_TYPE');
  assert.deepEqual(log, []);
  assert.throws(() => tc.traceCallback((cb) => cb(null, 1), 0, undefined, null, () => {}), /not extensible/);
  const thrown = new Error('inner');
  const context = {};
  assert.throws(() => tc.traceCallback(() => { throw thrown; }, 0, context, null, () => {}), (e) => e === thrown);
  assert.equal(context.error, thrown);
});

testCase('bound stores wrap start, the traced call and end', () => {
  const store = new AsyncLocalStorage();
  const tc = dc.tracingChannel('stores-' + unique++);
  tc.start.bindStore(store, (data) => ({ wrapped: data.id }));
  assert.equal(tc.hasSubscribers, true);
  assert.equal(tc.start.hasSubscribers, true);
  const seen = [];
  tc.subscribe({ start: () => seen.push(['start', store.getStore()]), end: () => seen.push(['end', store.getStore()]) });
  tc.traceSync(() => seen.push(['fn', store.getStore()]), { id: 5 });
  seen.push(['after', store.getStore()]);
  assert.deepEqual(seen, [['start', { wrapped: 5 }], ['fn', { wrapped: 5 }], ['end', { wrapped: 5 }], ['after', undefined]]);
});

testCase('channel runStores, bindStore and unbindStore follow Node', () => {
  const store = new AsyncLocalStorage();
  const other = new AsyncLocalStorage();
  const channel = dc.channel('run-stores-' + unique++);
  assert.deepEqual(channel.runStores({}, function (a) { return [this.x, a]; }, { x: 1 }, 2), [1, 2]);
  channel.bindStore(store);
  channel.bindStore(other, (data) => data.q + 1);
  const published = [];
  channel.subscribe((message) => published.push([message, store.getStore(), other.getStore()]));
  const data = { q: 1 };
  assert.deepEqual(channel.runStores(data, () => [store.getStore(), other.getStore()]), [data, 2]);
  assert.deepEqual(published, [[data, data, 2]]);
  assert.equal(channel.unbindStore(store), true);
  assert.equal(channel.unbindStore(store), false);
  assert.equal(channel.unbindStore(other), true);
  assert.equal(channel.hasSubscribers, true);
});
