//! Cross-provider fairness and isolate-reuse ownership regressions from the final review.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn simultaneous_child_tcp_http_and_timers_progress() {
    let p = Project::new("io-fairness-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"io-fairness-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports={testEnvironment:'node'};\n",
    );
    let fixture = p.file(
        "fairness.test.cjs",
        include_str!("data/compat-084/io-fairness.cjs"),
    );
    let oracle = std::process::Command::new("node")
        .args(["--test"])
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        oracle.status.success(),
        "{}{}",
        String::from_utf8_lossy(&oracle.stdout),
        String::from_utf8_lossy(&oracle.stderr)
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
#[test]
fn pending_ports_release_their_receivers_on_file_reset() {
    let p = Project::new("port-reset-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"port-reset-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports={testEnvironment:'node'};\n",
    );
    p.file(
        "00-post.test.cjs",
        r#"
const assert = require('node:assert/strict');
test('leave an owned posted task pending when the file finishes', () => {
  const channel = new MessageChannel();
  globalThis.__oldPortCalled = false;
  globalThis.__oldPortWeak = new WeakRef(channel.port1);
  channel.port1.onmessage = () => { globalThis.__oldPortCalled = true; };
  channel.port2.postMessage({ payload: 'x'.repeat(1024 * 1024) });
  assert.equal(globalThis.__oldPortCalled, false);
});
"#,
    );
    p.file(
        "01-collect.test.cjs",
        r#"
const assert = require('node:assert/strict');
test('next file releases undeliverable task ownership without firing old listeners', async () => {
  assert.equal(typeof gc, 'function');
  for (let attempt = 0; attempt < 20; attempt++) {
    gc(); await new Promise(resolve => setImmediate(resolve));
  }
  assert.equal(globalThis.__oldPortWeak.deref(), undefined);
  assert.equal(globalThis.__oldPortCalled, false);
});
"#,
    );
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--no-isolate", "--reporter", "json"],
        &[("TURBO_V8_FLAGS", "--expose-gc")],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn write_only_tcp_server_connections_close_on_empty_eof() {
    let p = Project::new("tcp-empty-eof-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"tcp-empty-eof-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports={testEnvironment:'node'};\n",
    );
    let fixture = p.file(
        "eof.test.cjs",
        include_str!("data/compat-084/tcp-empty-eof.cjs"),
    );
    let oracle = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        oracle.status.success(),
        "{}{}",
        String::from_utf8_lossy(&oracle.stdout),
        String::from_utf8_lossy(&oracle.stderr)
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
