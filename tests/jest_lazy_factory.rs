//! `jest.mock(spec, factory)` defers the factory to the module's first require, matching
//! jest 30.5.2 (turbo-test#275). `vi.mock` keeps vitest's eager hoist-time semantics.
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
fn jest_mock_factory_runs_on_first_require_not_at_hoist_time() {
    let p = Project::new("jest-lazy-275");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestlazy" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("m.js", "module.exports = 'real';\n");
    p.file(
        "probe.test.js",
        "const order = [];\n\
         jest.mock('./m', () => { order.push('factory'); return { x: 'x' } });\n\
         order.push('top');\n\
         const a = require('./m');\n\
         test('o', () => expect(order).toEqual(['top','factory']));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn jest_mock_factory_runs_exactly_once_across_multiple_requires() {
    let p = Project::new("jest-lazy-once-275");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestonce" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("m.js", "module.exports = { real: true };\n");
    p.file(
        "a.test.js",
        "let calls = 0;\n\
         jest.mock('./m', () => { calls++; return { v: calls } });\n\
         const a = require('./m');\n\
         const b = require('./m');\n\
         test('factory runs once', () => {\n\
             expect(calls).toBe(1);\n\
             expect(a).toBe(b);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn vi_mock_factory_still_runs_eagerly_at_hoist_time() {
    // vitest mode: `vi.mock` keeps its eager semantics — the factory runs before top-level
    // statements, not on first import (turbo-test#275 split).
    let p = Project::new("vi-eager-275");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "vieager" }"#);
    p.file("m.js", "module.exports = 'real';\n");
    p.file(
        "a.test.js",
        "import { vi } from 'vitest';\n\
         const order = [];\n\
         vi.mock('./m', () => { order.push('factory'); return { x: 'x' } });\n\
         order.push('top');\n\
         import('./m').then(() => {});\n\
         test('vi.mock factory ran first', () => expect(order).toEqual(['factory','top']));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}
