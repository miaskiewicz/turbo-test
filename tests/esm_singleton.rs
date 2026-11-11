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
