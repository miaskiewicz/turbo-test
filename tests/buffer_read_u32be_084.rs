//! Real big-endian Buffer reads required during offline MongoDB dependency loading (#373).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn buffer_read_u32be_values_views_and_errors() {
    let p = Project::new("buffer-read-u32be-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"buffer-read-u32be-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "buffer.test.cjs",
        include_str!("data/compat-084/buffer-read-u32be.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
