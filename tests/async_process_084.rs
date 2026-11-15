//! Real asynchronous host processes and Node fork IPC (#371).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn asynchronous_spawn_and_real_node_fork_ipc() {
    let p = Project::new("async-process-371");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-process-371"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', testTimeout: 15000 };\n",
    );
    p.file(
        "async.test.cjs",
        include_str!("data/compat-084/async-processes.cjs"),
    );
    p.file(
        "async-fork-child.cjs",
        include_str!("data/compat-084/async-fork-child.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 8, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
