#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn real_hmac_and_pbkdf2_known_answers() {
    let p = Project::new("crypto-hashes-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"crypto-hashes-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "hashes.test.cjs",
        include_str!("data/quick-crypto/hashes.cjs"),
    );
    p.file(
        "work-limit.test.cjs",
        include_str!("data/quick-crypto/work-limit.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
