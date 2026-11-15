#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn encrypted_pkcs8_real_sat_keys_and_import_errors() {
    let p = Project::new("encrypted-pkcs8-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"encrypted-pkcs8-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "import.test.cjs",
        include_str!("data/encrypted-pkcs8/import.cjs"),
    );
    p.file(
        "matching.key.der.base64",
        include_str!("data/encrypted-pkcs8/matching.key.der.base64"),
    );
    p.file(
        "matching.pbes1.key.der.base64",
        include_str!("data/encrypted-pkcs8/matching.pbes1.key.der.base64"),
    );
    p.file(
        "password.json",
        include_str!("data/encrypted-pkcs8/password.json"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let r = json_report(&run);
    assert_eq!(r["numPassedTests"], 4, "{}", run.all());
    assert_eq!(r["numFailedTests"], 0, "{}", run.all());
}
