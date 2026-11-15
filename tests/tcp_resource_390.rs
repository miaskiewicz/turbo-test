//! Internal TCP resources are independent of mutable public module exports (#390).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn tcp_resources_ignore_public_async_resource_replacements() {
    let p = Project::new("tcp-internal-resource-390");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"tcp-internal-resource-390"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    let fixture = p.file(
        "resource.test.cjs",
        include_str!("data/compat-084/tcp-internal-resource.cjs"),
    );
    let control = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        control.status.success(),
        "Node control: {}{}",
        String::from_utf8_lossy(&control.stdout),
        String::from_utf8_lossy(&control.stderr)
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
