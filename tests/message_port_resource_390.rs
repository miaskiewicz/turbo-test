#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn message_ports_do_not_invoke_public_async_resource_replacements() {
    let p = Project::new("port-internal-resource-390");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"port-internal-resource-390"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "resource.test.cjs",
        include_str!("data/compat-084/port-internal-resource.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let r = json_report(&run);
    assert_eq!(r["numPassedTests"], 1, "{}", run.all());
    assert_eq!(r["numFailedTests"], 0, "{}", run.all());
}
