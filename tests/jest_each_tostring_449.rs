//! `.each %s` of an object with its own `toString` (turbo-test#449): jest-each calls Node
//! `util.format` which passes the object through `String(value)`, so the title reads as the
//! toString result (e.g. `-1`). Before the fix turbo-test rendered `util.inspect(a, { depth: 0 })`
//! for such a value (e.g. `Money { currencyData: 'USD', safeAmount: -1 }`).
//!
//! Oracle: real Node 26 `util.format` matches real Jest 30's `jest-each` in-tree behaviour.
//! Vitest 5's `%s` already honours a custom `toString`, so a vitest project stays as it was.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

// A small Money-like value object whose `toString` returns a plain number; a plain object stays
// inspected one level deep (Node `util.format` keeps that for objects without a custom
// toString).
const SRC: &str = r#"
class Money {
  constructor(amount) { this.currencyData = 'USD'; this.safeAmount = amount; }
  toString() { return String(this.safeAmount); }
}
it.each([[new Money(-1)], [new Money(42)], [{ a: 1 }]])('money %s', () => { throw new Error('ARGS'); });
"#;

fn jest_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p
}

fn vitest_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "vitest.config.mjs",
        "export default { test: { globals: true, environment: 'node' } };",
    );
    p
}

fn titles(p: &Project) -> Vec<String> {
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let tests = report["testResults"][0]["assertionResults"]
        .as_array()
        .unwrap();
    tests
        .iter()
        .map(|a| a["title"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn jest_project_each_percent_s_uses_string_for_own_tostring() {
    let p = jest_project("jest-each-tostring-449");
    p.file("each.test.js", SRC);
    assert_eq!(
        titles(&p),
        vec![
            // own toString -> String(value) (jest-each via Node util.format %s)
            "money -1",
            "money 42",
            // plain object -> util.inspect(a, { depth: 0 })
            "money { a: 1 }",
        ],
    );
}

#[test]
fn vitest_project_each_percent_s_keeps_own_tostring() {
    let p = vitest_project("vitest-each-tostring-449");
    p.file("each.test.js", SRC);
    assert_eq!(
        titles(&p),
        vec![
            // vitest 5's own %s rule also honours a custom toString
            "money -1",
            "money 42",
            // vitest 5 inspect of a plain object at depth 10 fits within truncate 40
            "money { a: 1 }",
        ],
    );
}
