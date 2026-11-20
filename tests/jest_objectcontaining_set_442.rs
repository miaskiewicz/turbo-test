//! Jest 30's `expect.objectContaining({ k: Set })` compares each key's value with the
//! user-added equality testers only (no iterableEquality). For Set values that means Sets with
//! no own enumerable keys are always equal — subset, superset or disjoint all pass — which is
//! what real Jest emits. turbo-test's objectContaining uses iterableEquality so disjoint Sets
//! fail (turbo-test#442). Match Jest in jest projects; vitest projects keep their stricter
//! comparison. Oracle: real Jest 30 `--json`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn statuses(p: &Project) -> Vec<(String, String)> {
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    report["testResults"][0]["assertionResults"]
        .as_array()
        .unwrap_or_else(|| panic!("no tests: {}", run.all()))
        .iter()
        .map(|a| {
            (
                a["fullName"].as_str().unwrap().to_string(),
                a["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

const CASES: &str = r#"
test('equal sets pass', () => {
  expect({ t: new Set(['a', 'b']) }).toEqual(expect.objectContaining({ t: new Set(['a', 'b']) }));
});
test('actual superset passes', () => {
  expect({ t: new Set(['a', 'b', 'c']) }).toEqual(expect.objectContaining({ t: new Set(['a', 'b']) }));
});
test('expected superset passes', () => {
  expect({ t: new Set(['a']) }).toEqual(expect.objectContaining({ t: new Set(['a', 'b']) }));
});
test('disjoint sets pass', () => {
  expect({ t: new Set(['x']) }).toEqual(expect.objectContaining({ t: new Set(['y']) }));
});
test('plain toEqual on Sets stays strict', () => {
  // the switch does not touch top-level Set equality: Jest keeps iterableEquality for toEqual
  expect(new Set(['a', 'b'])).toEqual(new Set(['a', 'b']));
});
"#;

#[test]
fn jest_project_objectcontaining_sets_match_jest() {
    let p = Project::new("jest-objectcontaining-set-442");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file("sets.test.js", CASES);
    let got = statuses(&p);
    assert_eq!(got.len(), 5, "{got:?}");
    for (name, st) in &got {
        assert_eq!(st, "passed", "{name:?} should pass: {got:?}");
    }
}

#[test]
fn vitest_project_objectcontaining_sets_keep_strict_vitest_behaviour() {
    // vitest keeps iterableEquality so disjoint / different-size Sets fail
    let p = Project::new("vitest-objectcontaining-set-442");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "sets.test.js",
        r#"
import { test, expect } from 'vitest';
test('disjoint sets fail', () => {
  expect({ t: new Set(['x']) }).toEqual(expect.objectContaining({ t: new Set(['y']) }));
});
test('equal sets pass', () => {
  expect({ t: new Set(['a']) }).toEqual(expect.objectContaining({ t: new Set(['a']) }));
});
"#,
    );
    let got = statuses(&p);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0], ("disjoint sets fail".into(), "failed".into()));
    assert_eq!(got[1], ("equal sets pass".into(), "passed".into()));
}
