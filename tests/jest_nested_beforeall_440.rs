//! Jest 30 fails every test under a failing `beforeAll` hook (its describe and any nested
//! describes), with the hook's error as the test failure; vitest skips them. In jest projects
//! turbo-test must match Jest (turbo-test#440); vitest projects must still skip.
//! Oracle: real Jest 30 `--json` on the same fixture.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const FIXTURE: &str = r#"
describe('outer', () => {
  describe('inner', () => {
    beforeAll(() => { throw new Error('boom nested'); });
    test('a', () => { expect(1).toBe(1); });
    test('b', () => { expect(1).toBe(2); });
  });
  test('sibling', () => { expect(1).toBe(1); });
});
test('top', () => { expect(1).toBe(1); });
"#;

fn outcomes(p: &Project) -> Vec<(String, String, String)> {
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let file = &report["testResults"][0];
    file["assertionResults"]
        .as_array()
        .unwrap_or_else(|| panic!("no tests: {}", run.all()))
        .iter()
        .map(|a| {
            (
                a["fullName"].as_str().unwrap().to_string(),
                a["status"].as_str().unwrap().to_string(),
                a["failureMessages"][0]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn nested_before_all_fail_fails_tests_in_jest_project() {
    let p = Project::new("jest-nested-beforeall-440");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file("nested.test.js", FIXTURE);
    let got = outcomes(&p);
    assert_eq!(got.len(), 4, "{got:?}");
    // the two tests under the failing nested beforeAll: failed with the hook's error
    assert_eq!(got[0].0, "outer inner a");
    assert_eq!(got[0].1, "failed", "{got:?}");
    assert!(got[0].2.contains("boom nested"), "{got:?}");
    assert_eq!(got[1].0, "outer inner b");
    assert_eq!(got[1].1, "failed", "{got:?}");
    assert!(got[1].2.contains("boom nested"), "{got:?}");
    // sibling (same outer describe, no failing hook): still runs and passes
    assert_eq!(got[2].0, "outer sibling");
    assert_eq!(got[2].1, "passed");
    // top-level: untouched
    assert_eq!(got[3].0, "top");
    assert_eq!(got[3].1, "passed");
}

#[test]
fn nested_before_all_fail_skips_tests_in_vitest_project() {
    let p = Project::new("vitest-nested-beforeall-440");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "nested.test.js",
        r#"
import { describe, beforeAll, test, expect, it } from 'vitest';
describe('outer', () => {
  describe('inner', () => {
    beforeAll(() => { throw new Error('boom nested'); });
    test('a', () => { expect(1).toBe(1); });
    test('b', () => { expect(1).toBe(2); });
  });
  test('sibling', () => { expect(1).toBe(1); });
});
test('top', () => { expect(1).toBe(1); });
"#,
    );
    let got = outcomes(&p);
    assert_eq!(got.len(), 4, "{got:?}");
    assert_eq!(got[0].0, "outer inner a");
    assert_eq!(got[0].1, "skipped", "{got:?}");
    assert_eq!(got[1].0, "outer inner b");
    assert_eq!(got[1].1, "skipped", "{got:?}");
    assert_eq!(got[2].0, "outer sibling");
    assert_eq!(got[2].1, "passed");
    assert_eq!(got[3].0, "top");
    assert_eq!(got[3].1, "passed");
}

#[test]
fn outer_before_all_fail_fails_everything_under_it_in_jest_project() {
    // the outer beforeAll fails: every test in outer and its descendants is failed with its error
    let p = Project::new("jest-outer-beforeall-440");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file(
        "outer.test.js",
        r#"
describe('outer', () => {
  beforeAll(() => { throw new Error('outer boom'); });
  describe('inner', () => {
    test('a', () => { expect(1).toBe(1); });
  });
  test('sibling', () => { expect(1).toBe(1); });
});
test('top', () => { expect(1).toBe(1); });
"#,
    );
    let got = outcomes(&p);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0].0, "outer inner a");
    assert_eq!(got[0].1, "failed", "{got:?}");
    assert!(got[0].2.contains("outer boom"), "{got:?}");
    assert_eq!(got[1].0, "outer sibling");
    assert_eq!(got[1].1, "failed", "{got:?}");
    assert!(got[1].2.contains("outer boom"), "{got:?}");
    assert_eq!(got[2].0, "top");
    assert_eq!(got[2].1, "passed");
}
