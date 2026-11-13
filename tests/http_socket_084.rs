//! Real HTTP socket identity, body-read state and terminal lifecycle.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn http_sockets_follow_the_native_connection_lifecycle() {
    let p = Project::new("http-socket-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"http-socket-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "socket.test.cjs",
        include_str!("data/compat-084/http-sockets.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 4, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
