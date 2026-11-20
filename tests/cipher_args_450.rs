//! crypto.createCipheriv / createDecipheriv validate their arguments as Node v26.11.0 does (#450).
//! The data file also passes under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn cipher_argument_validation_matches_node() {
    let p = Project::new("cipher-args-450");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"cipher-args-450"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "cipher.test.cjs",
        include_str!("data/compat-085/cipher-args.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 6, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
