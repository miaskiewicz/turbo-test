//! Request idle timers share unchanged fixtures with the real Node HTTP oracle.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
use std::process::Command;

#[test]
fn request_timeout_disable_replace_activity_and_completion() {
    let p = Project::new("http-timeout-lifecycle");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"http-timeout-lifecycle"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "timeout.test.cjs",
        include_str!("data/compat-084/http-timeout.cjs"),
    );
    p.file(
        "node-control.cjs",
        "globalThis.test=require('node:test').test;require('./timeout.test.cjs');",
    );
    let node = Command::new("node")
        .arg("--test")
        .arg(p.path("node-control.cjs"))
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "{}{}",
        String::from_utf8_lossy(&node.stdout),
        String::from_utf8_lossy(&node.stderr)
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
