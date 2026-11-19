//! A `class X extends Date` keeps `new.target` under the runner's virtual and fake-timer Date
//! (turbo-test#416): @date-fns/tz's TZDate relies on it. Oracle: Node (the fixture runs under
//! `node --test` first) and Jest 30's fake timers.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const FAKE: &str = r#"
const assert = require('node:assert/strict');
class Early extends Date { early() { return this.getTime(); } }
test('subclasses follow the fake clock and keep their prototype', () => {
  jest.useFakeTimers();
  jest.setSystemTime(new Date('2024-03-01T00:00:00.000Z'));
  try {
    class Late extends Date { late() { return this.toISOString(); } }
    const late = new Late();
    assert.equal(late instanceof Late, true);
    assert.equal(late instanceof Date, true);
    assert.equal(late.late(), '2024-03-01T00:00:00.000Z');
    const early = new Early();
    assert.equal(early instanceof Early, true);
    // defined before useFakeTimers: it extends the real Date, which Jest does not fake
    assert.equal(early.early() > Date.parse('2025-01-01T00:00:00.000Z'), true);
    assert.equal(new Late(0).late(), '1970-01-01T00:00:00.000Z');
    jest.advanceTimersByTime(1000);
    assert.equal(Late.now(), Date.parse('2024-03-01T00:00:01.000Z'));
    assert.equal(Object.getPrototypeOf(Reflect.construct(Date, [], Late)), Late.prototype);
    assert.equal(typeof Date(), 'string');
    globalThis.Late = Late;
  } finally {
    jest.useRealTimers();
  }
  const after = new globalThis.Late(5);
  assert.equal(after instanceof globalThis.Late, true);
  assert.equal(after.late(), '1970-01-01T00:00:00.005Z');
  assert.equal(new Early(7).early(), 7);
});
"#;

#[test]
fn date_subclasses_keep_new_target_like_node() {
    let p = Project::new("date-subclass-416");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"date-subclass-416"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    let fixture = p.file(
        "subclass.test.cjs",
        include_str!("data/compat-085/date-subclass.cjs"),
    );
    let node = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "{}{}",
        String::from_utf8_lossy(&node.stdout),
        String::from_utf8_lossy(&node.stderr)
    );
    p.file("fake.test.cjs", FAKE);
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn date_subclass_under_vitest_fake_timers() {
    let p = Project::new("date-subclass-416-vitest");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "fake.test.js",
        r#"
import { vi, test, expect } from 'vitest';
test('vi fake timers construct subclasses with new.target', () => {
  vi.useFakeTimers({ now: new Date('2030-05-05T05:05:05.000Z') });
  class Sub extends Date { iso() { return this.toISOString(); } }
  try {
    expect(new Sub()).toBeInstanceOf(Sub);
    expect(new Sub().iso()).toBe('2030-05-05T05:05:05.000Z');
    expect(new Sub(2030, 0, 2).getFullYear()).toBe(2030);
  } finally {
    vi.useRealTimers();
  }
  expect(new Sub(0).iso()).toBe('1970-01-01T00:00:00.000Z');
});
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
