//! Asynchronous `child_process.ref`/`unref`, `detached:true` spawn/fork and loop-liveness
//! semantics (turbo-test#446, for mongodb-memory-server's MongoInstance and mongo_killer).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn async_child_ref_unref_and_detached_spawn_and_fork_accept_options_and_do_not_throw() {
    let p = Project::new("async-process-ref-unref-446");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-process-ref-unref-446"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', testTimeout: 10000 };\n",
    );
    p.file(
        "killer.cjs",
        "setInterval(() => {}, 1000); process.on('message', () => {});\n",
    );
    p.file(
        "ref.test.cjs",
        r#"
const assert = require('node:assert/strict');
const path = require('node:path');
const { spawn, fork } = require('node:child_process');

test('spawn + ref/unref are chainable and preserve pid / spawnfile', () => {
  const child = spawn('/bin/sleep', ['30']);
  assert.equal(typeof child.pid, 'number');
  assert.equal(child.unref(), undefined);
  assert.equal(child.ref(), undefined);
  assert.equal(child.unref(), undefined);
  assert.equal(child.kill('SIGKILL'), true);
});

test('spawn accepts detached:true without throwing', async () => {
  const child = spawn('/bin/sleep', ['30'], { detached: true });
  assert.equal(typeof child.pid, 'number');
  assert.equal(child.kill('SIGKILL'), true);
  await new Promise((r) => child.once('exit', r));
});

test('fork accepts { detached:true, stdio:"ignore" } + unref (mongo_killer shape)', () => {
  const killer = fork(path.join(__dirname, 'killer.cjs'), [], {
    detached: true,
    stdio: 'ignore',
  });
  assert.equal(typeof killer.pid, 'number');
  assert.equal(killer.unref(), undefined);
  assert.equal(killer.kill('SIGKILL'), true);
});
"#,
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn detached_and_unref_child_survives_context_teardown_between_files() {
    let p = Project::new("async-process-ref-unref-survive-446");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"async-process-ref-unref-survive-446"}"#,
    );
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', testTimeout: 15000 };\n",
    );
    let marker = p.dir.join("survived.marker").display().to_string();
    p.file(
        "00-spawn.test.cjs",
        &format!(
            r#"
const {{ spawn }} = require('node:child_process');
const marker = {marker:?};
test('detached+unref child is retained past context drop', () => {{
  const script =
    "setTimeout(() => require('fs').writeFileSync(" +
    JSON.stringify(marker) + ", 'alive'), 300); " +
    "setInterval(() => {{}}, 1000);";
  const child = spawn(process.execPath, ['-e', script], {{
    detached: true,
    stdio: 'ignore',
  }});
  child.unref();
  globalThis.__detachedPid = child.pid;
}});
"#,
            marker = marker
        ),
    );
    p.file(
        "01-check.test.cjs",
        &format!(
            r#"
const assert = require('node:assert/strict');
const fs = require('node:fs');
const {{ spawnSync }} = require('node:child_process');
const marker = {marker:?};
test('the detached+unref grandchild wrote its marker after our context dropped', () => {{
  spawnSync('/bin/sleep', ['1']);
  assert.equal(fs.existsSync(marker), true,
    'detached+unref grandchild was killed when the parent context dropped');
  assert.equal(fs.readFileSync(marker, 'utf8'), 'alive');
  // reap the surviving process so the host does not leak it
  try {{ process.kill(globalThis.__detachedPid, 'SIGKILL'); }} catch (_) {{}}
}});
"#,
            marker = marker
        ),
    );
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json", "--jobs", "1", "--no-isolate"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
