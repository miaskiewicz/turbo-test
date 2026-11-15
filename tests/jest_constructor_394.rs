//! Real Jest constructor mocks call implementations on the allocated mock receiver.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const CASES: &str = r#"
test('arrow object return keeps allocated instance distinct from result', () => {
  const value = { verify: jest.fn(() => 'ok') };
  const Mock = jest.fn(() => value);
  const result = new Mock('secret');
  expect(result).toBe(value);
  expect(result.verify()).toBe('ok');
  expect(Mock.mock.instances[0]).toBeInstanceOf(Mock);
  expect(Mock.mock.instances[0]).not.toBe(result);
  expect(Mock.mock.contexts[0]).toBe(Mock.mock.instances[0]);
  expect(Mock.mock.results[0]).toEqual({ type: 'return', value });
});
test('arrow primitive return uses allocated receiver but records primitive', () => {
  const Mock = jest.fn(() => 17);
  const result = new Mock();
  expect(result).toBeInstanceOf(Mock);
  expect(Mock.mock.instances[0]).toBe(result);
  expect(Mock.mock.results[0]).toEqual({ type: 'return', value: 17 });
});
test('constructor throw preserves receiver and error identity', () => {
  const error = new Error('constructor sentinel');
  const Mock = jest.fn(() => { throw error; });
  expect(() => new Mock()).toThrow(error);
  expect(Mock.mock.instances[0]).toBeInstanceOf(Mock);
  expect(Mock.mock.contexts[0]).toBe(Mock.mock.instances[0]);
  expect(Mock.mock.results[0]).toEqual({ type: 'throw', value: error });
});
test('normal implementation sees receiver but no implementation new.target', () => {
  let seenTarget;
  const Mock = jest.fn(function (value) { seenTarget = new.target; this.value = value; });
  function Derived() {}
  const result = Reflect.construct(Mock, [9], Derived);
  expect(Object.getPrototypeOf(result)).toBe(Derived.prototype);
  expect(result.value).toBe(9);
  expect(seenTarget).toBeUndefined();
  expect(Mock.mock.instances[0]).toBe(result);
  expect(Mock.mock.results[0].value).toBeUndefined();
});
test('Jest class implementation fails like direct class apply', () => {
  class Value { constructor() { this.value = 1; } }
  const Mock = jest.fn(Value);
  expect(() => new Mock()).toThrow(TypeError);
  expect(Mock.mock.instances[0]).toBeInstanceOf(Mock);
  expect(Mock.mock.results[0].type).toBe('throw');
});
test('queued arrow implementation and empty constructor preserve receivers', () => {
  const Mock = jest.fn().mockImplementationOnce(() => ({ once: true }));
  expect(new Mock()).toEqual({ once: true });
  const second = new Mock();
  expect(second).toBeInstanceOf(Mock);
  expect(Mock.mock.instances[1]).toBe(second);
  expect(Mock.mock.results[1].value).toBeUndefined();
});
"#;

#[test]
fn jest_constructor_implementations_follow_jest_receivers() {
    let p = Project::new("jest-constructor-394");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file("constructor.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 6, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn vitest_constructor_class_and_arrow_contract_remains_unchanged() {
    let p = Project::new("vitest-constructor-394");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "constructor.test.js",
        r#"
import { vi, test, expect } from 'vitest';
test('Vitest class remains constructible and arrow remains rejected', () => {
  class Value { constructor(value) { this.value = value; } }
  const Mock = vi.fn(Value);
  const value = new Mock(7);
  expect(value.value).toBe(7);
  expect(value).toBeInstanceOf(Mock);
  expect(Mock.mock.instances[0]).toBe(value);
  expect(() => new (vi.fn(() => ({ value: 1 })))()).toThrow(TypeError);
});
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
