//! An automocked class is a Jest mock constructor (turbo-test#421): under `new`, Jest applies the
//! implementation to the allocated receiver, so `Writer.mockImplementation(() => ({...}))` makes
//! `new Writer()` return that object, as a plain `jest.fn()` does (turbo-test#394). vitest's
//! automock keeps constructing the implementation. Oracle: Jest 30 / vitest 5.1.0.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const WRITER: &str = "class Writer { write() { return 'real'; } }\nmodule.exports = { Writer };\n";

const JEST: &str = r#"
jest.mock('./writer');
const { Writer } = require('./writer');
afterEach(() => Writer.mockReset());
test('arrow implementation object is what new returns', () => {
  const fake = { write: () => 'fake' };
  Writer.mockImplementation(() => fake);
  const w = new Writer('a');
  expect(w).toBe(fake);
  expect(w.write()).toBe('fake');
  expect(Writer).toHaveBeenCalledWith('a');
  expect(Writer.mock.instances[0]).toBeInstanceOf(Writer);
  expect(Writer.mock.instances[0]).not.toBe(fake);
  expect(Writer.mock.results[0]).toEqual({ type: 'return', value: fake });
});
test('once arrow implementation, then the automocked instance', () => {
  Writer.mockImplementationOnce(() => ({ write: () => 'once' }));
  expect(new Writer().write()).toBe('once');
  const second = new Writer();
  expect(second).toBeInstanceOf(Writer);
  expect(second.write()).toBeUndefined();
  expect(Writer.mock.instances[1]).toBe(second);
});
test('function implementation sets fields on the receiver', () => {
  Writer.mockImplementation(function (name) { this.name = name; });
  const w = new Writer('n');
  expect(w).toBeInstanceOf(Writer);
  expect(w.name).toBe('n');
  expect(Writer.mock.instances[0]).toBe(w);
});
test('createMockFromModule classes behave the same', () => {
  const { Writer: Made } = jest.createMockFromModule('./writer');
  Made.mockImplementation(() => ({ write: () => 'made' }));
  expect(new Made().write()).toBe('made');
});
"#;

#[test]
fn jest_automocked_class_applies_arrow_implementations_under_new() {
    let p = Project::new("jest-automock-new-421");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file("writer.js", WRITER);
    p.file("writer.test.js", JEST);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 4, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn vitest_automocked_class_rejects_arrow_implementations_under_new() {
    let p = Project::new("vitest-automock-new-421");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "writer.js",
        "export class Writer { write() { return 'real'; } }\n",
    );
    p.file(
        "writer.test.js",
        r#"
import { vi, test, expect } from 'vitest';
import { Writer } from './writer.js';
vi.mock('./writer.js');
test('an arrow implementation is rejected under new', () => {
  expect(new Writer()).toBeInstanceOf(Writer);
  Writer.mockImplementation(() => ({ write: () => 'arrow' }));
  expect(() => new Writer()).toThrow(TypeError);
});
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
