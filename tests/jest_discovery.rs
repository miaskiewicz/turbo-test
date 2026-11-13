//! Jest config testMatch and injectGlobals: false (#340).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn counts(r: &common::Run) -> (u64, u64) {
    let j = json_report(r);
    (
        j["numPassedTests"].as_u64().unwrap_or(0),
        j["numFailedTests"].as_u64().unwrap_or(0),
    )
}

fn base_jest(p: &Project) {
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestdiscovery" }"#);
}

/// #340: testMatch narrows which files are executed — a file not matching testMatch is skipped.
#[test]
fn jest_test_match_filters_files() {
    let p = Project::new("jest-testmatch-340");
    base_jest(&p);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', testMatch: ['**/*.spec.js'] };\n",
    );
    p.file(
        "included.spec.js",
        "test('included', () => { expect(1).toBe(1); });\n",
    );
    p.file(
        "excluded.test.js",
        "test('should not run', () => { expect(1).toBe(2); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!(
        (pass, fail),
        (1, 0),
        "testMatch should only run .spec.js files: {}",
        r.all()
    );
}

/// #340: injectGlobals: false hides test/expect/describe from the global scope;
/// they are only available through `require('@jest/globals')`.
#[test]
fn jest_inject_globals_false() {
    let p = Project::new("jest-noinject-340");
    base_jest(&p);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', injectGlobals: false };\n",
    );
    p.file(
        "a.test.js",
        r#"
const { test, expect } = require('@jest/globals');
test('globals are not injected', () => {
    expect(typeof globalThis.expect).toBe('undefined');
    expect(typeof globalThis.test).toBe('undefined');
    expect(typeof globalThis.describe).toBe('undefined');
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!(
        (pass, fail),
        (1, 0),
        "injectGlobals: false should hide globals: {}",
        r.all()
    );
}
