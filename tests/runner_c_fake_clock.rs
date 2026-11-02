//! Fake timers run on their own clock (turbo-test#168). Rows recorded from vitest 4.1.12
//! (`vi.useFakeTimers()` with vitest's defaults: @sinonjs/fake-timers, `shouldAdvanceTime` off):
//! a faked timer fires only when the test advances the fake clock; timers set before
//! `useFakeTimers` and the test timeout stay on the real clock, which advancing the fake clock
//! does not move; fake timers still pending at `useRealTimers` never fire; a faked `Date` stands
//! still while the test awaits real time.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

const FILE: &str = r#"import { test, expect, vi, afterEach } from 'vitest'
afterEach(() => { vi.useRealTimers() })
test('fake timers do not fire on their own', async () => {
  vi.useFakeTimers()
  let fired = false
  setTimeout(() => { fired = true }, 100)
  await new Promise((r) => setTimeout(r, 50))
  expect(fired).toBe(false)
}, 300)
test('a real timer set before useFakeTimers still fires', async () => {
  const log = []
  const done = new Promise((r) => setTimeout(() => { log.push('real'); r() }, 20))
  vi.useFakeTimers()
  setTimeout(() => log.push('fake'), 5)
  await done
  expect(log).toEqual(['real'])
  expect(vi.getTimerCount()).toBe(1)
  vi.advanceTimersByTime(5)
  expect(log).toEqual(['real', 'fake'])
})
test('advancing the fake clock does not bring the test timeout closer', async () => {
  vi.useFakeTimers()
  vi.advanceTimersByTime(10000)
  vi.useRealTimers()
  await new Promise((r) => setTimeout(r, 10))
  expect(1).toBe(1)
}, 300)
test('fake timers still pending at useRealTimers never fire', async () => {
  vi.useFakeTimers()
  let fired = false
  setTimeout(() => { fired = true }, 10)
  vi.useRealTimers()
  await new Promise((r) => setTimeout(r, 30))
  expect(fired).toBe(false)
})
test('Date.now under fake timers does not move while awaiting real time', async () => {
  const p = new Promise((r) => setTimeout(r, 40))
  vi.useFakeTimers()
  const t0 = Date.now()
  await p
  expect(Date.now() - t0).toBe(0)
  vi.advanceTimersByTime(25)
  expect(Date.now() - t0).toBe(25)
})
"#;

#[test]
fn fake_timers_fire_only_when_the_fake_clock_advances() {
    let p = Project::new("rc-fake-clock");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file("fake.test.js", FILE);
    for reuse in ["0", "1"] {
        let r = turbo_test(
            &p.dir,
            &["--reporter=json", "fake.test.js"],
            &[("TURBO_REUSE_ISOLATE", reuse)],
        );
        let json: serde_json::Value =
            serde_json::from_str(&r.stdout).unwrap_or_else(|e| panic!("json: {e}\n{}", r.all()));
        let rows: Vec<(String, String, String)> = json["testResults"][0]["assertionResults"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let first = t["failureMessages"][0]
                    .as_str()
                    .map_or("", |m| m.lines().next().unwrap())
                    .to_string();
                (
                    t["title"].as_str().unwrap().to_string(),
                    t["status"].as_str().unwrap().to_string(),
                    first,
                )
            })
            .collect();
        let row = |title: &str, status: &str, msg: &str| {
            (title.to_string(), status.to_string(), msg.to_string())
        };
        assert_eq!(
            rows,
            vec![
                row(
                    "fake timers do not fire on their own",
                    "failed",
                    // vitest's json: the timeout error's stack is the test's registration
                    "Error: STACK_TRACE_ERROR"
                ),
                row(
                    "a real timer set before useFakeTimers still fires",
                    "passed",
                    ""
                ),
                row(
                    "advancing the fake clock does not bring the test timeout closer",
                    "passed",
                    ""
                ),
                row(
                    "fake timers still pending at useRealTimers never fire",
                    "passed",
                    ""
                ),
                row(
                    "Date.now under fake timers does not move while awaiting real time",
                    "passed",
                    ""
                ),
            ],
            "reuse={reuse}\n{}",
            r.all()
        );
        assert_eq!(r.code, Some(1), "{}", r.all());
    }
    // the human reporter names the timeout, as vitest's does
    let r = turbo_test(
        &p.dir,
        &["fake.test.js"],
        &[("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")],
    );
    assert!(
        r.all().contains("Error: Test timed out in 300ms."),
        "{}",
        r.all()
    );
}

/// Rows recorded from vitest 4.1.12 (all pass there): vitest fakes the global timer functions
/// only, so node:timers, node:timers/promises and a setImmediate captured before
/// `useFakeTimers` stay on the real clock; `vi.waitFor` advances fake timers by its interval;
/// the `*Async` advance functions yield a real macrotask around each timer, so timers set by
/// promise callbacks run, the sync ones do not; `setSystemTime` moves Date and leaves pending
/// timers due where they were; a second `useFakeTimers` starts a fresh clock.
const REAL_AND_ASYNC: &str = r#"import { test, expect, vi, afterEach } from 'vitest'
import { setTimeout as sleep } from 'node:timers/promises'
import * as timers from 'node:timers'
const capturedImmediate = setImmediate
afterEach(() => { vi.useRealTimers() })
test('node:timers and a captured setImmediate stay real', async () => {
  vi.useFakeTimers()
  await sleep(10)
  const log = []
  await new Promise((r) => timers.setTimeout(() => { log.push('timers'); r() }, 10))
  await new Promise((r) => capturedImmediate(() => { log.push('captured'); r() }))
  expect([log, vi.getTimerCount(), setTimeout === timers.setTimeout, typeof setTimeout.clock]).toEqual([['timers', 'captured'], 0, false, 'object'])
  vi.useRealTimers()
  expect([setTimeout === timers.setTimeout, setTimeout.clock]).toEqual([true, undefined])
}, 500)
test('waitFor advances fake timers by its interval', async () => {
  vi.useFakeTimers()
  let done = false
  setTimeout(() => { done = true }, 120)
  const t0 = Date.now()
  await vi.waitFor(() => { if (!done) throw new Error('no') }, { interval: 50, timeout: 1000 })
  expect(Date.now() - t0).toBe(150)
})
test('the async advance functions run timers set by promise callbacks', async () => {
  vi.useFakeTimers()
  const log = []
  const chain = async (tag) => { await new Promise((r) => setTimeout(r, 10)); log.push(tag + 1); await new Promise((r) => setTimeout(r, 10)); log.push(tag + 2) }
  chain('a')
  await vi.advanceTimersByTimeAsync(20)
  chain('b')
  await vi.runAllTimersAsync()
  chain('c')
  await vi.advanceTimersToNextTimerAsync()
  expect([log, vi.getTimerCount()]).toEqual([['a1', 'a2', 'b1', 'b2', 'c1'], 1])
  await vi.runOnlyPendingTimersAsync()
  expect(log).toEqual(['a1', 'a2', 'b1', 'b2', 'c1', 'c2'])
})
test('the sync advance runs only timers already set', async () => {
  vi.useFakeTimers()
  const log = []
  const chain = async () => { await new Promise((r) => setTimeout(r, 10)); log.push(1); await new Promise((r) => setTimeout(r, 10)); log.push(2) }
  chain()
  vi.advanceTimersByTime(20)
  await null
  await null
  expect([log, vi.getTimerCount()]).toEqual([[1], 1])
})
test('setSystemTime moves Date, not the pending fake timers', () => {
  vi.useFakeTimers()
  let fired = 0
  setTimeout(() => { fired++ }, 100)
  vi.setSystemTime(new Date('2020-01-01T00:00:00Z'))
  vi.advanceTimersByTime(99)
  expect([fired, new Date().toISOString()]).toEqual([0, '2020-01-01T00:00:00.099Z'])
  vi.advanceTimersByTime(1)
  expect(fired).toBe(1)
})
test('useFakeTimers again starts a fresh clock', () => {
  vi.useFakeTimers()
  let fired = 0
  setTimeout(() => { fired++ }, 10)
  vi.useFakeTimers()
  expect(vi.getTimerCount()).toBe(0)
  vi.advanceTimersByTime(20)
  expect(fired).toBe(0)
})
"#;

#[test]
fn fake_timers_leave_node_timers_real_and_yield_in_the_async_functions() {
    let p = Project::new("rc-fake-clock-async");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file("nodet.test.js", REAL_AND_ASYNC);
    for reuse in ["0", "1"] {
        let r = turbo_test(
            &p.dir,
            &["nodet.test.js"],
            &[("TURBO_REUSE_ISOLATE", reuse)],
        );
        assert_eq!(r.code, Some(0), "reuse={reuse}\n{}", r.all());
        assert!(r.all().contains("6 passed"), "reuse={reuse}\n{}", r.all());
    }
}
