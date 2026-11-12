//! turbo-test#316: package re-exports and subpath imports must share module evaluation (identity).
//! When a package exports `"."` and `"./state"` and the root re-exports from `./state.js`, both
//! `require('pkg').state` and `require('pkg/state').state` must be the same object (`===`).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

/// Set up an ESM package with `exports: { ".": "./index.js", "./state": "./state.js" }` where
/// `index.js` re-exports from `./state.js`.
fn identity_fixture(p: &Project) {
    p.file(
        "node_modules/identity-fixture/package.json",
        r#"{"name":"identity-fixture","type":"module","exports":{".":"./index.js","./state":"./state.js"}}"#,
    );
    p.file(
        "node_modules/identity-fixture/index.js",
        "export { state } from './state.js';\n",
    );
    p.file(
        "node_modules/identity-fixture/state.js",
        "export const state = {};\n",
    );
}

#[test]
fn subpath_exports_share_module_identity() {
    for esbuild in [true, false] {
        let p = Project::new(&format!("esm-singleton-{esbuild}"));
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "name": "esm-singleton" }"#);
        identity_fixture(&p);
        p.file(
            "jest.config.js",
            "module.exports = { testEnvironment: 'node' };\n",
        );
        p.file(
            "a.test.js",
            r#"
const { test, expect } = require('@jest/globals');
test('shares state', () => {
    const root = require('identity-fixture');
    const sub  = require('identity-fixture/state');
    expect(root.state).toBe(sub.state);
});
"#,
        );
        let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
        let j = json_report(&r);
        assert_eq!(
            (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
            (Some(1), Some(0)),
            "esbuild={esbuild}: subpath exports must share identity:\n{}",
            r.all()
        );
    }
}

/// The bundle registers its sub-modules for identity without running them: a package's module
/// runs on its first require only (esbuild's lazy `__commonJS`, Node), so a lazily required file
/// that needs an uninstalled optional peer — or has side effects — costs nothing until used.
#[test]
fn bundled_sub_modules_run_only_when_required() {
    for (esbuild, config) in [
        (
            false,
            (
                "jest.config.js",
                "module.exports = { testEnvironment: 'node' };\n",
            ),
        ),
        (
            true,
            (
                "jest.config.js",
                "module.exports = { testEnvironment: 'node' };\n",
            ),
        ),
        (
            false,
            ("vitest.config.js", "export default { test: {} };\n"),
        ),
        (true, ("vitest.config.js", "export default { test: {} };\n")),
    ] {
        let p = Project::new(&format!("esm-lazy-{esbuild}-{}", config.0));
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "name": "esm-lazy" }"#);
        p.file(config.0, config.1);
        p.file(
            "node_modules/lazy-pkg/package.json",
            r#"{ "name": "lazy-pkg", "main": "index.js" }"#,
        );
        p.file(
            "node_modules/lazy-pkg/index.js",
            "exports.ok = 1;\nexports.heavy = () => require('./heavy.js');\n",
        );
        p.file(
            "node_modules/lazy-pkg/heavy.js",
            "globalThis.__heavyRan = (globalThis.__heavyRan || 0) + 1;\n\
             require('optional-peer-not-installed');\n",
        );
        // vitest test files are ES modules (no `require`); jest's are CommonJS
        let load = if config.0.starts_with("vitest") {
            "import pkg from 'lazy-pkg';"
        } else {
            "const pkg = require('lazy-pkg');"
        };
        p.file(
            "a.test.js",
            &(load.to_string()
                + r#"
test('an unused sub-module never runs', () => {
  expect(pkg.ok).toBe(1);
  expect(globalThis.__heavyRan).toBe(undefined);
  expect(() => pkg.heavy()).toThrow("Cannot find module 'optional-peer-not-installed'");
  expect(globalThis.__heavyRan).toBe(1);
});
"#),
        );
        let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
        let j = json_report(&r);
        assert_eq!(
            (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
            (Some(1), Some(0)),
            "esbuild={esbuild} {}:\n{}",
            config.0,
            r.all()
        );
    }
}
