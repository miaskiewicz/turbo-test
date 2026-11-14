#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn real_blocklist_addresses_ranges_and_subnets() {
    let p = Project::new("blocklist-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"blocklist-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "blocklist.test.cjs",
        include_str!("data/quick-crypto/blocklist.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let r = json_report(&run);
    assert_eq!(r["numPassedTests"], 4, "{}", run.all());
    assert_eq!(r["numFailedTests"], 0, "{}", run.all());
}
