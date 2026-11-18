//! Eager, non-executing vm.Script constructor parsing and context reuse (#364).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn vm_script_compiles_at_construction_without_execution() {
    let p = Project::new("vm-script-compile-364");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"vm-script-compile-364"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file(
        "vm-script.test.cjs",
        include_str!("data/compat-084/vm-script-compile.cjs"),
    );
    let control = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        control.status.success(),
        "Node vm.Script control: {}{}",
        String::from_utf8_lossy(&control.stdout),
        String::from_utf8_lossy(&control.stderr)
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 6, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
