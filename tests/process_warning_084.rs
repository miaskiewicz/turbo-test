//! Real asynchronous process warnings needed by Mongoose's unmodified Jest warning helper.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn process_warnings_have_real_events_metadata_and_stderr() {
    let p = Project::new("process-warning-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"process-warning-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "warnings.test.cjs",
        include_str!("data/compat-084/process-warnings.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 7, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    for text in [
        "TT_WARNING_STDERR_VISIBLE",
        "TT_WARNING_CODE",
        "TT_WARNING_DETAIL_VISIBLE",
    ] {
        assert!(
            run.stderr.contains(text),
            "warning missing from stderr: {text}\n{}",
            run.all()
        );
    }
}

#[test]
fn reused_files_do_not_accumulate_warning_listeners() {
    let p = Project::new("process-warning-reuse-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"process-warning-reuse-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("00-first.test.cjs", r#"
const assert = require('node:assert/strict');
globalThis.__priorWarningListenerCalled = false;
process.on('warning', () => { globalThis.__priorWarningListenerCalled = true; });
test('first file installs its own warning listener', () => { assert.equal(globalThis.__priorWarningListenerCalled, false); });
"#);
    p.file(
        "01-next.test.cjs",
        r#"
const assert = require('node:assert/strict');
test('next file warning does not call prior file listener', async () => {
  process.emitWarning('TT_WARNING_REUSE_VISIBLE');
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(globalThis.__priorWarningListenerCalled, false);
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
    assert!(
        run.stderr.contains("TT_WARNING_REUSE_VISIBLE"),
        "{}",
        run.all()
    );
}
