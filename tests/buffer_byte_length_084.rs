#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn buffer_byte_length_matches_node_encoding_and_memory_contracts() {
    let p = Project::new("buffer-byte-length-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"buffer-byte-length-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "length.test.cjs",
        include_str!("data/compat-084/buffer-byte-length.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let r = json_report(&run);
    assert_eq!(r["numPassedTests"], 5, "{}", run.all());
    assert_eq!(r["numFailedTests"], 0, "{}", run.all());
}
