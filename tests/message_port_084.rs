//! Native MessagePort ownership, queued delivery and transfer regressions (#327).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn message_ports_transfer_ownership_and_preserve_queued_delivery() {
    let p = Project::new("message-port-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"message-port-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "ports.test.cjs",
        include_str!("data/compat-084/message-ports.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 9, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
