//! Jest node projects load a CommonJS test file through the module runner's CJS path, with the
//! native oxc emitter when the project has no esbuild (turbo-test#316). That path must keep Node's
//! CommonJS and ES module semantics: wrapper parameters a module re-declares, imports named like
//! them, and ES modules by package `type` that have no `import` / `export`.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn jest_node(name: &str, esbuild: bool, pkg: &str) -> Project {
    let p = Project::new(&format!("{name}-{esbuild}"));
    if esbuild {
        p.with_esbuild();
    }
    p.file("package.json", pkg);
    p.file(
        "jest.config.cjs",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p
}

fn assert_passes(p: &Project, files: &[&str], tests: u64) {
    let mut args = vec!["--reporter", "json"];
    args.extend_from_slice(files);
    let r = turbo_test(&p.dir, &args, &[]);
    let j = json_report(&r);
    assert_eq!(
        (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
        (Some(tests), Some(0)),
        "{}",
        r.all()
    );
}

#[test]
fn modules_redeclaring_wrapper_params_keep_node_semantics() {
    for esbuild in [false, true] {
        let p = jest_node("ncjs-wrapper", esbuild, r#"{ "name": "ncjs" }"#);
        // the UMD idiom: `var exports` re-declares the parameter and keeps its value
        p.file("umd.js", "var exports = exports || {};\nexports.a = 1;\n");
        p.file(
            "dir.js",
            "var __dirname;\nmodule.exports = typeof __dirname;\n",
        );
        p.file("m6.mjs", "export default () => 'imported';\n");
        p.file(
            "imp.mjs",
            "import require from './m6.mjs';\nexport const r = require();\n",
        );
        p.file(
            "own.mjs",
            "export const require = (s) => 'own ' + s;\nclass exports {}\nfunction module() { return 'fn'; }\n\
             const _require = 'taken';\nexport { module, exports as ex, _require };\n",
        );
        p.file(
            "a.test.js",
            r#"var __filename;
test('var exports keeps the wrapper object', () => expect(require('./umd').a).toBe(1));
test('var __dirname / __filename keep their values', () => {
  expect(require('./dir')).toBe('string');
  expect(typeof __filename).toBe('string');
});
test('an import named require', () => expect(require('./imp.mjs').r).toBe('imported'));
test('exported bindings named like wrapper params are the module\'s own', () => {
  const m = require('./own.mjs');
  expect(m.require('x')).toBe('own x');
  expect(m.module()).toBe('fn');
  expect(typeof m.ex).toBe('function');
  expect(m._require).toBe('taken');
});
"#,
        );
        assert_passes(&p, &["a.test.js"], 4);
    }
}

/// Without esbuild the entry stays on the ES module graph. (Through esbuild's CJS transform an ES
/// module runs sloppy — esbuild emits no "use strict" — as it does on 0.8.2.)
#[test]
fn an_es_module_by_package_type_without_imports_stays_strict() {
    let p = jest_node(
        "ncjs-strict",
        false,
        r#"{ "name": "ncjs", "type": "module" }"#,
    );
    p.file(
        "s.test.js",
        r#"const self = this;
test('an ES module is strict', () => {
  expect(() => { undeclaredGlobal = 1; }).toThrow(ReferenceError);
  expect(self).toBe(undefined);
});
"#,
    );
    assert_passes(&p, &["s.test.js"], 1);
}
