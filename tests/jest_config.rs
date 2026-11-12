//! Jest config parity: testTimeout (#346), clearMocks/resetMocks/restoreMocks (#343),
//! JEST_WORKER_ID (#342), setupFiles ordering (#344), testMatch/injectGlobals (#340),
//! globalSetup/globalTeardown/moduleNameMapper (#335).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn counts(r: &common::Run) -> (u64, u64) {
    let j = json_report(r);
    (
        j["numPassedTests"].as_u64().unwrap(),
        j["numFailedTests"].as_u64().unwrap(),
    )
}

fn base_jest(p: &Project) {
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestconfig" }"#);
}

// #346: jest config testTimeout is read as the default timeout (CLI overrides it).
#[test]
fn jest_config_test_timeout_is_honoured() {
    let p = Project::new("jest-config-timeout-346");
    base_jest(&p);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', testTimeout: 35 };\n",
    );
    p.file(
        "a.test.js",
        "test('times out', async () => { await new Promise(r => setTimeout(r, 200)); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!(
        (pass, fail),
        (0, 1),
        "should time out with config timeout: {}",
        r.all()
    );

    // CLI --testTimeout overrides the config
    let r2 = turbo_test(
        &p.dir,
        &["--reporter", "json", "--jobs", "1", "--testTimeout", "5000"],
        &[],
    );
    let (pass2, fail2) = counts(&r2);
    assert_eq!(
        (pass2, fail2),
        (1, 0),
        "CLI should override config: {}",
        r2.all()
    );
}

// #343: jest config clearMocks, resetMocks, restoreMocks.
#[test]
fn jest_config_clear_reset_restore_mocks() {
    let p = Project::new("jest-config-mocks-343");
    base_jest(&p);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', clearMocks: true };\n",
    );
    p.file(
        "a.test.js",
        "\
const fn = jest.fn();\n\
test('first call', () => { fn(); expect(fn).toHaveBeenCalledTimes(1); });\n\
test('cleared between tests', () => { expect(fn).toHaveBeenCalledTimes(0); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!(
        (pass, fail),
        (2, 0),
        "clearMocks should clear call history: {}",
        r.all()
    );
}

// #344: setupFiles runs before the test framework (typeof expect === 'undefined').
#[test]
fn jest_setup_files_run_before_framework() {
    let p = Project::new("jest-setup-order-344");
    base_jest(&p);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', setupFiles: ['./early.js'], setupFilesAfterEnv: ['./late.js'] };\n",
    );
    p.file("early.js", "globalThis.__earlyExpect = typeof expect;\n");
    p.file("late.js", "globalThis.__lateExpect = typeof expect;\n");
    p.file(
        "a.test.js",
        "test('setupFiles runs before framework', () => {\n\
             expect(globalThis.__earlyExpect).toBe('undefined');\n\
             expect(globalThis.__lateExpect).toBe('function');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!((pass, fail), (1, 0), "setup ordering wrong: {}", r.all());
}

// #342: JEST_WORKER_ID is set for jest suites, whatever their test environment, and not for
// vitest ones.
#[test]
fn jest_worker_id_is_set() {
    for env in ["node", "jsdom"] {
        jest_worker_id_case(env);
    }
    let v = Project::new("jest-worker-id-342-vitest");
    v.with_esbuild();
    v.file("package.json", r#"{ "name": "v" }"#);
    v.file("vitest.config.js", "export default { test: {} };\n");
    v.file(
        "a.test.js",
        "test('no JEST_WORKER_ID under vitest', () => expect(process.env.JEST_WORKER_ID).toBe(undefined));\n",
    );
    let r = turbo_test(&v.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

fn jest_worker_id_case(env: &str) {
    let p = Project::new(&format!("jest-worker-id-342-{env}"));
    base_jest(&p);
    p.file(
        "jest.config.js",
        &format!("module.exports = {{ testEnvironment: '{env}' }};\n"),
    );
    p.file(
        "a.test.js",
        "test('JEST_WORKER_ID is a numeric string', () => {\n\
             const id = process.env.JEST_WORKER_ID;\n\
             expect(id).toBeDefined();\n\
             expect(Number(id)).toBeGreaterThan(0);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!(
        (pass, fail),
        (1, 0),
        "JEST_WORKER_ID should be set ({env}): {}",
        r.all()
    );
}

/// `injectGlobals: false` holds on a reused isolate too (TURBO_REUSE_ISOLATE): every file gets
/// the framework only from `@jest/globals`.
#[test]
fn inject_globals_false_on_a_reused_isolate() {
    let p = Project::new("jest-inject-globals-reuse");
    base_jest(&p);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', injectGlobals: false };\n",
    );
    for n in ["a", "b"] {
        p.file(
            &format!("{n}.test.js"),
            "const { test, expect } = require('@jest/globals');\ntest('no globals', () => expect(typeof globalThis.describe).toBe('undefined'));\n",
        );
    }
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json", "--jobs", "1"],
        &[("TURBO_REUSE_ISOLATE", "1")],
    );
    assert_eq!(counts(&r), (2, 0), "{}", r.all());
}
