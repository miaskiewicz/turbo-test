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
#[ignore = "turbo-test#168: fails until the fix"]
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
    let r = turbo_test(&p.dir, &["fake.test.js"], &[("TURBO_LEGACY_REPORTER", "0")]);
    assert!(
        r.all().contains("Error: Test timed out in 300ms."),
        "{}",
        r.all()
    );
}
