//! Config and mock semantics that follow jest/vitest: a JSON jest config (package.json
//! `"jest"` block, `jest.config.json`) is read like `jest.config.js`; a factory-less `vi.mock`
//! stays factory-less whatever code follows it; entry-file mocks intercept on the legacy
//! TURBO_NO_MR bundle path too; a bare `vi.hoisted` after a declared one is its own statement;
//! re-mocking under isolate reuse keeps an already-mocked importer, and a fresh isolate under the
//! reuse switch registers a doMock as a new module; `await import()` of a `TURBO_LAZY_STUBS`
//! barrel settles to its stub namespace.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

/// Assert a clean pass with exactly `n` passing tests.
fn assert_passes(r: &common::Run, n: u32) {
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains(&format!("Tests  {n} passed ({n})")),
        "{}",
        r.all()
    );
}

const SETUP_TEST: &str = "test('setup ran', () => { expect(globalThis.FROM_SETUP).toBe(1); });\n";

#[test]
fn json_jest_configs_are_read() {
    // package.json "jest" block (turbo-test#31 item 1 repro)
    let p = Project::new("rc-jest-pkg");
    p.file(
        "package.json",
        r#"{"name":"r1","jest":{"setupFiles":["<rootDir>/setup.js"]}}"#,
    );
    p.file("setup.js", "globalThis.FROM_SETUP = 1;\n");
    p.file("a.test.js", SETUP_TEST);
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);

    // jest.config.json, with a JSON rootDir and setupFilesAfterEnv
    let q = Project::new("rc-jest-json");
    q.file("package.json", r#"{"name":"r1b"}"#);
    q.file(
        "jest.config.json",
        "{\n  \"rootDir\": \"src\",\n  \"setupFilesAfterEnv\": [\"<rootDir>/setup.js\"]\n}\n",
    );
    q.file("src/setup.js", "globalThis.FROM_SETUP = 1;\n");
    q.file("src/a.test.js", SETUP_TEST);
    assert_passes(&turbo_test(&q.dir, &["src/a.test.js"], &[]), 1);

    // the jest block's keys only: a "setupFiles" key elsewhere in package.json is not jest's
    let r = Project::new("rc-jest-scoped");
    r.file(
        "package.json",
        r#"{"name":"r1c","other":{"setupFiles":["./wrong.js"]},"jest":{"testEnvironment":"node"}}"#,
    );
    r.file("wrong.js", "globalThis.FROM_SETUP = 'wrong';\n");
    r.file(
        "a.test.js",
        "test('no setup', () => { expect(globalThis.FROM_SETUP).toBe(undefined); });\n",
    );
    assert_passes(&turbo_test(&r.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn a_factory_less_mock_followed_by_more_code_stays_factory_less() {
    // turbo-test#31 item 4 end to end: `vi.mock('./a')` took the next call's factory (and that
    // mock was dropped), and in turbo-test#31 item 5's exact one-line repro the test callback
    // after a factory-less vi.mock became its "factory".
    let p = Project::new("rc-mock-parse");
    p.file("package.json", r#"{"name":"r4","type":"module"}"#);
    p.file("m.js", "export function f() { return 1 }\n");
    p.file("a.js", "export function a() { return 'real-a' }\n");
    p.file("b.js", "export const v = 'real-b';\n");
    p.file(
        "one.test.js",
        "import { f } from './m.js'; vi.mock('./m.js'); test('automock', () => { expect(vi.isMockFunction(f)).toBe(true) });\n",
    );
    p.file(
        "two.test.js",
        r#"import { a } from './a.js';
import { v } from './b.js';
vi.mock('./a.js');
vi.mock('./b.js', () => ({ v: 'mock-b' }));
test('both mocks', () => {
  expect(vi.isMockFunction(a)).toBe(true);
  expect(a()).toBe(undefined);
  expect(v).toBe('mock-b');
});
"#,
    );
    let r = turbo_test(&p.dir, &["one.test.js", "two.test.js"], &[]);
    assert_passes(&r, 2);
    p.with_esbuild();
    let r = turbo_test(&p.dir, &["one.test.js", "two.test.js"], &[]);
    assert_passes(&r, 2);
}

#[test]
fn entry_mocks_intercept_on_the_legacy_bundle_path() {
    // turbo-test#31 item 15: under TURBO_NO_MR the entry is esbuild-bundled and its mocked
    // modules were bundled into it, so entry vi.mock / jest.mock never intercepted (relative
    // modules and packages alike). They are now externalized like setup mocks.
    let p = Project::new("rc-nomr-entry-mocks");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"r15"}"#);
    p.file(
        "node_modules/react/package.json",
        r#"{"name":"react","main":"index.js"}"#,
    );
    p.file(
        "node_modules/react/index.js",
        "exports.createElement = () => null;\n",
    );
    p.file(
        "node_modules/pkg/package.json",
        r#"{"name":"pkg","main":"index.js"}"#,
    );
    p.file(
        "node_modules/pkg/index.js",
        "exports.p = () => 'real-pkg';\n",
    );
    p.file("src/rel.js", "export const r = () => 'real-rel';\n");
    p.file(
        "src/auto.js",
        "export function a() { return 'real-auto'; }\n",
    );
    p.file(
        "src/use.js",
        "import { r } from './rel.js';\nexport const useR = () => r();\n",
    );
    p.file(
        "src/a.test.js",
        r#"import { r } from './rel.js';
import { useR } from './use.js';
import { p } from 'pkg';
import { a } from './auto.js';
vi.mock('./rel.js', () => ({ r: () => 'mock-rel' }));
jest.mock('pkg', () => ({ p: () => 'mock-pkg' }));
vi.mock('./auto.js');
// a glob, and a query id whose file does not exist, are skipped without disturbing the others
vi.mock('./icons/*.svg', () => ({ default: 'svg' }));
vi.mock('./styles.css?inline', () => ({ default: '' }));
test('entry mocks intercept', () => {
  expect(r()).toBe('mock-rel');
  expect(useR()).toBe('mock-rel');
  expect(p()).toBe('mock-pkg');
  expect(vi.isMockFunction(a)).toBe(true);
});
"#,
    );
    for env in [&[("TURBO_NO_MR", "1")][..], &[][..]] {
        let r = turbo_test(&p.dir, &["src/a.test.js"], env);
        assert_passes(&r, 1);
    }
}

#[test]
fn a_bare_vi_hoisted_after_a_declared_one_runs_once_in_the_prepass() {
    // turbo-test#31 item 7: the bare call's statement start was walked back to the previous
    // `let`, so the mock prepass declared `b` twice (SyntaxError) and its mocks never
    // registered. Under TURBO_NO_MR (legacy bundle path) the prepass is the only registration
    // of an entry mock, so the real package leaked through.
    let p = Project::new("rc-hoisted");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"r7"}"#);
    p.file(
        "node_modules/react/package.json",
        r#"{"name":"react","main":"index.js"}"#,
    );
    p.file(
        "node_modules/react/index.js",
        "exports.createElement = () => null;\n",
    );
    p.file(
        "node_modules/dep/package.json",
        r#"{"name":"dep","main":"index.js"}"#,
    );
    p.file("node_modules/dep/index.js", "exports.dep = () => 'real';\n");
    p.file(
        "a.test.js",
        r#"import { dep } from 'dep';
let b = vi.hoisted(() => 2)
vi.hoisted(() => { globalThis.H3 = 3; });
vi.mock('dep', () => ({ dep: () => b + globalThis.H3 }));
test('hoisted values reach the factory', () => {
  expect(b).toBe(2);
  expect(dep()).toBe(5);
});
"#,
    );
    for env in [&[][..], &[("TURBO_NO_MR", "1")][..]] {
        let r = turbo_test(&p.dir, &["a.test.js"], env);
        assert_passes(&r, 1);
    }
}

#[test]
fn remocking_a_dependency_keeps_a_mocked_importer() {
    // turbo-test#31 item 2 repro: on the reuse path the y re-mock used to evict x's mocked
    // exports, so the next require('./x.cjs') read `{}`; the fresh retry failed the same way.
    let p = Project::new("rc-remock-reuse");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"r2"}"#);
    p.file("x.cjs", "module.exports = { v: require('./y.cjs').v };\n");
    p.file("y.cjs", "module.exports = { v: 'y' };\n");
    p.file(
        "a.test.js",
        r#"test('re-mock dependency keeps mocked importer', () => {
  expect(require('./x.cjs').v).toBe('y');
  vi.doMock('./x.cjs', () => ({ v: 'mx' }));
  expect(require('./x.cjs').v).toBe('mx');
  vi.doMock('./y.cjs', () => ({ v: 'my' }));
  expect(require('./x.cjs')).toEqual({ v: 'mx' });
  expect(require('./y.cjs')).toEqual({ v: 'my' });
});
"#,
    );
    // a second file in the same reused isolate: x's real importer chain re-mocks normally
    p.file(
        "b.test.js",
        r#"test('a real importer sees a re-mocked dependency', () => {
  vi.doMock('./y.cjs', () => ({ v: 'my2' }));
  expect(require('./x.cjs').v).toBe('my2');
});
"#,
    );
    for env in [
        &[("TURBO_REUSE_ISOLATE", "1"), ("TURBO_JOBS", "1")][..],
        &[("TURBO_NO_REUSE", "1")][..],
    ] {
        let r = turbo_test(&p.dir, &["a.test.js", "b.test.js"], env);
        assert_passes(&r, 2);
    }
}

#[test]
fn a_fresh_isolate_under_the_reuse_switch_does_not_patch_mocks_into_real_exports() {
    // turbo-test#31 item 2, second cause: register_mock_value checked the global reuse switch,
    // not whether this file runs on the reuse path. `--coverage` runs every file on a fresh
    // isolate even with TURBO_REUSE_ISOLATE set (as does the fresh-isolate retry); there a
    // doMock must register a new module (vitest), not copy the mock onto the real exports
    // object a test already holds.
    let p = Project::new("rc-remock-fresh");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"r2b"}"#);
    p.file("x.cjs", "module.exports = { v: 'x' };\n");
    p.file(
        "a.test.js",
        r#"test('doMock leaves a held real module alone', () => {
  const realX = require('./x.cjs');
  vi.doMock('./x.cjs', () => ({ v: 'mx' }));
  expect(require('./x.cjs').v).toBe('mx');
  expect(realX.v).toBe('x');
});
"#,
    );
    for env in [&[("TURBO_REUSE_ISOLATE", "1")][..], &[][..]] {
        let r = turbo_test(&p.dir, &["--coverage", "a.test.js"], env);
        assert_passes(&r, 1);
    }
}

#[test]
fn dynamic_import_of_a_lazy_stub_barrel_settles() {
    // turbo-test#31 item 3 repro (ESM-graph path, no esbuild): the stub Proxy answered `then`,
    // so the import() promise treated the namespace as a thenable and never settled.
    let p = Project::new("rc-lazy-import");
    p.file("package.json", r#"{"name":"r3","type":"module"}"#);
    p.file(
        "node_modules/react/package.json",
        r#"{"name":"react","main":"index.js"}"#,
    );
    p.file(
        "node_modules/react/index.js",
        "exports.createElement = (t, p) => ({ type: t, props: p });\nexports.forwardRef = (render) => render;\n",
    );
    p.file(
        "a.test.js",
        r#"test('dynamic import of a lazy-stub barrel settles', async () => {
  const m = await import('@mui/icons-material');
  expect(typeof m.Add).toBe('function');
  expect(m.then).toBe(undefined);
  const again = await import('@mui/icons-material');
  expect(again.Add).toBe(m.Add);
});
"#,
    );
    let r = turbo_test(
        &p.dir,
        &["--testTimeout", "2000", "a.test.js"],
        &[("TURBO_LAZY_STUBS", "@mui/icons-material")],
    );
    assert_passes(&r, 1);
}
