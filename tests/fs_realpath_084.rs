//! Real Node controls and filesystem bytes/errors for canonical paths and symlinks.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn realpath_sync_apis_match_node() {
    let p = Project::new("fs-realpath-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"fs-realpath-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file("realpath.test.cjs", include_str!("data/compat-084/fs-realpath.cjs"));
    let node = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .current_dir(&p.dir)
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "{}",
        String::from_utf8_lossy(&node.stderr)
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 4, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
