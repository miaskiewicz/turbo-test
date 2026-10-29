//! Entry-file `vi.mock` / `jest.mock` hoisting through the real CLI: the module-runner prepass
//! (vi.hoisted replay, factories closing over shared module `let`s, mocks declared below the
//! tests, re-binding of mocked requires captured with `vi.mocked`), jest projects, and the
//! legacy bundled prepass (TURBO_NO_MR).
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

fn prepass_files(p: &Project) -> Vec<String> {
    std::fs::read_dir(p.path("node_modules/.turbo-test-prepass"))
        .map(|d| {
            d.map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn module_runner_hoists_entry_mocks_with_shared_state() {
    let p = Project::new("ma-hoist");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "hoist" }"#);
    p.file(
        "dep.ts",
        "export const dep = () => 'real';\nexport const other = () => 'other';\n",
    );
    p.file("late.ts", "export const late = () => 'real-late';\n");
    p.file(
        "consumer.ts",
        "import { dep } from './dep';\nexport const viaConsumer = () => dep();\n",
    );
    p.file(
        "a.test.ts",
        r#"import { vi } from 'vitest';
import { dep, other } from './dep';
import { viaConsumer } from './consumer';
import { late } from './late';
const { spy } = vi.hoisted(() => ({ spy: vi.fn(() => 'hoisted') }));
let capturedValue;
let hookValue = 'initial';
const capturedCallsList = [];
vi.mock('./dep', () => {
  capturedValue = 'captured';
  capturedCallsList.push('factory');
  return { dep: spy, other: () => hookValue };
});
const mockedDep = vi.mocked(dep);
test('vi.hoisted value is the mock', () => {
  expect(dep()).toBe('hoisted');
  expect(spy).toHaveBeenCalled();
  expect(mockedDep).toBe(spy);
});
test('consumers see the mock', () => expect(viaConsumer()).toBe('hoisted'));
test('factory writes reach the test', () => {
  expect(capturedValue).toBe('captured');
  expect(capturedCallsList).toContain('factory');
});
test('test writes reach the factory', () => {
  hookValue = 'changed';
  expect(other()).toBe('changed');
});
test('a mock declared after the tests still intercepts', () => expect(late()).toBe('mocked-late'));
vi.mock('./late', () => ({ late: () => 'mocked-late' }));
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (5, 0), "{}", r.all());
    // the prepass replays vi.hoisted and routes the shared lets through globalThis.__ms
    let pre = prepass_files(&p);
    assert_eq!(pre.len(), 1, "{pre:?}");
    let pre = &pre[0];
    assert!(
        pre.starts_with("globalThis.__ms = globalThis.__ms || {};\n"),
        "{pre}"
    );
    assert!(
        pre.contains("const { spy } = vi.hoisted(() => ({ spy: vi.fn(() => 'hoisted') }));"),
        "{pre}"
    );
    assert!(
        pre.contains("globalThis.__ms.capturedValue = 'captured';"),
        "{pre}"
    );
    assert!(
        pre.contains("globalThis.__ms.capturedCallsList.push('factory');"),
        "{pre}"
    );
    assert!(pre.contains("() => globalThis.__ms.hookValue"), "{pre}");
}

#[test]
fn jest_mock_in_a_jest_project() {
    let p = Project::new("ma-jest");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestmock" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("svc.js", "exports.get = () => 'real';\n");
    p.file(
        "a.test.js",
        "const svc = require('./svc');\njest.mock('./svc', () => ({ get: () => 'jest-mocked' }));\ntest('jest.mock', () => expect(svc.get()).toBe('jest-mocked'));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn legacy_prepass_bundles_and_runs_entry_mock_factories() {
    let p = Project::new("ma-legacy");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "legacy-prepass" }"#);
    p.file(
        "node_modules/react/package.json",
        r#"{ "name": "react", "main": "index.js" }"#,
    );
    p.file(
        "node_modules/react/index.js",
        "exports.createElement = () => null;\n",
    );
    p.file("dep.ts", "export const dep = () => 'real';\n");
    p.file(
        "a.test.ts",
        "import { dep } from './dep';\nvi.mock('./dep', () => { globalThis.FACTORY_RAN = (globalThis.FACTORY_RAN || 0) + 1; return { dep: () => 'mocked' }; });\ntest('the prepass factory ran and its mock intercepts', () => { expect(globalThis.FACTORY_RAN).toBeGreaterThan(0); expect(dep()).toBe('mocked'); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_NO_MR", "1")]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
    let pre = prepass_files(&p);
    assert_eq!(pre.len(), 1, "{pre:?}");
    assert!(
        pre[0].starts_with("import { vi } from 'vitest';\nimport React from 'react';\n"),
        "{}",
        pre[0]
    );
}
