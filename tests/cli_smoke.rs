//! End-to-end smoke test of the CLI on a throwaway project: a passing and a failing file
//! produce the right per-file statuses, totals and exit code.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn passing_and_failing_files_report_and_exit_nonzero() {
    let p = Project::new("smoke");
    p.file(
        "ok.test.ts",
        "test('adds', () => { expect(1 + 1).toBe(2) })\n",
    );
    p.file(
        "bad.test.ts",
        "test('breaks', () => { expect(1).toBe(2) })\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    let j = json_report(&r);
    assert_eq!(j["numTotalTestSuites"], 2);
    assert_eq!(j["numPassedTests"], 1);
    assert_eq!(j["numFailedTests"], 1);
    assert_eq!(j["success"], false);
    let statuses: Vec<_> = j["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"]
                    .as_str()
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_string(),
                common::file_status(t),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("bad.test.ts".into(), "failed".into()),
            ("ok.test.ts".into(), "passed".into())
        ]
    );
}
