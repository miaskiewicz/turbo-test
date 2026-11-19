//! Buffer.from over every input kind matches Node v26.11.0: ArrayBuffer views share memory and
//! honour byteOffset/length, other objects copy, unsupported values throw ERR_INVALID_ARG_TYPE.
//! The data file also passes under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn buffer_from_matches_node_for_every_input_kind() {
    let p = Project::new("buffer-from-085");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"buffer-from-085"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "from.test.cjs",
        include_str!("data/compat-085/buffer-from.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
