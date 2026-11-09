//! `jest.mock(spec, factory, { virtual: true })`: the factory's exports are registered against
//! the raw spec (plus the caller's dir), so `require(spec)` returns them without the spec ever
//! being resolved against the filesystem (turbo-test#308). Jest-only; vi.mock keeps its usual
//! resolve-then-register semantics.
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

#[test]
fn jest_mock_virtual_registers_against_raw_spec() {
    // The spec `virtual-only-pkg` isn't a real file / package, but the virtual mock makes
    // `require('virtual-only-pkg')` return the factory's exports.
    let p = Project::new("jest-virtual-308-basic");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestvirt" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "virt.test.js",
        "jest.mock('virtual-only-pkg', () => ({ default: 42, named: 'hi' }), { virtual: true });\n\
         const m = require('virtual-only-pkg');\n\
         test('default + named reach us', () => {\n\
             expect(m.default).toBe(42);\n\
             expect(m.named).toBe('hi');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn jest_do_mock_virtual_registers_eagerly() {
    // jest.doMock's runtime form takes the same options object; the spec never needs to exist.
    let p = Project::new("jest-virtual-308-domock");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestvirtdm" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "virt.test.js",
        "test('doMock virtual', () => {\n\
             jest.doMock('nope', () => ({ v: 7 }), { virtual: true });\n\
             const m = require('nope');\n\
             expect(m.v).toBe(7);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn unresolvable_spec_without_virtual_still_errors() {
    // Regression guard: dropping the `{ virtual: true }` must NOT silently swallow the missing
    // module — jest.mock of an unresolvable spec without the flag must still error.
    let p = Project::new("jest-virtual-308-regression");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestvirtneg" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "virt.test.js",
        "jest.mock('definitely-not-real-pkg', () => ({ x: 1 }));\n\
         test('errors', () => {\n\
             let threw = false;\n\
             try { require('definitely-not-real-pkg'); } catch (e) { threw = true; }\n\
             expect(threw).toBe(true);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}
