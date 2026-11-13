//! Batched stream lifecycle regressions (#319, #334), shared with real Node controls.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn streams_pipeline_finished_and_lifecycle_compatibility() {
    let p = Project::new("streams-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"streams-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "streams.test.cjs",
        include_str!("data/compat-084/streams.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 14, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
