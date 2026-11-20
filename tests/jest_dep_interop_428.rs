//! A jest-transformed `"type":"module"` package that imports a CommonJS dependency whose
//! `module.exports` is marked `__esModule` (bundler-shaped CJS): `.default` must be the dep's
//! own `.default` property, as ts-jest / babel-jest's `_interopRequireDefault` returns.
//! turbo-test's Node ESM->CJS rule gives the whole `module.exports` as `.default` instead,
//! which is what real Jest 30 does NOT do for its CJS-transformed callers (turbo-test#428).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn project() -> Project {
    let p = Project::new("jest-dep-interop-428");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"jest-dep-interop-428","version":"0.0.0"}"#,
    );
    // A jest-governed project: `JEST_PROJECT=true` for every file under it.
    p.file(
        "jest.config.cjs",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "node_modules/pkg/package.json",
        include_str!("data/jest-dep-interop-428/pkg/package.json"),
    );
    p.file(
        "node_modules/pkg/index.js",
        include_str!("data/jest-dep-interop-428/pkg/index.js"),
    );
    p.file(
        "node_modules/dep/package.json",
        include_str!("data/jest-dep-interop-428/dep/package.json"),
    );
    p.file(
        "node_modules/dep/index.js",
        include_str!("data/jest-dep-interop-428/dep/index.js"),
    );
    p.file(
        "interop.test.ts",
        include_str!("data/jest-dep-interop-428/interop.test.ts"),
    );
    p
}

#[test]
fn jest_type_module_package_reads_esmodule_cjs_default() {
    let p = project();
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&r);
    assert_eq!(report["numPassedTests"], 1, "{}", r.all());
    assert_eq!(report["numFailedTests"], 0, "{}", r.all());
}
