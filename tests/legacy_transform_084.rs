//! Legacy constructor inheritance used by existing Transform consumers (#362).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn legacy_transform_construction_preserves_real_stream_behavior() {
    let p = Project::new("legacy-transform-362");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"legacy-transform-362"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "transform.test.cjs",
        include_str!("data/compat-084/legacy-transform.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 4, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
