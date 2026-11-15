#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn buffer_static_descriptors_support_enumeration_consumers() {
    let p = Project::new("buffer-enumerability-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"buffer-enumerability-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "statics.test.cjs",
        include_str!("data/compat-084/buffer-enumerability.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
