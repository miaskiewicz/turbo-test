//! Owned Node shim regressions for #330, #331, #333 and offline data URL fetch #326.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn node_shims_and_offline_fetch_compatibility() {
    let p = Project::new("shims-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"shims-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("shims.test.cjs", include_str!("data/compat-084/shims.cjs"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&r);
    assert_eq!(report["numPassedTests"], 7, "{}", r.all());
    assert_eq!(report["numFailedTests"], 0, "{}", r.all());
}
