//! Real Node controls and EventEmitter constructor-free inheritance (#376).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn constructor_free_event_emitters_match_node() {
    let p = Project::new("events-lazy-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"events-lazy-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file(
        "events.test.cjs",
        include_str!("data/compat-084/events-lazy.cjs"),
    );
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
