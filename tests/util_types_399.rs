//! Real Node controls and filesystem bytes/errors for the real util/types builtin (#399).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn util_types_aliases_match_node() {
    let p = Project::new("util-types-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"util-types-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file("util-types.test.cjs", include_str!("data/compat-084/util-types.cjs"));
    let node = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .current_dir(&p.dir)
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "{}",
        format!("{}\n{}", String::from_utf8_lossy(&node.stdout), String::from_utf8_lossy(&node.stderr))
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
