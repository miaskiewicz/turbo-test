//! Jest ignores what a hook returns unless it is a promise (it awaits that); vitest calls a
//! function returned by beforeEach / beforeAll as its teardown (turbo-test#417). A jest project
//! must not call `beforeEach(() => mock.mockClear())`'s returned mock, nor
//! `beforeAll(() => Model.init())`'s returned class. Oracle: Jest 30 / vitest 5.1.0.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const JEST: &str = r#"
const calls = [];
const boom = () => { throw new Error('a returned value was called as a teardown'); };
class Model { static init() { return this; } }
beforeAll(() => Model.init());
beforeAll(() => boom);
beforeAll(async () => boom);
describe('outer', () => {
  describe('inner', () => {
    const w = jest.fn((rows) => Promise.resolve(rows.map((r) => r + 1)));
    beforeEach(() => w.mockClear());
    beforeEach(() => jest.fn().mockReset());
    beforeEach(async () => { await null; calls.push('async'); return boom; });
    beforeEach(() => 42);
    it('calls a describe-scoped jest.fn with an arrow impl', async () => {
      expect(await w([1, 2])).toEqual([2, 3]);
    });
    it('runs again after the first test', async () => {
      expect(w).not.toHaveBeenCalled();
      expect(await w([5])).toEqual([6]);
    });
  });
});
afterAll(() => {
  expect(calls).toEqual(['async', 'async']);
});
it('runs at the top level', () => { expect(Model.init()).toBe(Model); });
"#;

#[test]
fn jest_hook_return_values_are_not_teardowns() {
    let p = Project::new("jest-hook-return-417");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file("hooks.test.js", JEST);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    assert_eq!(
        report["testResults"][0]["status"],
        "passed",
        "{}",
        run.all()
    );
    assert_eq!(run.code, Some(0), "{}", run.all());
}

#[test]
fn vitest_hook_returned_functions_stay_teardowns() {
    let p = Project::new("vitest-hook-return-417");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "hooks.test.js",
        r#"
import { beforeAll, beforeEach, afterAll, it, expect } from 'vitest';
const log = [];
beforeAll(() => () => log.push('all-teardown'));
beforeEach(() => () => log.push('each-teardown'));
it('first', () => { log.push('first'); });
it('second', () => { expect(log).toEqual(['first', 'each-teardown']); });
afterAll(() => { expect(log).toEqual(['first', 'each-teardown', 'each-teardown']); });
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(
        report["testResults"][0]["status"],
        "passed",
        "{}",
        run.all()
    );
    assert_eq!(run.code, Some(0), "{}", run.all());
}
