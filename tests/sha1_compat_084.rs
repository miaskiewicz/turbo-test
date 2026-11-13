//! SHA-256 Node and Web Crypto known-answer and lifecycle controls (#323, #328).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn sha1_known_answers_and_lifecycle() {
    let p = Project::new("sha1-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"sha1-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("sha1.test.cjs", include_str!("data/compat-084/sha1.cjs"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&r);
    assert_eq!(report["numPassedTests"], 4, "{}", r.all());
    assert_eq!(report["numFailedTests"], 0, "{}", r.all());
}
