//! Real Node controls and filesystem bytes/errors for copying files (#361).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn copy_file_apis_match_node() {
    let p = Project::new("fs-copy-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"fs-copy-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file("copy.test.cjs", include_str!("data/compat-084/fs-copy.cjs"));
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
    assert_eq!(report["numPassedTests"], 6, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
