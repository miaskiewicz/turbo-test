//! Actual loopback raw TCP transport and Node controls (#363).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn tcp_bytes_lifecycle_and_backpressure_match_node() {
    let p = Project::new("tcp-363");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"tcp-363"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file("tcp.test.cjs", include_str!("data/compat-084/tcp.cjs"));
    let control = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        control.status.success(),
        "Node TCP control: {}{}",
        String::from_utf8_lossy(&control.stdout),
        String::from_utf8_lossy(&control.stderr)
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 13, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
