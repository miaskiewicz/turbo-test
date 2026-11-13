//! Real child-process execution regressions (#321), shared with Node controls.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn child_process_sync_execution_output_errors_and_cleanup() {
    let p = Project::new("process-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"process-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "processes.test.cjs",
        include_str!("data/compat-084/processes.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 8, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
