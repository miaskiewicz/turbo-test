//! Node-owned AbortSignal.timeout regression batch (#325), shared with Node controls.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn abort_signal_timeout_uses_real_clock_and_validates_delay() {
    let p = Project::new("abort-timeout-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"abort-timeout-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "abort-timeout.test.cjs",
        include_str!("data/compat-084/abort-timeout.cjs"),
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&r);
    assert_eq!(report["numPassedTests"], 4, "{}", r.all());
    assert_eq!(report["numFailedTests"], 0, "{}", r.all());
}
