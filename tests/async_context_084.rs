//! Real native-await and scheduled-resource AsyncLocalStorage regressions (#322).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn async_local_storage_native_await_and_resources() {
    let p = Project::new("async-context-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-context-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "async-context.test.cjs",
        include_str!("data/compat-084/async-context.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 10, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn reused_isolate_does_not_retain_the_prior_files_async_frame() {
    let p = Project::new("async-context-reuse-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-context-reuse-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "00-first.test.cjs",
        r#"
const assert = require('node:assert/strict');
const { AsyncLocalStorage } = require('node:async_hooks');
globalThis.__asyncContextReuseStorage = new AsyncLocalStorage();
globalThis.__asyncContextReuseStorage.enterWith('prior-file');
test('first file has its own execution context', () => {
  assert.equal(globalThis.__asyncContextReuseStorage.getStore(), 'prior-file');
});
"#,
    );
    p.file(
        "01-second.test.cjs",
        r#"
const assert = require('node:assert/strict');
test('second file cannot observe the prior file execution context', () => {
  assert.equal(typeof globalThis.__asyncContextReuseStorage.getStore, 'function');
  assert.equal(globalThis.__asyncContextReuseStorage.getStore(), undefined);
});
"#,
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
