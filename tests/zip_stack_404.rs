//! The archiver / zip-encrypted stack (#404): callable EventEmitter, streaming zlib classes
//! and archiver-style stream finalisation, each fixture checked against real Node first.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

/// Runs `source` under `node --test` (the oracle) and then under turbo-test, expecting
/// `passed` tests to pass in both.
fn node_and_turbo(name: &str, source: &str, passed: u64) {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file("fixture.test.cjs", source);
    let node = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .current_dir(&p.dir)
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
    assert_eq!(report["numPassedTests"], passed, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn event_emitter_is_callable_like_node() {
    node_and_turbo(
        "events-callable-404",
        include_str!("data/zip-404/events-callable.cjs"),
        3,
    );
}

#[test]
fn zlib_streams_compress_incrementally_like_node() {
    node_and_turbo(
        "zlib-streams-404",
        include_str!("data/zip-404/zlib-streams.cjs"),
        8,
    );
}
