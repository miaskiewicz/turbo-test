//! An ES module Node runs (a `"type": "module"` package) importing a CommonJS module whose
//! `module.exports` is a primitive gets Node's namespace of it: `default` and `'module.exports'`
//! the value, `export *` passing `'module.exports'` on (turbo-test#193), on the module runner and
//! the ES-module graph loader. Expected: vitest 4.1.12 on the same project (Node 26 runs `pk`).
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

#[test]
fn a_primitive_module_exports_is_nodes_namespace_default() {
    for esbuild in [false, true] {
        let p = Project::new(if esbuild { "ncp-mr" } else { "ncp-graph" });
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{"name":"ncp","type":"module"}"#);
        p.file(
            "node_modules/pk/package.json",
            r#"{"name":"pk","type":"module","exports":"./index.js"}"#,
        );
        p.file(
            "node_modules/pk/index.js",
            "import v from './c.cjs';\nimport * as ns from './c.cjs';\nexport * from './s.cjs';\nexport { v, ns };\n",
        );
        p.file("node_modules/pk/c.cjs", "module.exports = 42;\n");
        p.file("node_modules/pk/s.cjs", "module.exports = 'str';\n");
        p.file(
            "a.test.js",
            "import { test, expect } from 'vitest';\nimport * as all from 'pk';\nimport { v, ns } from 'pk';\n\
             test('a', () => { expect([v, Object.keys(ns), ns.default, Object.keys(all)]).toEqual([42, ['default', 'module.exports'], 42, ['module.exports', 'ns', 'v']]); });\n",
        );
        let r = turbo_test(&p.dir, &[], &[]);
        assert!(
            r.stdout.contains("Tests  1 passed (1)"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
    }
}
