// The default clock reads the real wall clock, as Node's does (#447, #448): Date.now(), new Date()
// and performance.timeOrigin are near the host's time at the run (REAL_NOW, set by the harness)
// even when the framework snapshot is cached from an earlier run; Date.now() advances in
// synchronous code; and a timer wait still shows on Date.now(). Also passes under node --test.
const test = globalThis.test || require('node:test');
const assert = require('node:assert/strict');

const realNow = Number(process.env.REAL_NOW);
const near = (value, label) => assert.ok(Math.abs(value - realNow) < 1500, `${label} is ${value - realNow} ms from the host clock`);

test('Date and performance.timeOrigin read the host clock at the run', () => {
  near(Date.now(), 'Date.now()');
  near(new Date().getTime(), 'new Date()');
  near(performance.timeOrigin, 'performance.timeOrigin');
  assert.ok(performance.now() < 1500, `performance.now() is ${performance.now()} ms after timeOrigin`);
});

test('Date.now() advances in synchronous code', () => {
  const start = Date.now();
  const perfStart = performance.now();
  while (performance.now() - perfStart < 250) { /* busy */ }
  assert.ok(Date.now() - start >= 200, `Date.now() moved ${Date.now() - start} ms over a 250 ms busy loop`);
});

test('a timer wait shows on Date.now()', async () => {
  const start = Date.now();
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.ok(Date.now() - start >= 99, `Date.now() moved ${Date.now() - start} ms over setTimeout(100)`);
});
