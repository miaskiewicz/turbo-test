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

/// A bundled sub-module that throws is not cached (Node drops a failed module): every require of
/// it — from outside the package or by the package itself — runs it again and throws again,
/// never handing out its half-built exports.
#[test]
fn a_failed_bundled_sub_module_runs_again_on_the_next_require() {
    for esbuild in [false, true] {
        let p = Project::new(&format!("esm-lazy-throw-{esbuild}"));
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "name": "esm-lazy-throw" }"#);
        p.file(
            "jest.config.js",
            "module.exports = { testEnvironment: 'node' };\n",
        );
        p.file(
            "node_modules/throw-pkg/package.json",
            r#"{ "name": "throw-pkg", "main": "index.js" }"#,
        );
        p.file(
            "node_modules/throw-pkg/index.js",
            "exports.loadB = () => require('./b.js');\n",
        );
        p.file(
            "node_modules/throw-pkg/b.js",
            "globalThis.__bRuns = (globalThis.__bRuns || 0) + 1;\nexports.partial = 1;\nthrow new Error('b boom');\n",
        );
        p.file(
            "a.test.js",
            r#"const pkg = require('throw-pkg');
test('a throwing sub-module is never cached half-built', () => {
  expect(() => require('throw-pkg/b.js')).toThrow('b boom');
  expect(() => pkg.loadB()).toThrow('b boom');
  expect(() => require('throw-pkg/b.js')).toThrow('b boom');
  expect(globalThis.__bRuns).toBe(3);
});
"#,
        );
        let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
        let j = json_report(&r);
        assert_eq!(
            (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
            (Some(1), Some(0)),
            "esbuild={esbuild}:\n{}",
            r.all()
        );
    }
}

/// Identity holds in either order: a sub-path required before the package entry is the instance
/// the entry's bundle then re-exports (the bundle does not register a second one).
#[test]
fn a_sub_path_required_first_is_the_instance_the_entry_re_exports() {
    for esbuild in [true, false] {
        let p = Project::new(&format!("esm-singleton-order-{esbuild}"));
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
test('sub-path first', () => {
    const sub = require('identity-fixture/state');
    const root = require('identity-fixture');
    expect(root.state).toBe(sub.state);
});
"#,
        );
        let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
        let j = json_report(&r);
        assert_eq!(
            (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
            (Some(1), Some(0)),
            "esbuild={esbuild}:\n{}",
            r.all()
        );
    }
}

/// A bundle adopts only a REAL instance of its sub-path: under vitest a `vi.mock` of `pkg/sub`
/// reaches the test's import, never the package's own internal require (vitest leaves
/// node_modules to Node), and with `isolate: false` it does not leak into the next file.
#[test]
fn a_mocked_sub_path_never_replaces_the_packages_own_instance() {
    let p = Project::new("esm-bundle-mock");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "esm-bundle-mock" }"#);
    p.file(
        "vitest.config.js",
        "export default { test: { isolate: false, fileParallelism: false } };\n",
    );
    p.file(
        "node_modules/bpkg/package.json",
        r#"{ "name": "bpkg", "main": "index.js" }"#,
    );
    p.file(
        "node_modules/bpkg/index.js",
        "const sub = require('./sub.js');\nexports.v = () => sub.v;\n",
    );
    p.file("node_modules/bpkg/sub.js", "exports.v = 'real';\n");
    p.file(
        "a.test.js",
        r#"import { v as subV } from 'bpkg/sub.js';
import { v } from 'bpkg';
vi.mock('bpkg/sub.js', () => ({ v: 'mock', default: { v: 'mock' } }));
test('the test sees the mock, the package its own module', () => {
  expect(subV).toBe('mock');
  expect(v()).toBe('real');
});
"#,
    );
    p.file(
        "b.test.js",
        "import { v } from 'bpkg';\ntest('unmocked file', () => expect(v()).toBe('real'));\n",
    );
    let r = turbo_test(
        &p.dir,
        &[
            "--reporter",
            "json",
            "--jobs",
            "1",
            "a.test.js",
            "b.test.js",
        ],
        &[],
    );
    let j = json_report(&r);
    assert_eq!(
        (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
        (Some(2), Some(0)),
        "{}",
        r.all()
    );
}

/// A partial mock of a sub-path (`importOriginal` / `jest.requireActual` spread into the factory)
/// gets the package's OWN instance as the original: one instance for the package and the test.
#[test]
fn a_partial_mock_of_a_sub_path_wraps_the_packages_own_instance() {
    for jest in [false, true] {
        let p = Project::new(&format!("esm-partial-mock-{jest}"));
        p.with_esbuild();
        p.file("package.json", r#"{ "name": "esm-partial-mock" }"#);
        p.file(
            "node_modules/pk/package.json",
            r#"{"name":"pk","exports":{".":"./index.js","./state":"./state.js"}}"#,
        );
        p.file(
            "node_modules/pk/index.js",
            "const s = require('./state.js');\nexports.state = s.state;\nexports.bump = s.bump;\n",
        );
        p.file(
            "node_modules/pk/state.js",
            "exports.state = { n: 0 };\nexports.bump = () => { exports.state.n++; };\n",
        );
        let (config, test) = if jest {
            (
                (
                    "jest.config.js",
                    "module.exports = { testEnvironment: 'node' };\n",
                ),
                r#"jest.mock('pk/state', () => ({ ...jest.requireActual('pk/state'), extra: 1 }));
const { state: rootState, bump } = require('pk');
const sub = require('pk/state');
test('one instance', () => { bump(); expect({ same: rootState === sub.state, n: sub.state.n, extra: sub.extra }).toEqual({ same: true, n: 1, extra: 1 }); });
"#,
            )
        } else {
            (
                ("vitest.config.js", "export default { test: {} };\n"),
                r#"import { state as rootState, bump } from 'pk';
import * as sub from 'pk/state';
vi.mock('pk/state', async (orig) => ({ ...(await orig()), extra: 1 }));
test('one instance', () => { bump(); expect({ same: rootState === sub.state, n: sub.state.n, extra: sub.extra }).toEqual({ same: true, n: 1, extra: 1 }); });
"#,
            )
        };
        p.file(config.0, config.1);
        p.file("a.test.js", test);
        let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
        let j = json_report(&r);
        assert_eq!(
            (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
            (Some(1), Some(0)),
            "jest={jest}:\n{}",
            r.all()
        );
    }
}

/// One instance per file across a package's entry bundles, as in Node: a module one entry's
/// bundle has registered but not yet run (a lazy require) is the instance a second entry's bundle
/// uses — a provider and a consumer from different entries share their context module (MUI's
/// FormControl / FormLabel via FormControlContext).
#[test]
fn entry_bundles_share_a_module_registered_but_not_yet_run() {
    for esbuild in [true, false] {
        let p = Project::new(&format!("esm-share-lazy-{esbuild}"));
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "name": "esm-share-lazy" }"#);
        p.file("vitest.config.js", "export default { test: {} };\n");
        p.file(
            "node_modules/ctxpkg/package.json",
            r#"{"name":"ctxpkg","exports":{"./provider":"./provider.js","./consumer":"./consumer.js"}}"#,
        );
        p.file("node_modules/ctxpkg/ctx.js", "exports.value = 'unset';\n");
        p.file(
            "node_modules/ctxpkg/provider.js",
            "exports.provide = (v) => { require('./ctx.js').value = v; };\n",
        );
        p.file(
            "node_modules/ctxpkg/consumer.js",
            "require('./provider.js');\nexports.read = () => require('./ctx.js').value;\n",
        );
        p.file(
            "a.test.js",
            r#"import { provide } from 'ctxpkg/provider';
import { read } from 'ctxpkg/consumer';
test('provider and consumer share ctx.js', () => {
  provide('set');
  expect(read()).toBe('set');
});
"#,
        );
        let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
        let j = json_report(&r);
        assert_eq!(
            (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
            (Some(1), Some(0)),
            "esbuild={esbuild}:\n{}",
            r.all()
        );
    }
}
