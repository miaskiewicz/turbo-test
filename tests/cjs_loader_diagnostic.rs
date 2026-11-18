//! Opt-in loader tracing must preserve execution and remain silent by default.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn loader_diagnostics_preserve_commonjs_execution() {
    let p = Project::new("loader-diagnostic");
    p.with_esbuild();
    p.file("package.json", "{\"name\":\"loader-diagnostic\"}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("dep.cjs", "module.exports = { value: 42 };\n");
    p.file("loader.test.cjs", "const value = require('./dep.cjs').value; test('real dependency', () => expect(value).toBe(42));\n");
    let normal = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_CJS_LOAD_DIAGNOSTIC", "0")],
    );
    let traced = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_CJS_LOAD_DIAGNOSTIC", "1")],
    );
    assert_eq!(
        json_report(&normal)["numPassedTests"],
        1,
        "{}",
        normal.all()
    );
    assert_eq!(
        json_report(&traced)["numPassedTests"],
        1,
        "{}",
        traced.all()
    );
    assert!(!normal.all().contains("[cjs-load]"));
    assert!(traced.all().contains("[cjs-load]"));
    assert!(traced.all().contains("dep.cjs"));
}

#[test]
fn loader_diagnostics_report_real_body_failures_and_stop_at_the_record_limit() {
    let p = Project::new("loader-diagnostic-limit");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"loader-diagnostic-limit"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("good.cjs", "module.exports = { value: 42 };");
    p.file("bad.cjs", "throw globalThis.__expectedLoaderError;");
    p.file(
        "limit.test.cjs",
        r#"
      const error = globalThis.__expectedLoaderError = new Error('real body failure');
      let caught;
      try { require('./bad.cjs'); } catch (value) { caught = value; }
      let result;
      for (let i = 0; i < 2200; i++) result = require('./good.cjs');
      test('diagnostics preserve cached exports and error identity', () => {
        expect(caught).toBe(error);
        expect(result.value).toBe(42);
        expect(result).toBe(require('./good.cjs'));
      });
    "#,
    );
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json"],
        &[("TURBO_CJS_LOAD_DIAGNOSTIC", "1")],
    );
    assert_eq!(json_report(&run)["numPassedTests"], 1, "{}", run.all());
    let records: Vec<_> = run
        .stderr
        .lines()
        .filter(|line| line.starts_with("[cjs-load]"))
        .collect();
    assert_eq!(records.len(), 4096);
    assert!(records
        .iter()
        .any(|line| line.contains("event=body-failed") && line.contains("bad.cjs")));
}
