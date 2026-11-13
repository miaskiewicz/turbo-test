//! Supertest's HTTP path distinguishes TLS servers without opening a TLS connection.
#![cfg(unix)]
mod common;
use common::{json_report, run_exe, turbo_test, Project};

#[test]
fn node_control_preserves_tls_server_constructor_identity() {
    let p = Project::new("tls-server-node-control-084");
    p.file(
        "identity.cjs",
        include_str!("data/compat-084/tls-server-identity.cjs"),
    );
    let run = run_exe("node", &p.dir, &["--test", "identity.cjs"], &[]);
    assert_eq!(run.code, Some(0), "{}", run.all());
}

#[test]
fn tls_server_identity_is_available_while_transport_fails_explicitly() {
    let p = Project::new("tls-server-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"tls-server-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "identity.test.cjs",
        include_str!("data/compat-084/tls-server-identity.cjs"),
    );
    p.file(
        "capability.test.cjs",
        include_str!("data/compat-084/tls-server-capability.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
