//! Buffer float / BigInt64 / variable-length integer methods and Brotli streams, each fixture
//! checked against real Node first.
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
fn buffer_float_bigint_and_variable_integers_match_node() {
    node_and_turbo(
        "buffer-wide-ops-085",
        include_str!("data/buffer-085/buffer-wide-ops.cjs"),
        6,
    );
}
