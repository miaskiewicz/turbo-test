//! What busboy / multer need (#418): Buffer's internal per-encoding *Slice / *Write methods
//! and Readable#unpipe, each fixture checked against real Node first.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn buffer_internal_slices_and_writes_match_node() {
    node_and_turbo(
        "buffer-internal-slices-418",
        include_str!("data/buffer-085/buffer-internal-slices.cjs"),
        5,
    );
}

#[test]
fn readable_unpipe_detaches_destinations_like_node() {
    node_and_turbo(
        "readable-unpipe-418",
        include_str!("data/buffer-085/readable-unpipe.cjs"),
        3,
    );
}

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
