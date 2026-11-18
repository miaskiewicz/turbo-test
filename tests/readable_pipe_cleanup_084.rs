#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn readable_pipe_releases_owned_listeners_at_terminal_boundaries() {
    let p = Project::new("readable-pipe-cleanup-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"readable-pipe-cleanup-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "cleanup.test.cjs",
        include_str!("data/compat-084/pipe-cleanup.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let r = json_report(&run);
    assert_eq!(r["numPassedTests"], 4, "{}", run.all());
    assert_eq!(r["numFailedTests"], 0, "{}", run.all());
}
