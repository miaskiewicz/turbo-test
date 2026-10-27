//! The runner's legacy ESM-graph loader (projects without esbuild, so the module-runner CJS path
//! is not available): static imports of JSON/CSS/ESM/CJS/builtins/`vitest`, dynamic `import()`,
//! entry `vi.mock` factories registered without a prepass, per-file load errors, and the
//! `__tt.run()` drive loop's contract checks.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

/// The text-reporter line for `file` (`PASS  file  (...)`, `FAIL ...`, `ERROR file  (msg)`).
fn line_for<'a>(stdout: &'a str, file: &str) -> &'a str {
    stdout
        .lines()
        .find(|l| l.contains(&format!("/{file}  ")) || l.contains(&format!(" {file}  ")))
        .unwrap_or_else(|| panic!("no result line for {file} in:\n{stdout}"))
}

#[test]
fn static_and_dynamic_imports_of_every_module_kind() {
    let p = Project::new("rb-esm-kinds");
    p.file("package.json", r#"{"name":"rb-esm-kinds"}"#);
    p.file("dep.cjs", "exports.a = 1; exports.f = () => 'real';\n");
    p.file("data.json", r#"{"k": 5, "name": "x"}"#);
    p.file("s.module.css", ".c{}\n");
    p.file("esm.mjs", "export const E = 'esm'; export default 7;\n");
    p.file("bad.mjs", "export const = ;\n");
    p.file(
        "a.test.js",
        r#"import { describe, it, expect, vi } from 'vitest';
import { jest } from '@jest/globals';
import d from './data.json';
import s from './s.module.css';
import def, { E } from './esm.mjs';
import fs from 'node:fs';
import path from 'path';
import dep from './dep.cjs';
describe('g', () => {
  it('json', () => { expect(d.k).toBe(5); expect(d.name).toBe('x'); });
  it('css module proxy', () => { expect(s.foo).toBe('foo'); });
  it('esm', () => { expect(def).toBe(7); expect(E).toBe('esm'); });
  it('cjs', () => { expect(dep.a).toBe(1); expect(dep.f()).toBe('real'); });
  it('jest globals', () => { expect(typeof jest.fn).toBe('function'); expect(typeof vi.fn).toBe('function'); });
  it('import.meta.url', () => { expect(import.meta.url).toBe('file://' + globalThis.__cwd + '/a.test.js'); });
  it('dynamic', async () => {
    const m = await import('./esm.mjs'); expect(m.E).toBe('esm');
    const p = await import('node:path'); expect(typeof p.join).toBe('function');
    const v = await import('vitest'); expect(typeof v.vi.fn).toBe('function');
    await expect(import('./missing.mjs')).rejects.toThrow("Cannot find module '/missing.mjs' imported from ");
    // vite's parse error, as vitest's import() (turbo-test#108)
    await expect(import('./bad.mjs')).rejects.toThrow(expect.objectContaining({ name: 'RolldownError', message: expect.stringContaining('Parse failure: Parse failed with 1 error:\nUnexpected token\n') }));
  });
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        line_for(&r.stdout, "a.test.js").contains("(7 passed, 0 failed)"),
        "{}",
        r.all()
    );
}

/// turbo-test#61: an import() that does not resolve rejects as vitest 4.1.12's does: the
/// specifier as vite resolved it (a relative one URL-resolved against the importer's
/// root-relative path), the importer, and Node's code.
#[test]
fn unresolved_dynamic_imports_reject_like_vitest() {
    let p = Project::new("rb-esm-missing");
    p.file("package.json", r#"{"name":"rb-esm-missing"}"#);
    p.file(
        "sub/e.test.js",
        r#"const from = ` imported from ${globalThis.__cwd}/sub/e.test.js`;
const rejects = async (spec, message, code) => {
  const e = await import(spec).then(() => null, (e) => e);
  expect([spec, e && e.constructor.name, e && e.message, e && e.code]).toEqual([spec, 'Error', message, code]);
};
test('unresolved', async () => {
  const mod = 'ERR_MODULE_NOT_FOUND';
  await rejects('./nope.js', `Cannot find module '/sub/nope.js'${from}`, mod);
  await rejects('./nope', `Cannot find module '/sub/nope'${from}`, mod);
  await rejects('../nope.js', `Cannot find module '/nope.js'${from}`, mod);
  await rejects('../../nope.js', `Cannot find module '/nope.js'${from}`, mod);
  await rejects('/abs/nope.js', `Cannot find module '/abs/nope.js'${from}`, mod);
  await rejects('nopkg', `Cannot find package 'nopkg'${from}`, mod);
  await rejects('nopkg/sub', `Cannot find package 'nopkg/sub'${from}`, mod);
  await rejects('fs/nope', `Cannot find package 'fs/nope'${from}`, mod);
  await rejects('node:nope', 'No such built-in module: node:nope', 'ERR_UNKNOWN_BUILTIN_MODULE');
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        line_for(&r.stdout, "sub/e.test.js").contains("(1 passed, 0 failed)"),
        "{}",
        r.all()
    );
}

#[test]
fn node_fs_shim_is_backed_by_native_fs() {
    let p = Project::new("rb-esm-fs");
    p.file("package.json", r#"{"name":"rb-esm-fs"}"#);
    p.file("seed/one.txt", "1");
    p.file("seed/two.txt", "2");
    p.file(
        "fs.test.js",
        r#"import fs from 'node:fs';
import path from 'path';
test('fs natives', () => {
  const root = globalThis.__cwd;
  const dir = path.join(root, 'out');
  fs.mkdirSync(dir + '/sub/deeper', { recursive: true });
  expect(fs.existsSync(dir + '/sub/deeper')).toBe(true);
  fs.writeFileSync(dir + '/sub/f.txt', 'hello');
  expect(() => fs.writeFileSync(dir + '/auto/created/g.txt', 'no parents')).toThrow(`ENOENT: no such file or directory, open '${dir}/auto/created/g.txt'`);
  expect(fs.existsSync(dir + '/sub/f.txt')).toBe(true);
  expect(fs.readFileSync(dir + '/sub/f.txt', 'utf8')).toBe('hello');
  expect(fs.existsSync(dir + '/auto')).toBe(false);
  expect(fs.readdirSync(root + '/seed').sort()).toEqual(['one.txt', 'two.txt']);
  expect(() => fs.readdirSync(dir + '/nope')).toThrow(`ENOENT: no such file or directory, scandir '${dir}/nope'`);
  expect(() => fs.readFileSync(dir + '/nope.txt')).toThrow(`ENOENT: no such file or directory, open '${dir}/nope.txt'`);
  expect(() => fs.writeFileSync(dir + '/sub/f.txt/x', 'a')).toThrow(`ENOTDIR: not a directory, open '${dir}/sub/f.txt/x'`);
  fs.rmSync(dir + '/sub/f.txt');
  expect(fs.existsSync(dir + '/sub/f.txt')).toBe(false);
  fs.rmSync(dir + '/missing-entry', { force: true });
  fs.rmSync(dir, { recursive: true });
  expect(fs.existsSync(dir)).toBe(false);
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(!p.path("out").exists(), "rmSync(dir) removed the tree");
    assert!(p.path("seed/one.txt").exists());
}

#[test]
fn entry_mocks_register_without_an_esbuild_prepass() {
    let p = Project::new("rb-esm-mocks");
    p.file("package.json", r#"{"name":"rb-esm-mocks"}"#);
    for n in ["a", "b", "d"] {
        p.file(&format!("{n}.cjs"), &format!("exports.v = '{n}-real';\n"));
    }
    p.file("c.cjs", "exports.fn = () => 'c-real';\n");
    p.file("e.mjs", "export default 'e-real';\nexport const n = 0;\n");
    p.file(
        "m.test.js",
        r#"import a from './a.cjs';
import e, { n } from './e.mjs';
import b from './b.cjs';
import c from './c.cjs';
vi.mock('./a.cjs', () => ({ v: 'a-mock' }));
jest.mock('./b.cjs', () => ({ v: 'b-mock' }));
// a factory's `default` key is the mock's default export
vi.mock('./e.mjs', () => ({ default: 'e-mock', n: 1 }));
// a throwing factory does not break the registration of the others (and, its module never
// being imported, does not fail the file: vitest only calls a factory on import)
vi.mock('./d.cjs', () => { throw new Error('factory boom'); });
vi.mock('./not-there.cjs', () => ({ v: 1 }));
test('mocks', () => {
  // a vi.mock factory without `default`: reading the default import throws (vitest 4.1.12,
  // turbo-test#137); jest.mock's factory is its own default
  expect(() => a.v).toThrow('[vitest] No "default" export is defined on the "./a.cjs" mock.');
  expect(b.v).toBe('b-mock');
  expect(e).toBe('e-mock');
  expect(n).toBe(1);
  // no factory: an automock (the real implementation is not reachable)
  expect(vi.isMockFunction(c.fn)).toBe(true);
  expect(c.fn()).toBe(undefined);
});
// factory-less form (code may follow it since turbo-test#31 item 4)
vi.mock('./c.cjs');
"#,
    );
    for env in [&[][..], &[("TURBO_NO_MR", "1")][..]] {
        let r = turbo_test(&p.dir, &[], env);
        assert_eq!(r.code, Some(0), "{env:?}\n{}", r.all());
    }
}

#[test]
fn factory_less_entry_mock_is_an_automock() {
    // vitest's automocking algorithm: functions (and default exports) become vi.fn() mocks that
    // return undefined, classes get mocked statics + prototype methods, objects and class
    // instances are mocked deeply, primitives are kept and arrays emptied. Same result without
    // esbuild (ESM graph) and with it (module runner).
    let p = Project::new("rb-esm-automock");
    p.file(
        "package.json",
        r#"{"name":"rb-esm-automock","type":"module"}"#,
    );
    p.file(
        "m.js",
        r#"export function f() { return 1; }
export default function d() { return 8; }
class Base { base() { return 'b'; } }
export class C extends Base { constructor() { super(); this.x = 1; } m() { return 2; } static s() { return 3; } }
class Svc { call() { return 'real'; } }
export const svc = new Svc();
export const obj = { g: () => 4, nested: { h: () => 5, n: 6 } };
export const num = 7;
export const str = 's';
export const arr = [1, 2];
"#,
    );
    p.file(
        "a.test.js",
        r#"import d, { f, C, svc, obj, num, str, arr } from './m.js';
test('automock', () => {
  expect(vi.isMockFunction(f)).toBe(true);
  expect(f()).toBe(undefined);
  f.mockReturnValue(9);
  expect(f()).toBe(9);
  expect(vi.isMockFunction(d)).toBe(true);
  expect(d()).toBe(undefined);
  expect(vi.isMockFunction(C)).toBe(true);
  expect(vi.isMockFunction(C.s)).toBe(true);
  const c = new C();
  expect(c.x).toBe(undefined);
  expect(vi.isMockFunction(c.m)).toBe(true);
  expect(c.m()).toBe(undefined);
  expect(vi.isMockFunction(c.base)).toBe(true);
  expect(vi.isMockFunction(svc.call)).toBe(true);
  expect(svc.call()).toBe(undefined);
  expect(vi.isMockFunction(obj.g)).toBe(true);
  expect(vi.isMockFunction(obj.nested.h)).toBe(true);
  expect(obj.nested.n).toBe(6);
  expect(num).toBe(7);
  expect(str).toBe('s');
  expect(arr).toEqual([]);
});
// factory-less form (code may follow it since turbo-test#31 item 4)
vi.mock('./m.js');
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "no esbuild\n{}", r.all());
    p.with_esbuild();
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "module runner\n{}", r.all());
}

#[test]
fn load_errors_name_the_failing_phase() {
    let p = Project::new("rb-esm-errors");
    p.file("package.json", r#"{"name":"rb-esm-errors"}"#);
    p.file("lib.mjs", "export const x = 1;\n");
    p.file("syn.mjs", "export const = ;\n");
    let t = "test('t', () => {});";
    p.file(
        "inst.test.js",
        &format!("import {{ nope }} from './lib.mjs'; {t}\n"),
    );
    p.file("depsyn.test.js", &format!("import './syn.mjs'; {t}\n"));
    p.file("miss.test.js", &format!("import './missing.mjs'; {t}\n"));
    p.file(
        "evalthrow.test.js",
        &format!("{t} throw new Error('top boom');\n"),
    );
    p.file(
        "tla.test.js",
        &format!("{t} await Promise.reject(new Error('tla boom'));\n"),
    );
    p.file("entrysyn.test.js", &format!("{t} )\n"));
    p.file("ok.test.js", &format!("{t}\n"));
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    let o = &r.stdout;
    assert!(
        line_for(o, "inst.test.js").contains(
            "inst.test.js  (SyntaxError: The requested module './lib.mjs' does not provide an export named 'nope')"
        ),
        "{o}"
    );
    // a module of the graph that does not parse: vite's parse error (turbo-test#99, #108)
    assert!(line_for(o, "depsyn.test.js")
        .ends_with("(RolldownError: Parse failure: Parse failed with 1 error:"));
    assert!(line_for(o, "miss.test.js").ends_with(&format!(
        "(Error: Cannot find module './missing.mjs' imported from {}/miss.test.js)",
        std::fs::canonicalize(&p.dir).unwrap().display()
    )));
    assert!(line_for(o, "entrysyn.test.js")
        .ends_with("(Error: Failed to parse source for import analysis because the content contains invalid JS syntax. If you are using JSX, make sure to name the file with the .jsx or .tsx extension.)"));
    assert!(!o.contains("Uncaught"), "{o}");
    assert!(line_for(o, "evalthrow.test.js").contains("(Error: top boom)"));
    assert!(line_for(o, "tla.test.js").contains("(Error: tla boom)"));
    assert!(line_for(o, "ok.test.js").starts_with("PASS"));
    assert!(!r.stderr.contains("cannot resolve"), "{}", r.all());
}

#[test]
fn drive_loop_rejects_a_broken_runtime_contract() {
    let p = Project::new("rb-esm-drive");
    p.file("package.json", r#"{"name":"rb-esm-drive"}"#);
    let cases = [
        (
            "nott",
            "Object.defineProperty(globalThis, '__tt', { get() { throw new Error('g'); } });",
            "(no __tt)",
        ),
        ("ttnum", "globalThis.__tt = 5;", "(__tt not object)"),
        ("runnum", "globalThis.__tt.run = 1;", "(run not fn)"),
        (
            "runget",
            "Object.defineProperty(globalThis.__tt, 'run', { get() { throw 1; } });",
            "(no run)",
        ),
        (
            "runval",
            "globalThis.__tt.run = () => 1;",
            "(run() not promise)",
        ),
        (
            "runthrow",
            "globalThis.__tt.run = () => { throw new Error('x'); };",
            "(run() threw)",
        ),
        (
            "runrej",
            "globalThis.__tt.run = () => Promise.reject(new Error('rj'));",
            "(test run promise did not fulfill (rejected: Error: rj))",
        ),
        (
            "runpend",
            "globalThis.__tt.run = () => new Promise(() => {});",
            "(test run promise did not fulfill (still pending))",
        ),
        (
            "runsum",
            "globalThis.__tt.run = async () => 5;",
            "(summary not object)",
        ),
    ];
    for (name, body, _) in &cases {
        p.file(
            &format!("{name}.test.js"),
            &format!("test('t', () => {{}}); {body}\n"),
        );
    }
    // A hand-built summary: counts, failure strings and per-test rows (null message -> empty,
    // non-object rows skipped) are read back field by field.
    p.file(
        "custom.test.js",
        "test('t', () => {}); globalThis.__tt.run = async () => ({ passed: 2, failed: 1, \
         failures: ['a: b'], tests: [{ name: 'n', status: 'passed', duration_ms: 1500, \
         message: null }, 7] });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "verbose"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    for (name, _, msg) in &cases {
        let l = line_for(&r.stdout, &format!("{name}.test.js"));
        assert!(
            l.starts_with("ERROR") && l.ends_with(msg),
            "{name}: {l}\n{}",
            r.all()
        );
    }
    let l = line_for(&r.stdout, "custom.test.js");
    assert!(
        l.starts_with("FAIL") && l.ends_with("custom.test.js  (2 passed, 1 failed)"),
        "{l}"
    );
    assert!(r.stdout.contains("✗ a: b"), "{}", r.all());
    assert!(r.stdout.contains("✓ n (1500ms)"), "{}", r.all());
}

#[test]
fn synthetic_modules_for_shared_assets_defaults_and_builtins() {
    let p = Project::new("rb-esm-synth");
    p.file("package.json", r#"{"name":"rb-esm-synth"}"#);
    p.file("shared.json", r#"{"n": 1}"#);
    p.file("shared.css", ".a{}\n");
    p.file(
        "withdefault.cjs",
        "exports.default = 'dflt'; exports.named = 'n';\n",
    );
    p.file(
        "a.mjs",
        "import j from './shared.json'; import c from './shared.css'; export const fromA = j.n + ':' + c.k;\n",
    );
    p.file(
        "s.test.js",
        r#"import j from './shared.json';
import c from './shared.css';
import { fromA } from './a.mjs';
import def, { named } from './withdefault.cjs';
import vm from 'node:vm';
test('synthetic modules', () => {
  expect(j.n).toBe(1);
  expect(c.k).toBe('k');
  expect(fromA).toBe('1:k');
  // an app CommonJS file's own `default` is its default (vitest 4.1.12, turbo-test#122)
  expect(def).toBe('dflt');
  expect(named).toBe('n');
  expect(vm.runInThisContext('1 + 1')).toBe(2);
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn a_dependency_evicted_during_graph_load_is_reloaded_at_link_time() {
    // y.cjs runs vi.resetModules() while the entry's graph is loading, dropping the already
    // loaded x.cjs; linking the entry then loads x.cjs again (it runs twice).
    let p = Project::new("rb-esm-evict");
    p.file("package.json", r#"{"name":"rb-esm-evict"}"#);
    p.file(
        "x.cjs",
        "globalThis.__xRuns = (globalThis.__xRuns || 0) + 1; exports.n = globalThis.__xRuns;\n",
    );
    p.file("y.cjs", "vi.resetModules(); exports.y = 1;\n");
    p.file(
        "e.test.js",
        "import { n } from './x.cjs';\nimport { y } from './y.cjs';\n\
         test('reloaded', () => { expect(y).toBe(1); expect(n).toBe(2); });\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());

    // Same on an esbuild project: an entry esbuild cannot emit as CJS (top-level await) takes
    // the graph path, the evicted dep is reloaded through the module-runner CJS loader, and a
    // dynamic import goes through it too.
    let q = Project::new("rb-esm-evict-mr");
    q.with_esbuild();
    q.file("package.json", r#"{"name":"rb-esm-evict-mr"}"#);
    q.file(
        "x.cjs",
        "globalThis.__xRuns = (globalThis.__xRuns || 0) + 1; exports.n = globalThis.__xRuns;\n",
    );
    q.file("y.cjs", "vi.resetModules(); exports.y = 1;\n");
    q.file("lib.ts", "export const L: string = 'lib';\n");
    q.file(
        "e.test.js",
        "import { n } from './x.cjs';\nimport { y } from './y.cjs';\nconst lib = await import('./lib.ts');\n\
         test('reloaded', () => { expect(y).toBe(1); expect(n).toBe(2); expect(lib.L).toBe('lib'); });\n",
    );
    let r = turbo_test(&q.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn dynamic_import_of_a_lazy_stub_barrel_without_react_loads_the_real_package() {
    // The stub factory needs `react`; without it the configured barrel loads for real.
    let p = Project::new("rb-esm-lazy");
    p.file("package.json", r#"{"name":"rb-esm-lazy"}"#);
    p.file(
        "node_modules/fake-icons/package.json",
        r#"{"name":"fake-icons","main":"index.mjs"}"#,
    );
    p.file(
        "node_modules/fake-icons/index.mjs",
        "export const Real = 'real-export';\n",
    );
    p.file(
        "l.test.js",
        "test('lazy', async () => { expect((await import('fake-icons')).Real).toBe('real-export'); });\n",
    );
    let r = turbo_test(&p.dir, &[], &[("TURBO_LAZY_STUBS", "fake-icons")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

/// turbo-test#82: `import()` of a module with top-level await settles after the module has
/// evaluated (Node 26 / vitest 4.1.12), not with a namespace whose bindings are uninitialised.
#[test]
fn dynamic_import_waits_for_top_level_await() {
    let p = Project::new("rb-esm-tla");
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "tla.ts",
        "await new Promise((r) => setTimeout(r, 20));\nexport const x = 1;\n",
    );
    p.file(
        "a.test.ts",
        "import { expect, it } from 'vitest';\n\
         it('import() of a module with top-level await waits for it', async () => {\n\
         \x20 const m = await import('./tla');\n\
         \x20 expect(m.x).toBe(1);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        line_for(&r.stdout, "a.test.ts").starts_with("PASS"),
        "{}",
        r.all()
    );
}
