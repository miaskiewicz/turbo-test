//! Actual V8 Promise/resource lifecycle observation for cls-hooked prerequisites (#372).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn async_hooks_observe_promises_resources_and_disable_lifecycle() {
    let p = Project::new("async-hooks-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-hooks-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "hooks.test.cjs",
        include_str!("data/compat-084/async-hooks.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 10, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
