#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn backend_dependency_stack_filenames_survive_bytecode_cache_reuse() {
    let project = Project::new("cjs-node-origins");
    project.with_esbuild();
    project.file("package.json", r#"{"name":"cjs-node-origins"}"#);
    project.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    project.file(
        "node_modules/caller-discovery/package.json",
        r#"{"name":"caller-discovery","main":"index.cjs"}"#,
    );
    project.file(
        "node_modules/caller-discovery/index.cjs",
        include_str!("data/cjs-node-origin/discovery.cjs"),
    );
    project.file(
        "node_modules/native-caller/package.json",
        r#"{"name":"native-caller","main":"index.cjs"}"#,
    );
    project.file(
        "node_modules/native-caller/index.cjs",
        include_str!("data/cjs-node-origin/consumer.cjs"),
    );
    project.file(
        "origin.spec.cjs",
        include_str!("data/cjs-node-origin/origin.spec.cjs"),
    );
    let oracle = std::process::Command::new("node")
        .args(["--test", "origin.spec.cjs"])
        .current_dir(&project.dir)
        .output()
        .unwrap();
    assert!(
        oracle.status.success(),
        "{}",
        String::from_utf8_lossy(&oracle.stderr)
    );
    for _ in 0..2 {
        let run = turbo_test(&project.dir, &["--jobs", "1", "--reporter", "json"], &[]);
        let report = json_report(&run);
        assert_eq!(report["numPassedTests"], 1, "{}", run.all());
        assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    }
}
