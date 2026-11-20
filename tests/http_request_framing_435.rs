//! node:http ClientRequest body framing matches Node v26.11.0 (turbo-test#435): a raw TCP server
//! records each request's line, framing headers and body bytes. The data file also runs under
//! `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn client_requests_frame_their_bodies_like_node() {
    let p = Project::new("http-request-framing-435");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"http-request-framing-435"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "framing.test.cjs",
        include_str!("data/compat-085/http-request-framing.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 17, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
