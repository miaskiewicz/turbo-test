//! Web Crypto integer-array brand, quota and view semantics (#370).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn random_values_integer_array_contract() {
    let p = Project::new("webcrypto-random-values-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"webcrypto-random-values-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "random.test.cjs",
        include_str!("data/compat-084/webcrypto-random-values.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
