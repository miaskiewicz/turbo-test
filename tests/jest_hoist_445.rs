//! Jest's `jest.mock` hoisting must preserve the file's source order of requires vs
//! declarations: moving a require below a mock-prefixed const silently runs a file that real
//! Jest 30 / ts-jest rejects with `Cannot access 'mockGet' before initialization`
//! (turbo-test#445).
#![cfg(unix)]

mod common;
use common::{file_status, json_report, turbo_test, Project};

fn project() -> Project {
    let p = Project::new("jest-hoist-445");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"jest-hoist-445","version":"0.0.0"}"#,
    );
    // A jest-governed project (jest.config.*) — turbo-test's `jest_project` detector reads it.
    p.file(
        "jest.config.cjs",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "decorating.ts",
        include_str!("data/jest-hoist-445/decorating.ts"),
    );
    p.file("x.ts", include_str!("data/jest-hoist-445/x.ts"));
    p.file(
        "tdz.test.ts",
        include_str!("data/jest-hoist-445/tdz.test.ts"),
    );
    p
}

#[test]
fn mock_hoisting_preserves_jest_source_order_so_the_file_fails_with_tdz() {
    let p = project();
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&r);
    let files = report["testResults"]
        .as_array()
        .unwrap_or_else(|| panic!("{}", r.all()));
    assert_eq!(files.len(), 1, "{}", r.all());
    assert_eq!(file_status(&files[0]), "error", "{}", r.all());
    let message = files[0]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("Cannot access 'mockGet' before initialization"),
        "load error should be the real Jest 30 TDZ, got:\n{message}\n---\n{}",
        r.all()
    );
}
