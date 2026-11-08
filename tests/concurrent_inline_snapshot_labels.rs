//! turbo-test#277: concurrent tests with mismatching inline snapshots each get their own label.
//!
//! Before the fix, `__beginAttempt` wrote `__snap.testName` to a global and the `await null`
//! inside `__runAttemptInner` let a concurrent sibling overwrite it before this test's body
//! threw — so both error messages carried the second-scheduled test's name.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn concurrent_inline_snapshot_errors_use_their_own_label() {
    let p = Project::new("conc-inline-277");
    p.file(
        "s.test.js",
        "import { test, expect } from 'vitest';\n\
         test.concurrent('alpha', () => { expect('x').toMatchInlineSnapshot(`\"nope1\"`); });\n\
         test.concurrent('beta',  () => { expect('y').toMatchInlineSnapshot(`\"nope2\"`); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    let report = json_report(&r);
    let tests = report["testResults"][0]["assertionResults"]
        .as_array()
        .unwrap();
    assert_eq!(tests.len(), 2, "{}", r.all());

    let label = |name: &str| -> String {
        tests
            .iter()
            .find(|t| t["title"] == name)
            .unwrap_or_else(|| panic!("no {name} test in {}", r.all()))["failureMessages"][0]
            .as_str()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string()
    };

    assert_eq!(
        label("alpha"),
        "Error: Snapshot `alpha 1` mismatched",
        "{}",
        r.all()
    );
    assert_eq!(
        label("beta"),
        "Error: Snapshot `beta 1` mismatched",
        "{}",
        r.all()
    );
}
