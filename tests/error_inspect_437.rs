//! util.inspect / util.format('%s') print renamed and stack-less errors as Node v26.11.0 does
//! (#437). The data file also passes under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn error_inspect_matches_node() {
    let p = Project::new("error-inspect-437");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"error-inspect-437"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "inspect.test.cjs",
        include_str!("data/compat-085/error-inspect.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 9, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
