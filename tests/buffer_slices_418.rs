//! Buffer's internal per-encoding *Slice / *Write methods (#418), checked against real Node
//! first.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn buffer_internal_slices_and_writes_match_node() {
    let p = Project::new("buffer-internal-slices-418");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"buffer-internal-slices-418"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file(
        "fixture.test.cjs",
        include_str!("data/buffer-085/buffer-internal-slices.cjs"),
    );
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
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
