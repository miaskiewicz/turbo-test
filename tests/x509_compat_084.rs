//! Real X.509 certificate parsing and RSA KeyObject interoperability (#367).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn node(p: &Project, arguments: &[&str]) {
    let output = std::process::Command::new("node")
        .args(arguments)
        .current_dir(&p.dir)
        .output()
        .expect("Node is required for the independent certificate oracle");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn certificate_facts_and_real_public_keys_match_node() {
    let p = Project::new("x509-real-367");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"x509-real-367"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "certificates.json",
        include_str!("data/compat-084/x509/certificates.json"),
    );
    p.file(
        "write-oracle.cjs",
        include_str!("data/compat-084/x509/write-oracle.cjs"),
    );
    p.file(
        "controls.cjs",
        include_str!("data/compat-084/x509/controls.cjs"),
    );
    node(&p, &["write-oracle.cjs"]);
    node(&p, &["--test", "controls.cjs"]);
    p.file(
        "x509.test.cjs",
        include_str!("data/compat-084/x509/controls.cjs"),
    );
    p.file(
        "capability.test.cjs",
        include_str!("data/compat-084/x509/capability.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 7, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
