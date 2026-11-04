//! Module resolution through the real CLI: vitest `resolve.alias` (TURBO_ALIAS), tsconfig
//! `paths`, package `exports` conditions (ESM-first for vitest, CJS-first for jest/node
//! projects), `?query` suffixes + the svgr `?react` stub, `node_modules/` specifiers, extension
//! probing, missing-module errors, the E12 memo switch, and the transform cache.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

/// (passed, failed, per-file statuses) from a `--reporter json` run.
fn counts(r: &common::Run) -> (u64, u64, Vec<String>) {
    let j = json_report(r);
    let statuses = j["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(common::file_status)
        .collect();
    (
        j["numPassedTests"].as_u64().unwrap(),
        j["numFailedTests"].as_u64().unwrap(),
        statuses,
    )
}

/// A minimal CommonJS `react` with just what the svgr stub uses.
fn fake_react(p: &Project) {
    p.file(
        "node_modules/react/package.json",
        r#"{ "name": "react", "main": "index.js" }"#,
    );
    p.file(
        "node_modules/react/index.js",
        "exports.forwardRef = (fn) => ({ fwd: true, render: fn });\nexports.createElement = (type, props) => ({ type, props });\n",
    );
}

/// A package whose `exports` resolve differently under the `import` and `require` conditions.
fn dual_pkg(p: &Project) {
    p.file(
        "node_modules/dual/package.json",
        r#"{ "name": "dual", "exports": { ".": { "import": "./esm.mjs", "require": "./cjs.js" } } }"#,
    );
    p.file("node_modules/dual/esm.mjs", "export const which = 'esm';\n");
    p.file("node_modules/dual/cjs.js", "exports.which = 'cjs';\n");
}

fn svgr_stubs(p: &Project) -> Vec<String> {
    std::fs::read_dir(p.path(".tt-cache"))
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().to_string_lossy().into_owned();
            n.starts_with("__turbo_svgr_stub_").then_some(n)
        })
        .collect()
}

#[test]
fn aliases_paths_conditions_queries_and_svgr_resolve() {
    let p = Project::new("ra-resolve");
    p.with_esbuild();
    fake_react(&p);
    dual_pkg(&p);
    p.file("package.json", r#"{ "name": "ra-resolve" }"#);
    p.file(
        "vitest.config.ts",
        &format!(
            "export default {{ resolve: {{ alias: {{ '@app': '{}' }} }}, test: {{}} }};\n",
            p.path("src").display()
        ),
    );
    p.file(
        "tsconfig.json",
        r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "~/*": ["src/*"] } } }"#,
    );
    p.file("src/lib.ts", "export const v = 'lib';\n");
    p.file("src/only.mts", "export const only = 'mts';\n");
    p.file("src/icon.svg", "<svg/>\n");
    p.file("node_modules/plain/package.json", r#"{ "name": "plain" }"#);
    p.file("node_modules/plain/lib/x.js", "exports.deep = 'deep';\n");
    p.file(
        "src/r.test.ts",
        r#"import { v } from '~/lib';
import { v as v2 } from '@app/lib';
import { which } from 'dual';
import { only } from './only';
import Icon, { ReactComponent } from './icon.svg?react';
import raw from './icon.svg?raw';
test('tsconfig paths + config alias', () => { expect(v).toBe('lib'); expect(v2).toBe('lib'); });
test('vitest projects resolve the import condition', () => expect(which).toBe('esm'));
test('.mts is found by extension probing', () => expect(only).toBe('mts'));
test('svgr stub renders an svg with props and ref', () => {
  expect(Icon).toBe(ReactComponent);
  expect(Icon.render({ title: 't', ref: 1, key: 2 }, null)).toEqual({ type: 'svg', props: { title: 't' } });
  expect(Icon.render(null, null)).toEqual({ type: 'svg', props: { 'aria-hidden': true } });
  const r = {};
  expect(Icon.render({}, r).props.ref).toBe(r);
});
test('a missing module names the specifier', () => {
  expect(() => require('./nope')).toThrow("Cannot find module './nope'");
});
test('node_modules/<pkg>/... specifiers', async () => {
  const m = await import('node_modules/plain/lib/x');
  expect(m.deep).toBe('deep');
});
test('?raw resolves the base file', () => expect(typeof raw).toBe('string'));
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (7, 0, vec!["passed".into()]), "{}", r.all());
    assert!(r.stderr.contains("cache 0% hit"), "{}", r.all());
    // the stub imports react by the concrete path this importer resolves
    let stubs = svgr_stubs(&p);
    assert_eq!(stubs.len(), 1, "{stubs:?}");
    let stub = std::fs::read_to_string(p.path(".tt-cache").join(&stubs[0])).unwrap();
    let react = p.path("node_modules/react/index.js");
    assert!(
        stub.contains(&format!("from '{}';", react.display())),
        "{stub}"
    );

    // Same project again with the E12 memos disabled: identical results, and every
    // transform now comes from the warm cache (the svgr stub file is reused, not rewritten).
    let r2 = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_NO_E12", "1")]);
    assert_eq!(counts(&r2), (7, 0, vec!["passed".into()]), "{}", r2.all());
    assert!(r2.stderr.contains("cache 100% hit"), "{}", r2.all());
    assert_eq!(svgr_stubs(&p), stubs);
}

#[test]
fn svgr_stub_falls_back_to_bare_react_when_unresolvable() {
    let p = Project::new("ra-svgr-bare");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "ra-svgr-bare" }"#);
    p.file("icon.svg", "<svg/>\n");
    p.file(
        "a.test.ts",
        "test('svgr needs react', () => { expect(() => require('./icon.svg?react')).toThrow(); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0, vec!["passed".into()]), "{}", r.all());
    let stubs = svgr_stubs(&p);
    assert_eq!(stubs.len(), 1, "{stubs:?}");
    let stub = std::fs::read_to_string(p.path(".tt-cache").join(&stubs[0])).unwrap();
    assert!(
        stub.contains("import { createElement, forwardRef } from 'react';"),
        "{stub}"
    );
}

#[test]
fn jest_node_projects_resolve_cjs_first_and_dom_projects_esm_first() {
    let p = Project::new("ra-jest-node");
    p.with_esbuild();
    dual_pkg(&p);
    p.file("package.json", r#"{ "name": "ra-jest-node" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { rootDir: 'src', testEnvironment: 'node', setupFiles: ['<rootDir>/../setup.js'] };\n",
    );
    p.file("setup.js", "globalThis.JEST_SETUP = 'ran';\n");
    p.file(
        "src/a.test.ts",
        "import { which } from 'dual';\ntest('require condition', () => expect(which).toBe('cjs'));\ntest('setupFiles ran', () => expect(globalThis.JEST_SETUP).toBe('ran'));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (2, 0, vec!["passed".into()]), "{}", r.all());
    // the uncached (TURBO_NO_E12) project-kind detection agrees
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_NO_E12", "1")]);
    assert_eq!(counts(&r), (2, 0, vec!["passed".into()]), "{}", r.all());

    let d = Project::new("ra-jest-dom");
    d.with_esbuild();
    dual_pkg(&d);
    d.file("package.json", r#"{ "name": "ra-jest-dom" }"#);
    d.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'jsdom' };\n",
    );
    d.file(
        "a.test.ts",
        "import { which } from 'dual';\ntest('import condition', () => expect(which).toBe('esm'));\n",
    );
    let r = turbo_test(&d.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0, vec!["passed".into()]), "{}", r.all());
}

#[test]
fn esm_projects_without_esbuild_load_through_the_graph_loader() {
    let p = Project::new("ra-esm-graph");
    p.file(
        "package.json",
        r#"{ "name": "ra-esm-graph", "type": "module" }"#,
    );
    p.file(
        "src/paths.ts",
        "export const d: string = __dirname;\nexport const f: string = __filename;\n",
    );
    p.file(
        "src/own.ts",
        "const __dirname = 'own';\nexport const own = __dirname;\n",
    );
    p.file("src/dep.ts", "export const dep = () => 'real';\n");
    p.file(
        "src/a.test.ts",
        r#"import { d, f } from './paths';
import { own } from './own';
import { dep } from './dep';
vi.mock('./dep', () => ({ dep: () => 'mocked' }));
test('__dirname/__filename are injected', () => {
  expect(d.endsWith('/src')).toBe(true);
  expect(f.endsWith('/src/paths.ts')).toBe(true);
});
test('a local __dirname binding is left alone', () => expect(own).toBe('own'));
test('entry mocks register without esbuild', () => expect(dep()).toBe('mocked'));
"#,
    );
    p.file("src/bad.test.ts", "const x: number = ;\n");
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        counts(&r),
        (3, 0, vec!["passed".into(), "error".into()]),
        "{}",
        r.all()
    );
    // the file that does not transform fails with vite's oxc report, nothing on stderr
    // (turbo-test#113)
    let j = json_report(&r);
    let bad = j["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"].as_str().unwrap().ends_with("src/bad.test.ts"))
        .unwrap();
    let message = bad["message"].as_str().unwrap();
    assert!(
        message.starts_with(
            "Transform failed with 1 error:\n\n\x1b[31m[PARSE_ERROR] \x1b[0mUnexpected token\n"
        ) && message.contains(" src/bad.test.ts:1:19 "),
        "{message:?}"
    );
    assert!(!r.stderr.contains("transform "), "{}", r.all());
}

/// pnpm's isolated layout (turbo-test#230): `node_modules/foo` links into the virtual store
/// `node_modules/.pnpm/foo@1.0.0/node_modules/foo`, whose dependencies are its siblings there
/// (themselves links to their own store entries), and an undeclared dependency is found in the
/// store's hoisted `node_modules/.pnpm/node_modules`. Node resolves a package's bare imports from
/// its real path, so it finds all three; vitest 4.1.12 passes this file. Both the ES module
/// graph and the module runner's CommonJS path (a file with `vi.mock`) resolve them the same way.
#[test]
fn bare_imports_resolve_through_the_pnpm_virtual_store() {
    let p = Project::new("ra-pnpm");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{ "name": "app", "dependencies": { "foo": "1.0.0", "bar": "1.0.0" } }"#,
    );
    let store = "node_modules/.pnpm";
    // foo: an ES module package importing two declared dependencies and one hoisted one
    p.file(
        &format!("{store}/foo@1.0.0/node_modules/foo/package.json"),
        r#"{ "name": "foo", "version": "1.0.0", "type": "module", "main": "index.js" }"#,
    );
    p.file(
        &format!("{store}/foo@1.0.0/node_modules/foo/index.js"),
        "import { __name } from 'tslib';\nimport { warning } from 'hey-listen';\nimport { h } from 'hoisted';\n\
         export const all = [__name, warning(), h, import.meta.dirname.includes('.pnpm')].join(',');\n",
    );
    // bar: a CommonJS package requiring the same three
    p.file(
        &format!("{store}/bar@1.0.0/node_modules/bar/package.json"),
        r#"{ "name": "bar", "version": "1.0.0", "main": "index.js" }"#,
    );
    p.file(
        &format!("{store}/bar@1.0.0/node_modules/bar/index.js"),
        "exports.all = [require('tslib').__name, require('hey-listen').warning(), require('hoisted').h, __dirname.includes('.pnpm')].join(',');\n",
    );
    p.file(
        &format!("{store}/tslib@2.8.1/node_modules/tslib/package.json"),
        r#"{ "name": "tslib", "version": "2.8.1", "main": "tslib.js" }"#,
    );
    p.file(
        &format!("{store}/tslib@2.8.1/node_modules/tslib/tslib.js"),
        "exports.__name = 'tslib';\n",
    );
    p.file(
        &format!("{store}/hey-listen@1.0.8/node_modules/hey-listen/package.json"),
        r#"{ "name": "hey-listen", "version": "1.0.8", "main": "dist/index.js" }"#,
    );
    p.file(
        &format!("{store}/hey-listen@1.0.8/node_modules/hey-listen/dist/index.js"),
        "exports.warning = () => 'hey';\n",
    );
    p.file(
        &format!("{store}/node_modules/hoisted/package.json"),
        r#"{ "name": "hoisted", "main": "i.js" }"#,
    );
    p.file(
        &format!("{store}/node_modules/hoisted/i.js"),
        "exports.h = 'hoisted';\n",
    );
    let link = |target: &str, at: &str| {
        std::os::unix::fs::symlink(target, p.path(at)).unwrap();
    };
    for pkg in ["foo", "bar"] {
        link(
            "../../tslib@2.8.1/node_modules/tslib",
            &format!("{store}/{pkg}@1.0.0/node_modules/tslib"),
        );
        link(
            "../../hey-listen@1.0.8/node_modules/hey-listen",
            &format!("{store}/{pkg}@1.0.0/node_modules/hey-listen"),
        );
        link(
            &format!(".pnpm/{pkg}@1.0.0/node_modules/{pkg}"),
            &format!("node_modules/{pkg}"),
        );
    }
    let body = "test('pnpm', () => { expect(all).toBe('tslib,hey,hoisted,true'); expect(bar).toBe('tslib,hey,hoisted,true'); });\n";
    p.file(
        "a.test.js",
        &format!("import {{ all }} from 'foo';\nimport {{ all as bar }} from 'bar';\n{body}"),
    );
    p.file(
        "b.test.ts",
        &format!("import {{ all }} from 'foo';\nimport {{ all as bar }} from 'bar';\nimport {{ n }} from './n';\nvi.mock('./n', () => ({{ n: 1 }}));\n{body}"),
    );
    p.file("n.ts", "export const n = 0;\n");
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(
        counts(&r),
        (2, 0, vec!["passed".to_string(); 2]),
        "{}",
        r.all()
    );
}
