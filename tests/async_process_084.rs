//! Real asynchronous host processes and Node fork IPC (#371).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn asynchronous_spawn_and_real_node_fork_ipc() {
    let p = Project::new("async-process-371");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-process-371"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', testTimeout: 15000 };\n",
    );
    p.file(
        "async.test.cjs",
        include_str!("data/compat-084/async-processes.cjs"),
    );
    p.file(
        "async-fork-child.cjs",
        include_str!("data/compat-084/async-fork-child.cjs"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 8, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn reused_context_terminates_and_reaps_its_prior_files_child() {
    let p = Project::new("async-process-reset-371");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-process-reset-371"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', testTimeout: 15000 };\n",
    );
    p.file("00-spawn.test.cjs", r#"
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
test('first file owns an actual live child', async () => {
  const child = spawn(process.execPath, ['-e', 'process.stdout.write("ready"); setInterval(() => {}, 1000)'], { env: {} });
  globalThis.__ownedAsyncPid = child.pid;
  await new Promise((resolve, reject) => { child.once('error', reject); child.stdout.once('data', resolve); });
  assert.equal(typeof globalThis.__ownedAsyncPid, 'number');
});
"#);
    p.file("01-check.test.cjs", r#"
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
test('next file cannot inherit its previous file child', () => {
  const source = 'try { process.kill(' + globalThis.__ownedAsyncPid + ', 0); process.exit(7); } catch (error) { process.exit(error.code === "ESRCH" ? 0 : 8); }';
  const probe = spawnSync(process.execPath, ['-e', source], { env: {} });
  assert.equal(probe.status, 0);
});
"#);
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json", "--jobs", "1", "--no-isolate"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
