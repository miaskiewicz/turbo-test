const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { setTimeout: realTimeout } = require('node:timers');
const wait = (delay) => new Promise((resolve) => realTimeout(resolve, delay));

testCase('AbortSignal.timeout aborts once with a TimeoutError reason', async () => {
  const signal = AbortSignal.timeout(1);
  assert.equal(signal.aborted, false);
  assert.equal(signal.reason, undefined);
  let events = 0;
  signal.addEventListener('abort', (event) => {
    assert.equal(event.type, 'abort');
    assert.equal(event.target, signal);
    events++;
  });
  await wait(20);
  assert.equal(signal.aborted, true);
  assert.equal(signal.reason instanceof DOMException, true);
  assert.equal(signal.reason.name, 'TimeoutError');
  assert.equal(signal.reason.code, 23);
  assert.throws(() => signal.throwIfAborted(), (error) => error === signal.reason);
  assert.equal(events, 1);
  await wait(5);
  assert.equal(events, 1);
});

testCase('AbortSignal.timeout zero delay follows synchronous work and microtasks', async () => {
  const order = [];
  const signal = AbortSignal.timeout(0);
  signal.onabort = () => order.push('abort');
  order.push('sync');
  await Promise.resolve().then(() => order.push('microtask'));
  assert.equal(signal.aborted, false);
  await wait(10);
  assert.deepEqual(order, ['sync', 'microtask', 'abort']);
});

testCase('AbortSignal.timeout validates Node delay types and range', () => {
  for (const delay of [undefined, null, '1', true, {}, 1n]) {
    assert.throws(() => AbortSignal.timeout(delay), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE' });
  }
  for (const delay of [-1, NaN, Infinity, -Infinity, 0.5, 4294967296]) {
    assert.throws(() => AbortSignal.timeout(delay), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  }
});

testCase('AbortSignal.timeout uses the real timer clock under fake timers', async (context) => {
  let tick;
  let restore;
  if (globalThis.jest) {
    jest.useFakeTimers();
    tick = (delay) => jest.advanceTimersByTime(delay);
    restore = () => jest.useRealTimers();
  } else {
    context.mock.timers.enable({ apis: ['setTimeout'] });
    tick = (delay) => context.mock.timers.tick(delay);
    restore = () => context.mock.timers.reset();
  }
  let signal;
  try {
    signal = AbortSignal.timeout(1);
    let fakeCalled = false;
    setTimeout(() => { fakeCalled = true; }, 5);
    tick(50);
    assert.equal(fakeCalled, true);
    assert.equal(signal.aborted, false);
  } finally {
    restore();
  }
  await wait(20);
  assert.equal(signal.aborted, true);
  assert.equal(signal.reason.name, 'TimeoutError');
});
