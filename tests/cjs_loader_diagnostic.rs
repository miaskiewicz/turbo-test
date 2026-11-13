//! Opt-in loader tracing must preserve execution and remain silent by default.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn loader_diagnostics_preserve_commonjs_execution() {
    let p = Project::new("loader-diagnostic");
    p.with_esbuild();
    p.file("package.json", "{\"name\":\"loader-diagnostic\"}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("dep.cjs", "module.exports = { value: 42 };\n");
    p.file("loader.test.cjs", "const value = require('./dep.cjs').value; test('real dependency', () => expect(value).toBe(42));\n");
    let normal = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_CJS_LOAD_DIAGNOSTIC", "0")],
    );
    let traced = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_CJS_LOAD_DIAGNOSTIC", "1")],
    );
    assert_eq!(
        json_report(&normal)["numPassedTests"],
        1,
        "{}",
        normal.all()
    );
    assert_eq!(
        json_report(&traced)["numPassedTests"],
        1,
        "{}",
        traced.all()
    );
    assert!(!normal.all().contains("[cjs-load]"));
    assert!(traced.all().contains("[cjs-load]"));
    assert!(traced.all().contains("dep.cjs"));
}
