//! The fake-timer API against vitest 4.1.12 (every file's rows were recorded with real vitest,
//! where its tests pass): the advance / run functions throw without fake timers
//! (turbo-test#171); `performance` / `process.hrtime` faking and the `toFake`, `now`,
//! `shouldAdvanceTime` / `advanceTimeDelta` and `loopLimit` options (turbo-test#172);
//! `vi.waitFor` / `vi.waitUntil` results, timeouts, their messages and locations (turbo-test#174);
//! `runOnlyPendingTimers` runs to the last pending timer and a timer callback's error is
//! rethrown by the advance call (turbo-test#177); the real `performance.now()` /
//! `process.hrtime()` move with real time (turbo-test#178).
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

const API_171: &str = r#"import { test, expect, vi } from 'vitest'
const MSG = 'A function to advance timers was called but the timers APIs are not mocked. Call `vi.useFakeTimers()` in the test file first.'
const run = async (calls) => {
  const out = []
  for (const [k, f] of Object.entries(calls)) {
    try { const r = await f(); out.push([k, r === vi ? 'vi' : r]) } catch (e) { out.push([k, e.message === MSG ? 'MSG' : e.message]) }
  }
  return out
}
const calls = {
  advanceTimersByTime: () => vi.advanceTimersByTime(1), runAllTimers: () => vi.runAllTimers(),
  runOnlyPendingTimers: () => vi.runOnlyPendingTimers(), advanceTimersToNextTimer: () => vi.advanceTimersToNextTimer(),
  getTimerCount: () => vi.getTimerCount(), clearAllTimers: () => vi.clearAllTimers(), runAllTicks: () => vi.runAllTicks(),
  advanceTimersToNextFrame: () => vi.advanceTimersToNextFrame(),
  advanceTimersByTimeAsync: () => vi.advanceTimersByTimeAsync(1), runAllTimersAsync: () => vi.runAllTimersAsync(),
  runOnlyPendingTimersAsync: () => vi.runOnlyPendingTimersAsync(), advanceTimersToNextTimerAsync: () => vi.advanceTimersToNextTimerAsync(),
}
test('the advance functions throw without fake timers', async () => {
  expect(await run(calls)).toEqual([
    ['advanceTimersByTime', 'MSG'], ['runAllTimers', 'MSG'], ['runOnlyPendingTimers', 'MSG'], ['advanceTimersToNextTimer', 'MSG'],
    ['getTimerCount', 'MSG'], ['clearAllTimers', 'vi'], ['runAllTicks', 'MSG'], ['advanceTimersToNextFrame', 'MSG'],
    ['advanceTimersByTimeAsync', 'MSG'], ['runAllTimersAsync', 'MSG'], ['runOnlyPendingTimersAsync', 'MSG'], ['advanceTimersToNextTimerAsync', 'MSG'],
  ])
  // the async ones reject: no throw on call
  const p = vi.advanceTimersByTimeAsync(1)
  await expect(p).rejects.toThrow(MSG)
})
test('with fake timers they return vi (getTimerCount the count), after useRealTimers they throw again', async () => {
  vi.useFakeTimers()
  setTimeout(() => {}, 5)
  expect(await run(calls)).toEqual([
    ['advanceTimersByTime', 'vi'], ['runAllTimers', 'vi'], ['runOnlyPendingTimers', 'vi'], ['advanceTimersToNextTimer', 'vi'],
    ['getTimerCount', 0], ['clearAllTimers', 'vi'], ['runAllTicks', 'vi'], ['advanceTimersToNextFrame', 'vi'],
    ['advanceTimersByTimeAsync', 'vi'], ['runAllTimersAsync', 'vi'], ['runOnlyPendingTimersAsync', 'vi'], ['advanceTimersToNextTimerAsync', 'vi'],
  ])
  vi.useRealTimers()
  expect(() => vi.runAllTimers()).toThrow(MSG)
})
test('advanceTimersToNextFrame moves to the next 16ms frame', () => {
  vi.useFakeTimers()
  const t0 = performance.now()
  vi.advanceTimersByTime(5)
  vi.advanceTimersToNextFrame()
  const a = performance.now() - t0
  vi.advanceTimersToNextFrame()
  expect([a, performance.now() - t0]).toEqual([16, 32])
  vi.useRealTimers()
})
"#;

const OPTS_172: &str = r#"import { test, expect, vi, afterEach } from 'vitest'
import { performance as perfHooks } from 'node:perf_hooks'
afterEach(() => { vi.useRealTimers() })
test('performance.now and process.hrtime follow the fake clock, not setSystemTime', () => {
  const realPerf = globalThis.performance, realHrtime = process.hrtime
  vi.useFakeTimers({ now: 5000 })
  const p0 = performance.now(), h0 = process.hrtime(), b0 = process.hrtime.bigint()
  vi.advanceTimersByTime(1250)
  vi.setSystemTime(new Date('2020-01-01T00:00:00Z'))
  expect([p0, performance.now(), performance.timeOrigin, process.hrtime(), process.hrtime(h0), process.hrtime.bigint() - b0, typeof process.hrtime.clock])
    .toEqual([0, 1250, 5000, [1, 250000000], [1, 250000000], 1250000000n, 'object'])
  expect([globalThis.performance === realPerf, perfHooks === realPerf, process.hrtime === realHrtime, performance.mark('m').name, performance.getEntriesByName('m')]).toEqual([false, true, false, 'm', []])
  vi.useRealTimers()
  expect([globalThis.performance === realPerf, process.hrtime === realHrtime]).toEqual([true, true])
})
test('toFake fakes only the named functions', async () => {
  const realDate = Date
  vi.useFakeTimers({ toFake: ['setTimeout', 'performance'] })
  let im = 0
  await new Promise((r) => setImmediate(() => { im++; r() }))
  let st = 0
  setTimeout(() => { st++ }, 5)
  vi.advanceTimersByTime(5)
  expect([im, st, Date === realDate, typeof setImmediate.clock, typeof setTimeout.clock, typeof clearTimeout.clock, vi.isFakeTimers(), vi.getTimerCount(), performance.now()])
    .toEqual([1, 1, true, 'undefined', 'object', 'undefined', true, 0, 5])
})
test('now sets the fake clock start; a later call without it starts at the current Date', () => {
  vi.useFakeTimers({ now: new Date('2021-02-03T04:05:06Z') })
  const a = new Date().toISOString()
  vi.useFakeTimers({ now: 1000 })
  const b = Date.now()
  vi.advanceTimersByTime(10)
  vi.useFakeTimers()
  expect([a, b, Date.now(), performance.now()]).toEqual(['2021-02-03T04:05:06.000Z', 1000, 1010, 0])
})
test('shouldAdvanceTime moves the fake clock with real time', async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true, advanceTimeDelta: 10 })
  let fired = 0
  setTimeout(() => { fired++ }, 30)
  await new Promise((r) => setTimeout(r, 100))
  expect(fired).toBe(1)
  expect(performance.now() >= 100).toBe(true)
})
test('loopLimit: runAllTimers aborts an endless timer chain', async () => {
  vi.useFakeTimers({ loopLimit: 5 })
  let n = 0
  const tick = () => { n++; setTimeout(tick, 1) }
  tick()
  expect(() => vi.runAllTimers()).toThrow('Aborting after running 5 timers, assuming an infinite loop!')
  expect(n).toBe(6)
  await expect(vi.runAllTimersAsync()).rejects.toThrow('Aborting after running 5 timers, assuming an infinite loop!')
  expect(n).toBe(11)
})
test('the default loopLimit is 10000 (an interval makes runAllTimers abort)', () => {
  vi.useFakeTimers()
  let n = 0
  setInterval(() => { n++ }, 1)
  expect(() => vi.runAllTimers()).toThrow('Aborting after running 10000 timers, assuming an infinite loop!')
  expect(n).toBe(10000)
})
"#;

const WAIT_174: &str = r#"import { test, expect, vi } from 'vitest'
test('waitFor results', async () => {
  let calls = 0
  const out = []
  out.push(await vi.waitFor(() => false))
  out.push(await vi.waitFor(() => null))
  let n = 0
  out.push(await vi.waitFor(() => { if (++n < 3) throw new Error('not yet'); return n }, { interval: 5 }))
  let m = 0
  out.push(await vi.waitFor(() => (++m < 3 ? Promise.reject(new Error('rej')) : Promise.resolve('ok:' + m)), { interval: 5 }))
  const e1 = await vi.waitFor(() => { throw new Error('always') }, 30).catch((e) => e.message)
  const e2 = await vi.waitFor(() => new Promise(() => {}), { timeout: 30, interval: 5 }).catch((e) => e.message)
  out.push(e1, e2)
  out.push(await vi.waitUntil(() => 0 || 'truthy'))
  let k = 0
  out.push(await vi.waitUntil(() => ++k >= 3 && k, { interval: 5 }))
  const e3 = await vi.waitUntil(() => { throw new Error('boom') }).catch((e) => e.message)
  const e4 = await vi.waitUntil(() => false, 30).catch((e) => e.message)
  const e5 = await vi.waitUntil(() => Promise.reject(new Error('prej')), 30).catch((e) => e.message)
  const e6 = await vi.waitUntil(() => { calls++; return Promise.resolve(false) }, { timeout: 30, interval: 10 }).catch((e) => e.message)
  out.push(e3, e4, e5, e6, k)
  expect(out).toEqual([false, null, 3, 'ok:3', 'always', 'Timed out in waitFor!', 'truthy', 3, 'boom', 'Timed out in waitUntil!', 'prej', 'Timed out in waitUntil!', 3])
  expect(calls).toBe(3)
})
test('waitUntil timeout', async () => {
  await vi.waitUntil(() => false, { timeout: 30, interval: 10 })
})
test('waitFor timeout', async () => {
  await vi.waitFor(() => new Promise(() => {}), { timeout: 30, interval: 10 })
})
"#;

const RUN_177: &str = r#"import { test, expect, vi, afterEach } from 'vitest'
afterEach(() => { vi.useRealTimers() })
test('runOnlyPendingTimers runs every timer due up to the last pending one', async () => {
  vi.useFakeTimers()
  const log = []
  setInterval(() => log.push('i'), 10)
  setTimeout(() => { log.push('t'); setTimeout(() => log.push('in-range'), 5); setTimeout(() => log.push('late'), 100) }, 30)
  setTimeout(() => log.push('last'), 50)
  vi.runOnlyPendingTimers()
  expect([log.join(','), vi.getTimerCount(), Date.now() % 1000 >= 0]).toEqual(['i,i,i,t,in-range,i,i,last', 2, true])
  vi.clearAllTimers()
  setInterval(() => log.push('j'), 10)
  setTimeout(() => log.push('end'), 25)
  log.length = 0
  await vi.runOnlyPendingTimersAsync()
  expect(log.join(',')).toBe('j,j,end')
})
test('a timer callback error is rethrown after the tick', async () => {
  vi.useFakeTimers()
  const log = []
  setTimeout(() => { log.push(1); throw new Error('first') }, 1)
  setTimeout(() => { log.push(2); throw new Error('second') }, 2)
  setTimeout(() => log.push(3), 3)
  expect(() => vi.advanceTimersByTime(5)).toThrow('first')
  expect([log, vi.getTimerCount()]).toEqual([[1, 2, 3], 0])
  setTimeout(() => { log.push(4); throw new Error('ra') }, 1)
  setTimeout(() => log.push(5), 2)
  expect(() => vi.runAllTimers()).toThrow('ra')
  expect([log, vi.getTimerCount()]).toEqual([[1, 2, 3, 4], 1])
  vi.clearAllTimers()
  setTimeout(() => { log.push(6); throw new Error('nx') }, 1)
  expect(() => vi.advanceTimersToNextTimer()).toThrow('nx')
  setTimeout(() => { log.push(7); throw new Error('rop') }, 1)
  setTimeout(() => log.push(8), 2)
  expect(() => vi.runOnlyPendingTimers()).toThrow('rop')
  expect(log).toEqual([1, 2, 3, 4, 6, 7, 8])
  setTimeout(() => { log.push(9); throw new Error('async') }, 1)
  setTimeout(() => log.push(10), 2)
  await expect(vi.advanceTimersByTimeAsync(5)).rejects.toThrow('async')
  setTimeout(() => { log.push(11); throw new Error('raa') }, 1)
  setTimeout(() => log.push(12), 2)
  await expect(vi.runAllTimersAsync()).rejects.toThrow('raa')
  expect([log, vi.getTimerCount()]).toEqual([[1, 2, 3, 4, 6, 7, 8, 9, 10, 11], 1])
})
"#;

const PERF_178: &str = r#"import { test, expect } from 'vitest'
test('real performance.now and process.hrtime advance with real time', async () => {
  const p = performance.now(), h = process.hrtime(), b = process.hrtime.bigint()
  await new Promise((r) => setTimeout(r, 40))
  const dp = performance.now() - p, dh = process.hrtime(h), db = process.hrtime.bigint() - b
  expect([dp >= 39 && dp < 1000, dh[0], dh[1] >= 39e6 && dh[1] < 1e9, db >= 39000000n && db < 1000000000n, p > 0, typeof performance.timeOrigin, performance.timeOrigin + performance.now() - Date.now() < 1000])
    .toEqual([true, 0, true, true, true, 'number', true])
})
"#;

/// vitest's stderr for the two timeouts (`NO_COLOR=1`)
const WAIT_174_HUMAN: &str = r#" FAIL  wait.test.js > waitUntil timeout
Error: Timed out in waitUntil!
 ❯ wait.test.js:26:12
     24| })
     25| test('waitUntil timeout', async () => {
     26|   await vi.waitUntil(() => false, { timeout: 30, interval: 10 })
       |            ^
     27| })
     28| test('waitFor timeout', async () => {

 FAIL  wait.test.js > waitFor timeout
Error: Timed out in waitFor!
 ❯ wait.test.js:29:12
     27| })
     28| test('waitFor timeout', async () => {
     29|   await vi.waitFor(() => new Promise(() => {}), { timeout: 30, interva…
       |            ^
     30| })
     31|

"#;

fn passes(name: &str, file: &str, body: &str, tests: usize) {
    let p = Project::new(name);
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(file, body);
    for reuse in ["0", "1"] {
        let r = turbo_test(&p.dir, &[file], &[("TURBO_REUSE_ISOLATE", reuse)]);
        assert_eq!(r.code, Some(0), "reuse={reuse}\n{}", r.all());
        assert!(
            r.all().contains(&format!("{tests} passed")),
            "reuse={reuse}\n{}",
            r.all()
        );
    }
}

#[test]
fn advance_functions_throw_without_fake_timers() {
    passes("rc-ft-171", "api171.test.js", API_171, 3);
}

#[test]
fn performance_hrtime_and_the_fake_timer_options() {
    passes("rc-ft-172", "opts172.test.js", OPTS_172, 6);
}

#[test]
fn wait_for_and_wait_until_are_vitests() {
    let p = Project::new("rc-ft-174");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file("wait.test.js", WAIT_174);
    let r = turbo_test(
        &p.dir,
        &["wait.test.js"],
        &[("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(r.all().contains("2 failed | 1 passed (3)"), "{}", r.all());
    for block in WAIT_174_HUMAN
        .split("\n\n")
        .filter(|b| !b.trim().is_empty())
    {
        assert!(r.stderr.contains(block), "missing:\n{block}\n{}", r.all());
    }
}

#[test]
fn run_only_pending_runs_to_the_last_timer_and_timer_errors_are_rethrown() {
    passes("rc-ft-177", "run177.test.js", RUN_177, 2);
}

#[test]
fn real_performance_now_and_hrtime_move() {
    passes("rc-ft-178", "perf178.test.js", PERF_178, 1);
}
