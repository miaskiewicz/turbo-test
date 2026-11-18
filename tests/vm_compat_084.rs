//! Batched Node-mode pristine VM context regressions (#332).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn vm_new_context_bindings_isolation_errors_and_timeout() {
    let p = Project::new("vm-context-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"vm-context-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("vm.test.cjs", include_str!("data/compat-084/vm.cjs"));
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 8, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn vm_immutable_global_shadows_report_a_capability_error() {
    let p = Project::new("vm-immutable-limits-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"vm-immutable-limits-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "vm-limits.test.cjs",
        r#"
const assert = require('node:assert/strict');
const vm = require('node:vm');
test('unsupported immutable shadows fail explicitly rather than producing wrong bindings', () => {
  for (const sandbox of [{ Infinity: 7 }, { NaN: 7 }, { undefined: undefined }]) {
    assert.throws(() => vm.runInNewContext('1', sandbox), { code: 'ERR_VM_UNSUPPORTED_GLOBAL' });
  }
  assert.equal(vm.runInNewContext('6 * 7'), 42);
});
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
