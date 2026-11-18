#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn zlib_standard_constants_match_node_values_and_descriptors() {
    let p = Project::new("zlib-constants-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"zlib-constants-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "constants.test.cjs",
        include_str!("data/compat-084/zlib-constants.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let r = json_report(&run);
    assert_eq!(r["numPassedTests"], 4, "{}", run.all());
    assert_eq!(r["numFailedTests"], 0, "{}", run.all());
}
