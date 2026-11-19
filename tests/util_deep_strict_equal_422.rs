//! util.isDeepStrictEqual as Node's (turbo-test#422): own-key order never matters, while
//! strict deep equality (undefined-valued keys, -0, prototypes, cycles, Map/Set contents) still
//! decides the result, which a JSON-text comparison gets wrong.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r#"
const { isDeepStrictEqual } = require('node:util');
const util = require('util');
test('isDeepStrictEqual ignores own-key order at every depth', () => {
 expect(isDeepStrictEqual({ a: 1, b: [1], c: { x: 1, y: 2 } }, { c: { y: 2, x: 1 }, b: [1], a: 1 })).toBe(true);
 expect(isDeepStrictEqual({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true);
 expect(util.isDeepStrictEqual).toBe(isDeepStrictEqual);
 const s = Symbol('s');
 expect(isDeepStrictEqual({ [s]: 1, a: 2 }, { a: 2, [s]: 1 })).toBe(true);
 expect(isDeepStrictEqual(new Map([['a', 1], ['b', 2]]), new Map([['b', 2], ['a', 1]]))).toBe(true);
 expect(isDeepStrictEqual(new Set([1, { a: 1 }]), new Set([{ a: 1 }, 1]))).toBe(true);
});
test('isDeepStrictEqual keeps strict semantics JSON text would lose', () => {
 const pairs = [
  [{ a: 1 }, { a: 1, b: undefined }, false],
  [[1, 2], [2, 1], false],
  [NaN, NaN, true],
  [0, -0, false],
  [{ a: 0 }, { a: -0 }, false],
  [1, '1', false],
  [{ a: 1 }, { a: '1' }, false],
  [Object.create(null), {}, false],
  [[1, 2], { 0: 1, 1: 2 }, false],
  [new Date(5), new Date(5), true],
  [new Date(5), new Date(6), false],
  [new Number(1), new Number(2), false],
  [new Uint8Array([1, 2]), new Uint8Array([1, 2]), true],
  [new Uint8Array([1, 2]), new Int8Array([1, 2]), false],
  [/a/g, /a/g, true],
  [/a/g, /a/i, false],
  [{ [Symbol('x')]: 1 }, {}, false],
  [new Map([['a', 1]]), new Map([['a', 2]]), false],
  [undefined, null, false],
  [1n, 1n, true],
 ];
 expect(pairs.map(([a, b]) => isDeepStrictEqual(a, b))).toEqual(pairs.map((p) => p[2]));
});
test('isDeepStrictEqual compares cyclic structures', () => {
 const a = { n: 1 }; a.self = a;
 const b = { self: null, n: 1 }; b.self = b;
 expect(isDeepStrictEqual(a, b)).toBe(true);
 const c = { n: 2 }; c.self = c;
 expect(isDeepStrictEqual(a, c)).toBe(false);
});
"#;
#[test]
fn util_is_deep_strict_equal_matches_node() {
    let p = Project::new("util-deep-strict-equal-422");
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("util.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
