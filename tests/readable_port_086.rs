//! Node 26 Readable state machine port (#436): byte-accurate read(n) with howMuchToRead,
//! 'readable' first-HWM-cross and end semantics, needReadable / emittedReadable latching,
//! _read throttling via state.reading and maybeReadMore, pause/resume, pipe backpressure and
//! destroy/end/close ordering. Expectations come from real Node v26.11.0; the data file also
//! runs under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn readable_state_machine_matches_node_26() {
    let p = Project::new("readable-port-086");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"readable-port-086"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "readable.test.cjs",
        include_str!("data/compat-086/readable-port.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 15, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
