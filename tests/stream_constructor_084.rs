//! CJS module constructors used by superagent and other legacy stream consumers.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn stream_export_is_callable_and_supports_legacy_and_modern_subclasses() {
    let p = Project::new("stream-constructor-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"stream-constructor-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "stream.test.cjs",
        include_str!("data/compat-084/stream-constructor.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 6, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
