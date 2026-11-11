//! turbo-test#317: a dependency's published tsconfig with a missing `extends` target (a
//! dev-only dependency not installed by the consumer) must NOT break normal Node resolution
//! of already-compiled JS in that package. The resolver keys options by `nearest_tsconfig`,
//! so a dependency's broken tsconfig poisons the resolver for every module inside it.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn dep_tsconfig_with_missing_extends_does_not_break_require() {
    let p = Project::new("tsconfig-missing-extends");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "tsconfig-missing-extends" }"#);

    // A dependency that re-exports another dependency. Its tsconfig extends a dev-only
    // config package that the consumer hasn't installed.
    p.file(
        "node_modules/side-pkg/package.json",
        r#"{ "name": "side-pkg", "main": "index.js" }"#,
    );
    p.file(
        "node_modules/side-pkg/tsconfig.json",
        r#"{ "extends": "@missing-dev-tsconfig/base", "compilerOptions": {} }"#,
    );
    p.file(
        "node_modules/side-pkg/index.js",
        "module.exports = require('leaf-pkg/type');\n",
    );

    // The leaf dependency that side-pkg imports.
    p.file(
        "node_modules/leaf-pkg/package.json",
        r#"{ "name": "leaf-pkg", "main": "index.js" }"#,
    );
    p.file("node_modules/leaf-pkg/index.js", "exports.ok = true;\n");
    p.file(
        "node_modules/leaf-pkg/type.js",
        "module.exports = TypeError;\n",
    );

    // The test file: require both directly and through the dep with the broken tsconfig.
    p.file(
        "a.test.js",
        r#"
const { test, expect } = require('@jest/globals');
test('direct require works', () => {
    expect(require('leaf-pkg/type')).toBe(TypeError);
});
test('require through dep with broken tsconfig works', () => {
    expect(require('side-pkg')).toBe(TypeError);
});
"#,
    );
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );

    let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
    let j = json_report(&r);
    assert_eq!(
        (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
        (Some(2), Some(0)),
        "dep with missing tsconfig extends should not break resolution:\n{}",
        r.all()
    );
}
