//! Integration tests for the four jest compat fixes in 0.8.3:
//! - #337: restoreAllMocks undoes replaceProperty + replaceValue method
//! - #338: requireActual throws on missing module; requireMock generates a mock
//! - #339: runAllTicks flushes process.nextTick under modern fake timers
//! - #336: isolateModulesAsync preserves the outer module registry
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
    p.file("package.json", r#"{ "name": "jest083" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
}

// ---- #337: restoreAllMocks undoes replaceProperty ----

#[test]
fn restore_all_mocks_undoes_replace_property() {
    let p = Project::new("jest-337-restore-replace");
    base_jest(&p);
    p.file(
        "a.test.js",
        r#"
const obj = { greeting: 'hello' };
test('restoreAllMocks undoes jest.replaceProperty', () => {
    jest.replaceProperty(obj, 'greeting', 'hi');
    expect(obj.greeting).toBe('hi');
    jest.restoreAllMocks();
    expect(obj.greeting).toBe('hello');
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn replace_property_handle_has_replace_value() {
    let p = Project::new("jest-337-replace-value");
    base_jest(&p);
    p.file(
        "a.test.js",
        r#"
const obj = { x: 1 };
test('replaceProperty handle exposes replaceValue', () => {
    const handle = jest.replaceProperty(obj, 'x', 10);
    expect(obj.x).toBe(10);
    handle.replaceValue(20);
    expect(obj.x).toBe(20);
    handle.restore();
    expect(obj.x).toBe(1);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

// ---- #338: requireActual throws; requireMock generates a mock ----

#[test]
fn require_actual_throws_on_missing_module() {
    let p = Project::new("jest-338-require-actual");
    base_jest(&p);
    p.file(
        "a.test.js",
        r#"
test('requireActual throws on missing module', () => {
    expect(() => jest.requireActual('./no-such-module')).toThrow();
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn require_mock_generates_mock_for_unmocked_module() {
    let p = Project::new("jest-338-require-mock");
    base_jest(&p);
    p.file("helper.js", "module.exports = { add: (a, b) => a + b };\n");
    p.file(
        "a.test.js",
        r#"
test('requireMock generates a mock for an unmocked module', () => {
    const mocked = jest.requireMock('./helper');
    expect(typeof mocked.add).toBe('function');
    expect(mocked.add(1, 2)).toBe(undefined);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

// ---- #339: runAllTicks flushes nextTick under fake timers ----

#[test]
fn run_all_ticks_flushes_next_tick_under_fake_timers() {
    let p = Project::new("jest-339-run-all-ticks");
    base_jest(&p);
    p.file(
        "a.test.js",
        r#"
test('runAllTicks flushes process.nextTick', () => {
    jest.useFakeTimers();
    let called = 0;
    process.nextTick(() => { called++; });
    jest.runAllTicks();
    expect(called).toBe(1);
    jest.useRealTimers();
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

// ---- #336: isolateModulesAsync preserves the outer registry ----

#[test]
fn isolate_modules_async_preserves_outer_registry() {
    let p = Project::new("jest-336-isolate-async");
    base_jest(&p);
    p.file("counter.js", "let n = 0; module.exports = { inc: () => ++n, get: () => n };\n");
    p.file(
        "a.test.js",
        r#"
test('isolateModulesAsync preserves outer module registry', async () => {
    const outer = require('./counter');
    outer.inc();
    expect(outer.get()).toBe(1);

    await jest.isolateModulesAsync(async () => {
        const inner = require('./counter');
        // inner is a fresh module — counter starts at 0
        expect(inner.get()).toBe(0);
        inner.inc();
        expect(inner.get()).toBe(1);
    });

    // outer must still be the same instance, counter still at 1
    expect(outer.get()).toBe(1);
    const again = require('./counter');
    expect(again).toBe(outer);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn isolate_modules_async_restores_on_rejection() {
    let p = Project::new("jest-336-isolate-reject");
    base_jest(&p);
    p.file("counter.js", "let n = 0; module.exports = { inc: () => ++n, get: () => n };\n");
    p.file(
        "a.test.js",
        r#"
test('isolateModulesAsync restores outer registry on rejection', async () => {
    const outer = require('./counter');
    outer.inc();

    await expect(jest.isolateModulesAsync(async () => {
        throw new Error('boom');
    })).rejects.toThrow('boom');

    // outer registry is still intact
    expect(outer.get()).toBe(1);
    const again = require('./counter');
    expect(again).toBe(outer);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}
