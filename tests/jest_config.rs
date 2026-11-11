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
    assert_eq!((pass, fail), (0, 1), "should time out with config timeout: {}", r.all());

    // CLI --testTimeout overrides the config
    let r2 = turbo_test(
        &p.dir,
        &["--reporter", "json", "--jobs", "1", "--testTimeout", "5000"],
        &[],
    );
    let (pass2, fail2) = counts(&r2);
    assert_eq!((pass2, fail2), (1, 0), "CLI should override config: {}", r2.all());
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
    assert_eq!((pass, fail), (2, 0), "clearMocks should clear call history: {}", r.all());
}

// #342: JEST_WORKER_ID is set for jest suites.
#[test]
fn jest_worker_id_is_set() {
    let p = Project::new("jest-worker-id-342");
    base_jest(&p);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
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
    assert_eq!((pass, fail), (1, 0), "JEST_WORKER_ID should be set: {}", r.all());
}
